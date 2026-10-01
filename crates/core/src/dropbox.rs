//! Dropbox: a file in the app's folder (`Apps/PswManager Sync`) as a remote
//! store. Signing in is [crate::oauth].

use crate::oauth::{self, Access, Provider};
use crate::remote::{Remote, RemoteError};
use serde_json::{json, Value};
use std::sync::Mutex;

const FILE_LIMIT: u64 = 100 * 1024 * 1024;
const API: &str = "https://api.dropboxapi.com/2";
const CONTENT: &str = "https://content.dropboxapi.com/2";

static ACCESS: Access = Mutex::new(None);

/// The app at Dropbox: a public client (PKCE needs no secret), its redirect
/// registered as `http://localhost:53134/dropbox`.
pub const DROPBOX: Provider = Provider {
    name: "Dropbox",
    authorize_url: "https://www.dropbox.com/oauth2/authorize",
    token_url: "https://api.dropboxapi.com/oauth2/token",
    path: "/dropbox",
    redirect_host: "localhost",
    client_id: "let5y7fm9kxwrdi",
    client_secret: None,
    extra: &[("token_access_type", "offline")],
    credential: "dropbox",
    access: &ACCESS,
};

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
        if oauth::is_temporary(self.status) {
            RemoteError::Offline(message)
        } else {
            RemoteError::Failed(message)
        }
    }
}

/// Calls the API; a token Dropbox no longer takes is renewed once.
fn call(url: &str, arg: Option<&Value>, body: Option<(&str, &[u8])>) -> Result<Answer, RemoteError> {
    let answer = send(url, arg, body)?;
    if answer.status != 401 {
        return Ok(answer);
    }
    DROPBOX.forget_access();
    match send(url, arg, body)? {
        again if again.status == 401 => Err(RemoteError::SignIn(DROPBOX.sign_in_again())),
        again => Ok(again),
    }
}

fn send(url: &str, arg: Option<&Value>, body: Option<(&str, &[u8])>) -> Result<Answer, RemoteError> {
    let token = DROPBOX.access_token()?;
    let mut request = oauth::agent().post(url).header("Authorization", &format!("Bearer {}", token.as_str()));
    if let Some(arg) = arg {
        request = request.header("Dropbox-API-Arg", &header_json(arg));
    }
    let sent = match body {
        Some((content_type, bytes)) => request.header("Content-Type", content_type).send(bytes),
        None => request.send_empty(),
    };
    let mut response = sent.map_err(|e| DROPBOX.transport(e))?;
    let status = response.status().as_u16();
    let result = response.headers().get("dropbox-api-result").and_then(|v| v.to_str().ok()).map(str::to_string);
    let body = response.body_mut().with_config().limit(FILE_LIMIT).read_to_vec().map_err(|e| DROPBOX.transport(e))?;
    Ok(Answer { status, result, body })
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
}
