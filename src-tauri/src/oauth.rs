//! Signing in to a cloud store: OAuth 2 with PKCE in the system browser, the
//! answer coming back to this app on a loopback address. The refresh token is
//! kept in the Credential Manager, the access token only in memory.

use crate::credentials;
use crate::dbfile::hex;
use crate::remote::RemoteError;
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use url::Url;
use zeroize::Zeroizing;

/// Registered with the stores, so the port is fixed.
const PORT: u16 = 53134;
const SIGN_IN_WAIT: Duration = Duration::from_secs(300);
const TIMEOUT: Duration = Duration::from_secs(60);

/// Set to give up waiting for the browser.
static CANCEL: AtomicBool = AtomicBool::new(false);

/// An access token and when it runs out.
pub type Access = Mutex<Option<(Zeroizing<String>, Instant)>>;

/// One client for every request to the stores, so connections are reused.
pub fn agent() -> &'static ureq::Agent {
    static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| crate::icons::http_agent(TIMEOUT, false));
    &AGENT
}

/// Too many requests, or trouble at the store: worth trying again later.
pub fn is_temporary(status: u16) -> bool {
    status == 429 || status >= 500
}

/// A store's sign-in.
pub struct Provider {
    /// For messages: "Dropbox", "Google Drive".
    pub name: &'static str,
    pub authorize_url: &'static str,
    pub token_url: &'static str,
    /// The path the browser comes back to on the loopback address.
    pub path: &'static str,
    /// As registered with the store (`localhost` or `127.0.0.1`).
    pub redirect_host: &'static str,
    pub client_id: &'static str,
    /// Stores that want one for installed apps; not a secret there.
    pub client_secret: Option<&'static str>,
    /// What else the authorisation asks for (scope, offline access).
    pub extra: &'static [(&'static str, &'static str)],
    /// The refresh token's name in the Credential Manager.
    pub credential: &'static str,
    pub access: &'static Access,
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

impl Provider {
    pub fn sign_in_again(&self) -> String {
        format!("Sign in to {} again", self.name)
    }

    fn redirect_uri(&self) -> String {
        format!("http://{}:{PORT}{}", self.redirect_host, self.path)
    }

    /// A request that did not get an answer: offline, or the store unreachable.
    pub fn transport(&self, e: ureq::Error) -> RemoteError {
        RemoteError::Offline(format!("Cannot reach {}: {e}", self.name))
    }

    /// Signs in: `open` shows the store's page in the browser.
    pub fn sign_in(&self, open: impl FnOnce(&str) -> Result<(), String>) -> Result<(), String> {
        let client_id = self.client_id()?;
        let verifier = random_text(32);
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let state = random_text(16);
        let listener = Loopback::bind(self)?;
        CANCEL.store(false, Ordering::Relaxed);
        let redirect = self.redirect_uri();
        let mut url = Url::parse(self.authorize_url).map_err(|e| e.to_string())?;
        url.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("response_type", "code")
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("redirect_uri", &redirect)
            .append_pair("state", &state)
            .extend_pairs(self.extra);
        open(url.as_str())?;
        let code = listener.wait_for_code(self, &state)?;

        let form = [
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("redirect_uri", redirect.as_str()),
        ];
        let mut tokens = self.request_tokens(&form).map_err(|e| e.message())?;
        let refresh = Zeroizing::new(tokens.refresh_token.take().ok_or_else(|| format!("{} gave no lasting sign-in", self.name))?);
        credentials::write(self.credential, &refresh)?;
        self.remember(tokens);
        Ok(())
    }

    pub fn sign_out(&self) {
        credentials::delete(self.credential);
        self.forget_access();
    }

    /// Drops the access token, so the next request gets a new one.
    pub fn forget_access(&self) {
        *self.access.lock().unwrap() = None;
    }

    fn client_id(&self) -> Result<&'static str, String> {
        if self.client_id.is_empty() {
            return Err(format!("{} is not available in this build", self.name));
        }
        Ok(self.client_id)
    }

    fn request_tokens(&self, form: &[(&str, &str)]) -> Result<Tokens, RemoteError> {
        let client_id = self.client_id().map_err(RemoteError::Failed)?;
        let mut form = form.to_vec();
        form.push(("client_id", client_id));
        if let Some(secret) = self.client_secret {
            form.push(("client_secret", secret));
        }
        let mut response = agent().post(self.token_url).send_form(form).map_err(|e| self.transport(e))?;
        let status = response.status().as_u16();
        let body = Zeroizing::new(response.body_mut().read_to_string().map_err(|e| self.transport(e))?);
        let error = || serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|v| v["error"].as_str().map(str::to_string));
        match status {
            200 => serde_json::from_str(&body).map_err(|e| RemoteError::Failed(format!("{} answered oddly: {e}", self.name))),
            // The code or the refresh token is no longer valid (access was
            // withdrawn, or a test app's week ran out).
            400 | 401 if error().is_none_or(|e| e == "invalid_grant") => Err(RemoteError::SignIn(self.sign_in_again())),
            400 | 401 => Err(RemoteError::Failed(format!("{} refused the sign-in ({})", self.name, error().unwrap_or_default()))),
            _ if is_temporary(status) => Err(RemoteError::Offline(format!("{} is busy ({status}); trying again later", self.name))),
            _ => Err(RemoteError::Failed(format!("{} sign-in failed ({status})", self.name))),
        }
    }

    /// Keeps the access token in memory, and returns it.
    fn remember(&self, tokens: Tokens) -> Zeroizing<String> {
        // A minute early, so a token never runs out mid-request.
        let until = Instant::now() + Duration::from_secs(tokens.expires_in.saturating_sub(60));
        let token = Zeroizing::new(tokens.access_token);
        *self.access.lock().unwrap() = Some((token.clone(), until));
        token
    }

    /// A valid access token: the one in memory, or a new one from the refresh token.
    pub fn access_token(&self) -> Result<Zeroizing<String>, RemoteError> {
        if let Some((token, until)) = &*self.access.lock().unwrap() {
            if Instant::now() < *until {
                return Ok(token.clone());
            }
        }
        let refresh = credentials::read(self.credential).ok_or_else(|| RemoteError::SignIn(self.sign_in_again()))?;
        let mut tokens = self.request_tokens(&[("grant_type", "refresh_token"), ("refresh_token", refresh.as_str())])?;
        // Some stores (Microsoft) hand out a new refresh token each time: keep
        // it, unless the account was signed out meanwhile. Failing to store it
        // costs nothing now: the old one still works for a while.
        if let Some(rotated) = tokens.refresh_token.take().map(Zeroizing::new) {
            if *rotated != *refresh && credentials::read(self.credential).is_some() {
                let _ = credentials::write(self.credential, &rotated);
            }
        }
        Ok(self.remember(tokens))
    }
}

/// Stops waiting for the browser (the user gave up on signing in).
pub fn cancel_sign_in() {
    CANCEL.store(true, Ordering::Relaxed);
}

pub fn random_text(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).expect("the OS random number generator");
    hex(&buffer)
}

/// The loopback address the browser comes back to: IPv4 and, where there is
/// one, IPv6, since browsers may resolve `localhost` to either.
struct Loopback(Vec<TcpListener>);

impl Loopback {
    fn bind(provider: &Provider) -> Result<Loopback, String> {
        let v4 = TcpListener::bind(("127.0.0.1", PORT))
            .map_err(|e| format!("Cannot wait for {} on port {PORT} (is another sign-in open?): {e}", provider.name))?;
        let listeners: Vec<TcpListener> = [Some(v4), TcpListener::bind(("::1", PORT)).ok()].into_iter().flatten().collect();
        for listener in &listeners {
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        }
        Ok(Loopback(listeners))
    }

    /// Waits for the browser to arrive with the sign-in's code.
    fn wait_for_code(&self, provider: &Provider, state: &str) -> Result<Zeroizing<String>, String> {
        let until = Instant::now() + SIGN_IN_WAIT;
        while Instant::now() < until {
            if CANCEL.load(Ordering::Relaxed) {
                return Err(format!("The {} sign-in was cancelled", provider.name));
            }
            for listener in &self.0 {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Some(answer) = answer(stream, provider, state) {
                            return answer;
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(format!("The {} sign-in was not finished in time", provider.name))
    }
}

/// Reads one browser request. `None` for anything but the sign-in's answer
/// (a favicon request, say).
fn answer(mut stream: TcpStream, provider: &Provider, state: &str) -> Option<Result<Zeroizing<String>, String>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).ok()?;
    let target = line.split_whitespace().nth(1)?;
    let url = Url::parse(&format!("http://localhost{target}")).ok()?;
    if url.path() != provider.path {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return None;
    }
    let param = |name: &str| url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| Zeroizing::new(v.into_owned()));
    let name = provider.name;
    let result = if param("state").as_deref().map(String::as_str) != Some(state) {
        Err(format!("The {name} sign-in answer did not match; try again"))
    } else if let Some(code) = param("code") {
        Ok(code)
    } else {
        Err(format!("The {name} sign-in was cancelled"))
    };
    let page = match &result {
        Ok(_) => format!("PswManager is signed in to {name}. You can close this tab."),
        Err(message) => message.clone(),
    };
    let body = format!("<!doctype html><meta charset=utf-8><title>PswManager</title><p style=\"font:16px sans-serif\">{page}</p>");
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_ACCESS: Access = Mutex::new(None);
    const TEST: Provider = Provider {
        name: "Test",
        authorize_url: "https://example.com/authorize",
        token_url: "https://example.com/token",
        path: "/test",
        redirect_host: "localhost",
        client_id: "id",
        client_secret: None,
        extra: &[],
        credential: "test",
        access: &TEST_ACCESS,
    };

    #[test]
    fn the_browser_answer_gives_the_code_only_with_the_right_state() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let ask = |path: &'static str| {
            std::thread::spawn(move || {
                let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
                write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
                let mut reply = String::new();
                let _ = std::io::Read::read_to_string(&mut s, &mut reply);
                reply
            })
        };
        let client = ask("/test?code=abc&state=s1");
        let (stream, _) = listener.accept().unwrap();
        assert_eq!(answer(stream, &TEST, "s1").unwrap().unwrap().as_str(), "abc");
        assert!(client.join().unwrap().contains("signed in to Test"));

        let client = ask("/test?code=abc&state=forged");
        let (stream, _) = listener.accept().unwrap();
        assert!(answer(stream, &TEST, "s1").unwrap().is_err());
        client.join().unwrap();

        let client = ask("/favicon.ico");
        let (stream, _) = listener.accept().unwrap();
        assert!(answer(stream, &TEST, "s1").is_none());
        client.join().unwrap();
    }
}
