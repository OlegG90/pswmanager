//! Dropbox: signing in (OAuth 2 with PKCE in the system browser) and a file in
//! the app's folder (`Apps/PswManager Sync`) as a remote store.

use crate::credentials;
use crate::dbfile::hex;
use crate::remote::{Remote, RemoteError};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use url::Url;
use zeroize::Zeroizing;

/// The app's public id at Dropbox; PKCE needs no secret.
const APP_KEY: &str = "let5y7fm9kxwrdi";
/// Registered with the app at Dropbox, so the port is fixed.
const PORT: u16 = 53134;
const REDIRECT_URI: &str = "http://localhost:53134/dropbox";
/// The refresh token's name in the Credential Manager.
const CREDENTIAL: &str = "dropbox";
const SIGN_IN_WAIT: Duration = Duration::from_secs(300);
const TIMEOUT: Duration = Duration::from_secs(60);
const FILE_LIMIT: u64 = 100 * 1024 * 1024;

const API: &str = "https://api.dropboxapi.com/2";
const CONTENT: &str = "https://content.dropboxapi.com/2";
const TOKEN_URL: &str = "https://api.dropboxapi.com/oauth2/token";

pub const SIGN_IN_AGAIN: &str = "Sign in to Dropbox again";

/// The current access token and when it runs out; it lives only in memory.
static ACCESS: Mutex<Option<(Zeroizing<String>, Instant)>> = Mutex::new(None);
/// Set to give up waiting for the browser.
static CANCEL: AtomicBool = AtomicBool::new(false);

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build())
        .build()
        .into()
}

/// A request that did not get an answer: offline, or Dropbox unreachable.
fn transport(e: ureq::Error) -> RemoteError {
    RemoteError::Offline(format!("Cannot reach Dropbox: {e}"))
}

/// Too many requests, or trouble at Dropbox: worth trying again later.
fn is_temporary(status: u16) -> bool {
    status == 429 || status >= 500
}

// ------------------------------------------------------------ signing in

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

/// Signs in: `open` shows Dropbox's page in the browser, and the answer comes
/// back to this app on the loopback address. The refresh token is kept in the
/// Credential Manager.
pub fn sign_in(open: impl FnOnce(&str) -> Result<(), String>) -> Result<(), String> {
    let verifier = random_text(32);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_text(16);
    let listener = Loopback::bind()?;
    CANCEL.store(false, Ordering::Relaxed);
    let mut url = Url::parse("https://www.dropbox.com/oauth2/authorize").expect("a valid URL");
    url.query_pairs_mut()
        .append_pair("client_id", APP_KEY)
        .append_pair("response_type", "code")
        .append_pair("token_access_type", "offline")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("state", &state);
    open(url.as_str())?;
    let code = listener.wait_for_code(&state)?;

    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("client_id", APP_KEY),
        ("code_verifier", verifier.as_str()),
        ("redirect_uri", REDIRECT_URI),
    ];
    let tokens = request_tokens(&form).map_err(|e| e.message())?;
    let refresh = Zeroizing::new(tokens.refresh_token.clone().ok_or("Dropbox gave no lasting sign-in")?);
    credentials::write(CREDENTIAL, &refresh)?;
    remember(tokens);
    Ok(())
}

/// Stops waiting for the browser (the user gave up on signing in).
pub fn cancel_sign_in() {
    CANCEL.store(true, Ordering::Relaxed);
}

pub fn sign_out() {
    credentials::delete(CREDENTIAL);
    *ACCESS.lock().unwrap() = None;
}

fn request_tokens(form: &[(&str, &str)]) -> Result<Tokens, RemoteError> {
    let mut response = agent().post(TOKEN_URL).send_form(form.iter().copied()).map_err(transport)?;
    let status = response.status().as_u16();
    let body = Zeroizing::new(response.body_mut().read_to_string().map_err(transport)?);
    match status {
        200 => serde_json::from_str(&body).map_err(|e| RemoteError::Failed(format!("Dropbox answered oddly: {e}"))),
        // The code or the refresh token is no longer valid (the app was
        // removed from the account, say).
        400 | 401 => Err(RemoteError::SignIn(SIGN_IN_AGAIN.into())),
        _ if is_temporary(status) => Err(RemoteError::Offline(format!("Dropbox is busy ({status}); trying again later"))),
        _ => Err(RemoteError::Failed(format!("Dropbox sign-in failed ({status})"))),
    }
}

fn remember(tokens: Tokens) {
    // A minute early, so a token never runs out mid-request.
    let until = Instant::now() + Duration::from_secs(tokens.expires_in.saturating_sub(60));
    *ACCESS.lock().unwrap() = Some((Zeroizing::new(tokens.access_token), until));
}

/// A valid access token: the one in memory, or a new one from the refresh token.
fn access_token() -> Result<Zeroizing<String>, RemoteError> {
    if let Some((token, until)) = &*ACCESS.lock().unwrap() {
        if Instant::now() < *until {
            return Ok(token.clone());
        }
    }
    let refresh = credentials::read(CREDENTIAL).ok_or_else(|| RemoteError::SignIn(SIGN_IN_AGAIN.into()))?;
    let tokens = request_tokens(&[("grant_type", "refresh_token"), ("refresh_token", refresh.as_str()), ("client_id", APP_KEY)])?;
    let token = Zeroizing::new(tokens.access_token.clone());
    remember(tokens);
    Ok(token)
}

fn random_text(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).expect("the OS random number generator");
    hex(&buffer)
}

/// The loopback address the browser comes back to: IPv4 and, where there is
/// one, IPv6, since browsers may resolve `localhost` to either.
struct Loopback(Vec<TcpListener>);

impl Loopback {
    fn bind() -> Result<Loopback, String> {
        let v4 = TcpListener::bind(("127.0.0.1", PORT))
            .map_err(|e| format!("Cannot wait for Dropbox on port {PORT} (is another sign-in open?): {e}"))?;
        let listeners: Vec<TcpListener> = [Some(v4), TcpListener::bind(("::1", PORT)).ok()].into_iter().flatten().collect();
        for listener in &listeners {
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        }
        Ok(Loopback(listeners))
    }

    /// Waits for the browser to arrive with the sign-in's code.
    fn wait_for_code(&self, state: &str) -> Result<Zeroizing<String>, String> {
        let until = Instant::now() + SIGN_IN_WAIT;
        while Instant::now() < until {
            if CANCEL.load(Ordering::Relaxed) {
                return Err("The Dropbox sign-in was cancelled".into());
            }
            for listener in &self.0 {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Some(answer) = answer(stream, state) {
                            return answer;
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err("The Dropbox sign-in was not finished in time".into())
    }
}

/// Reads one browser request. `None` for anything but the sign-in's answer
/// (a favicon request, say).
fn answer(mut stream: TcpStream, state: &str) -> Option<Result<Zeroizing<String>, String>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).ok()?;
    let target = line.split_whitespace().nth(1)?;
    let url = Url::parse(&format!("http://localhost{target}")).ok()?;
    if url.path() != "/dropbox" {
        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return None;
    }
    let param = |name: &str| url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| Zeroizing::new(v.into_owned()));
    let result = if param("state").as_deref().map(String::as_str) != Some(state) {
        Err("The Dropbox sign-in answer did not match; try again".to_string())
    } else if let Some(code) = param("code") {
        Ok(code)
    } else {
        Err("The Dropbox sign-in was cancelled".to_string())
    };
    let page = match &result {
        Ok(_) => "PswManager is signed in to Dropbox. You can close this tab.",
        Err(message) => message.as_str(),
    };
    let body = format!("<!doctype html><meta charset=utf-8><title>PswManager</title><p style=\"font:16px sans-serif\">{page}</p>");
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    Some(result)
}

// ------------------------------------------------------------ the store

/// A file in the app folder; `path` is from the folder's top, e.g. `/base.kdbx`.
pub struct Dropbox {
    pub path: String,
}

/// An answer from the API: its status, the `Dropbox-API-Result` header (for
/// downloads) and the body.
struct Answer {
    status: u16,
    result: Option<String>,
    body: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Result<Value, RemoteError> {
        serde_json::from_slice(&self.body).map_err(|e| RemoteError::Failed(format!("Dropbox answered oddly: {e}")))
    }

    /// Dropbox's short error code, e.g. `path/not_found/..`.
    fn error(&self) -> String {
        self.json().ok().and_then(|v| v["error_summary"].as_str().map(str::to_string)).unwrap_or_default()
    }

    fn failed(&self, what: &str) -> RemoteError {
        let message = format!("Dropbox could not {what} ({} {})", self.status, self.error());
        if is_temporary(self.status) {
            RemoteError::Offline(message)
        } else {
            RemoteError::Failed(message)
        }
    }
}

/// Calls the API with a fresh token; an expired one is renewed once.
fn call(url: &str, arg: Option<&Value>, body: Option<(&str, &[u8])>) -> Result<Answer, RemoteError> {
    for attempt in 0..2 {
        let token = access_token()?;
        let mut request = agent().post(url).header("Authorization", &format!("Bearer {}", token.as_str()));
        if let Some(arg) = arg {
            request = request.header("Dropbox-API-Arg", &header_json(arg));
        }
        let sent = match body {
            Some((content_type, bytes)) => request.header("Content-Type", content_type).send(bytes),
            None => request.send_empty(),
        };
        let mut response = sent.map_err(transport)?;
        let status = response.status().as_u16();
        if status == 401 && attempt == 0 {
            *ACCESS.lock().unwrap() = None;
            continue;
        }
        if status == 401 {
            return Err(RemoteError::SignIn(SIGN_IN_AGAIN.into()));
        }
        let result = response.headers().get("dropbox-api-result").and_then(|v| v.to_str().ok()).map(str::to_string);
        let body = response.body_mut().with_config().limit(FILE_LIMIT).read_to_vec().map_err(transport)?;
        return Ok(Answer { status, result, body });
    }
    unreachable!("the loop returns")
}

/// JSON for an HTTP header: Dropbox wants DEL and anything beyond ASCII escaped.
fn header_json(value: &Value) -> String {
    value
        .to_string()
        .encode_utf16()
        .map(|u| if u < 0x7f { char::from(u as u8).to_string() } else { format!("\\u{u:04x}") })
        .collect()
}

fn rpc(endpoint: &str, arg: Value) -> Result<Answer, RemoteError> {
    call(&format!("{API}/{endpoint}"), None, Some(("application/json", arg.to_string().as_bytes())))
}

/// The `.kdbx` files in the app folder, as paths.
pub fn list_databases() -> Result<Vec<String>, RemoteError> {
    let mut paths = Vec::new();
    let mut answer = rpc("files/list_folder", json!({ "path": "" }))?;
    loop {
        if answer.status != 200 {
            return Err(answer.failed("list the app folder"));
        }
        let page = answer.json()?;
        let files = page["entries"].as_array().into_iter().flatten().filter(|e| e[".tag"] == "file");
        let names = files.filter_map(|e| e["path_display"].as_str()).filter(|p| p.to_lowercase().ends_with(".kdbx"));
        paths.extend(names.map(str::to_string));
        match page["cursor"].as_str() {
            Some(cursor) if page["has_more"] == true => answer = rpc("files/list_folder/continue", json!({ "cursor": cursor }))?,
            _ => break,
        }
    }
    paths.sort_by_key(|p| p.to_lowercase());
    Ok(paths)
}

impl Remote for Dropbox {
    fn revision(&self) -> Result<Option<String>, RemoteError> {
        let answer = rpc("files/get_metadata", json!({ "path": self.path }))?;
        match answer.status {
            200 => {
                let meta = answer.json()?;
                if meta[".tag"] != "file" {
                    return Err(RemoteError::Failed(format!("{} in Dropbox is not a file", self.path)));
                }
                Ok(meta["rev"].as_str().map(str::to_string))
            }
            409 if answer.error().starts_with("path/not_found") => Ok(None),
            _ => Err(answer.failed("look at the file")),
        }
    }

    fn download(&self) -> Result<(Vec<u8>, String), RemoteError> {
        let answer = call(&format!("{CONTENT}/files/download"), Some(&json!({ "path": self.path })), None)?;
        if answer.status != 200 {
            return Err(answer.failed("download the file"));
        }
        let meta: Value = answer
            .result
            .as_deref()
            .and_then(|r| serde_json::from_str(r).ok())
            .ok_or_else(|| RemoteError::Failed("Dropbox sent the file without its revision".into()))?;
        let rev = meta["rev"].as_str().ok_or_else(|| RemoteError::Failed("Dropbox sent no revision".into()))?;
        Ok((answer.body, rev.to_string()))
    }

    /// Dropbox checks the revision itself (`update` mode, strict conflicts),
    /// so no other device's upload is ever overwritten.
    fn upload(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, RemoteError> {
        let mode = match expected {
            Some(rev) => json!({ ".tag": "update", "update": rev }),
            None => json!("add"),
        };
        let arg = json!({ "path": self.path, "mode": mode, "autorename": false, "mute": true, "strict_conflict": true });
        let answer = call(&format!("{CONTENT}/files/upload"), Some(&arg), Some(("application/octet-stream", bytes)))?;
        match answer.status {
            200 => answer.json()?["rev"].as_str().map(str::to_string).ok_or_else(|| RemoteError::Failed("Dropbox sent no revision".into())),
            409 if answer.error().contains("conflict") => Err(RemoteError::Changed),
            _ => Err(answer.failed("upload the file")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_json_escapes_beyond_ascii() {
        let arg = json!({ "path": "/Паролі é.kdbx" });
        let header = header_json(&arg);
        assert!(header.is_ascii(), "{header}");
        assert_eq!(serde_json::from_str::<Value>(&header).unwrap(), arg);
    }

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
        let client = ask("/dropbox?code=abc&state=s1");
        let (stream, _) = listener.accept().unwrap();
        assert_eq!(answer(stream, "s1").unwrap().unwrap().as_str(), "abc");
        assert!(client.join().unwrap().contains("signed in to Dropbox"));

        let client = ask("/dropbox?code=abc&state=forged");
        let (stream, _) = listener.accept().unwrap();
        assert!(answer(stream, "s1").unwrap().is_err());
        client.join().unwrap();

        let client = ask("/favicon.ico");
        let (stream, _) = listener.accept().unwrap();
        assert!(answer(stream, "s1").is_none());
        client.join().unwrap();
    }
}
