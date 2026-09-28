//! Google Drive: a file in the `PswManager` folder at the top of the Drive as
//! a remote store. The app asks only for `drive.file`: it sees the files it
//! created itself, nothing else in the Drive. Signing in is [crate::oauth].

use crate::oauth::{self, Access, Provider};
use crate::remote::{Remote, RemoteError};
use serde_json::{json, Value};
use std::sync::Mutex;
use url::Url;

const CLIENT_ID: &str = "488783310128-a29rg9cu85a3ki2kdibuvvruapgc7ua3.apps.googleusercontent.com";
/// Google wants one for installed apps though it is no secret there; it is
/// given at build time (`PSWM_GOOGLE_CLIENT_SECRET`) rather than kept in the
/// source. Without it the build offers no Google Drive.
const CLIENT_SECRET: Option<&str> = option_env!("PSWM_GOOGLE_CLIENT_SECRET");
const FILE_LIMIT: u64 = 100 * 1024 * 1024;
const FILES: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
pub const FOLDER: &str = "PswManager";
const FOLDER_TYPE: &str = "application/vnd.google-apps.folder";

static ACCESS: Access = Mutex::new(None);

pub const GOOGLE: Provider = Provider {
    name: "Google Drive",
    authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
    token_url: "https://oauth2.googleapis.com/token",
    path: "/google",
    redirect_host: "127.0.0.1",
    client_id: match CLIENT_SECRET {
        Some(_) => CLIENT_ID,
        None => "",
    },
    client_secret: CLIENT_SECRET,
    // `prompt=consent`: Google gives a refresh token only when asked again.
    extra: &[("scope", "https://www.googleapis.com/auth/drive.file"), ("access_type", "offline"), ("prompt", "consent")],
    credential: "google",
    access: &ACCESS,
};

/// A file in the Drive, by its id.
pub struct GoogleDrive {
    pub id: String,
}

struct Answer {
    status: u16,
    body: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Result<Value, RemoteError> {
        serde_json::from_slice(&self.body).map_err(|e| RemoteError::Failed(format!("Google Drive answered oddly: {e}")))
    }

    fn failed(&self, what: &str) -> RemoteError {
        let reason = self.json().ok().and_then(|v| v["error"]["message"].as_str().map(str::to_string)).unwrap_or_default();
        let message = format!("Google Drive could not {what} ({} {reason})", self.status);
        if oauth::is_temporary(self.status) {
            RemoteError::Offline(message)
        } else {
            RemoteError::Failed(message)
        }
    }
}

#[derive(Clone, Copy)]
enum Method {
    Get,
    Post,
    Patch,
}

/// Calls the API; a token Google no longer takes is renewed once.
fn call(method: Method, url: &str, body: Option<(&str, &[u8])>) -> Result<Answer, RemoteError> {
    let answer = send(method, url, body)?;
    if answer.status != 401 {
        return Ok(answer);
    }
    GOOGLE.forget_access();
    match send(method, url, body)? {
        again if again.status == 401 => Err(RemoteError::SignIn(GOOGLE.sign_in_again())),
        again => Ok(again),
    }
}

fn send(method: Method, url: &str, body: Option<(&str, &[u8])>) -> Result<Answer, RemoteError> {
    let token = GOOGLE.access_token()?;
    let bearer = format!("Bearer {}", token.as_str());
    let agent = oauth::agent();
    let sent = match method {
        Method::Get => agent.get(url).header("Authorization", &bearer).call(),
        Method::Post | Method::Patch => {
            let request = match method {
                Method::Post => agent.post(url),
                _ => agent.patch(url),
            };
            let request = request.header("Authorization", &bearer);
            match body {
                Some((content_type, bytes)) => request.header("Content-Type", content_type).send(bytes),
                None => request.send_empty(),
            }
        }
    };
    let mut response = sent.map_err(|e| GOOGLE.transport(e))?;
    let status = response.status().as_u16();
    let body = response.body_mut().with_config().limit(FILE_LIMIT).read_to_vec().map_err(|e| GOOGLE.transport(e))?;
    Ok(Answer { status, body })
}

fn url(base: &str, query: &[(&str, &str)]) -> String {
    let mut url = Url::parse(base).expect("a valid URL");
    url.query_pairs_mut().extend_pairs(query);
    url.into()
}

/// A Drive search's quoting: `\` and `'` are escaped.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Files matching a Drive search, as `(id, name)`.
fn find(query: &str) -> Result<Vec<(String, String)>, RemoteError> {
    let answer = call(Method::Get, &url(FILES, &[("q", query), ("fields", "files(id,name)"), ("spaces", "drive")]), None)?;
    if answer.status != 200 {
        return Err(answer.failed("search the Drive"));
    }
    let files = answer.json()?["files"].as_array().cloned().unwrap_or_default();
    Ok(files
        .iter()
        .filter_map(|f| Some((f["id"].as_str()?.to_string(), f["name"].as_str()?.to_string())))
        .collect())
}

/// The app's folder, created the first time.
fn folder() -> Result<String, RemoteError> {
    let query = format!("name = {} and mimeType = '{FOLDER_TYPE}' and 'root' in parents and trashed = false", quoted(FOLDER));
    if let Some((id, _)) = find(&query)?.into_iter().next() {
        return Ok(id);
    }
    let meta = json!({ "name": FOLDER, "mimeType": FOLDER_TYPE });
    let answer = call(Method::Post, &url(FILES, &[("fields", "id")]), Some(("application/json", meta.to_string().as_bytes())))?;
    if answer.status != 200 {
        return Err(answer.failed("create the PswManager folder"));
    }
    answer.json()?["id"].as_str().map(str::to_string).ok_or_else(|| RemoteError::Failed("Google Drive sent no folder id".into()))
}

/// The `.kdbx` files in the app's folder, as `(id, name)`.
pub fn list_databases() -> Result<Vec<(String, String)>, RemoteError> {
    let mut files = find(&format!("{} in parents and trashed = false", quoted(&folder()?)))?;
    files.retain(|(_, name)| name.to_lowercase().ends_with(".kdbx"));
    files.sort_by_key(|(_, name)| name.to_lowercase());
    Ok(files)
}

/// Uploads a new file into the app's folder and returns its id; a file of
/// that name already there is never replaced ([RemoteError::Changed]).
pub fn create(name: &str, bytes: &[u8]) -> Result<String, RemoteError> {
    if list_databases()?.iter().any(|(_, existing)| existing.eq_ignore_ascii_case(name)) {
        return Err(RemoteError::Changed);
    }
    let boundary = format!("pswm-{:016x}", u64::from_le_bytes(random_bytes()));
    let meta = json!({ "name": name, "parents": [folder()?] });
    let mut body = format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let content_type = format!("multipart/related; boundary={boundary}");
    let answer = call(Method::Post, &url(UPLOAD, &[("uploadType", "multipart"), ("fields", "id")]), Some((&content_type, &body)))?;
    if answer.status != 200 {
        return Err(answer.failed("upload the file"));
    }
    answer.json()?["id"].as_str().map(str::to_string).ok_or_else(|| RemoteError::Failed("Google Drive sent no file id".into()))
}

fn random_bytes() -> [u8; 8] {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the OS random number generator");
    bytes
}

impl GoogleDrive {
    fn file_url(&self, query: &[(&str, &str)]) -> String {
        url(&format!("{FILES}/{}", self.id), query)
    }
}

impl Remote for GoogleDrive {
    /// Drive's `version`, which every change of the file raises.
    fn revision(&self) -> Result<Option<String>, RemoteError> {
        let answer = call(Method::Get, &self.file_url(&[("fields", "version,trashed")]), None)?;
        match answer.status {
            200 => {
                let meta = answer.json()?;
                if meta["trashed"] == true {
                    return Ok(None);
                }
                Ok(meta["version"].as_str().map(str::to_string))
            }
            404 => Ok(None),
            _ => Err(answer.failed("look at the file")),
        }
    }

    /// The version is read before and after, so the bytes are known to be
    /// that version (the download itself does not say).
    fn download(&self) -> Result<(Vec<u8>, String), RemoteError> {
        for _ in 0..3 {
            let before = self.revision()?.ok_or_else(|| RemoteError::Failed("The file is gone from Google Drive".into()))?;
            let answer = call(Method::Get, &self.file_url(&[("alt", "media")]), None)?;
            if answer.status != 200 {
                return Err(answer.failed("download the file"));
            }
            if self.revision()?.as_ref() == Some(&before) {
                return Ok((answer.body, before));
            }
        }
        Err(RemoteError::Failed("The file in Google Drive keeps changing; try again in a moment".into()))
    }

    /// Drive has no conditional upload: the version is checked right before,
    /// which leaves a moment in which another device's upload is not seen.
    fn upload(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, RemoteError> {
        let Some(expected) = expected else {
            return Err(RemoteError::Failed("The file is gone from Google Drive: stop syncing and set it up again".into()));
        };
        if self.revision()?.as_deref() != Some(expected) {
            return Err(RemoteError::Changed);
        }
        let upload = url(&format!("{UPLOAD}/{}", self.id), &[("uploadType", "media"), ("fields", "version")]);
        let answer = call(Method::Patch, &upload, Some(("application/octet-stream", bytes)))?;
        if answer.status != 200 {
            return Err(answer.failed("upload the file"));
        }
        answer.json()?["version"].as_str().map(str::to_string).ok_or_else(|| RemoteError::Failed("Google Drive sent no version".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_text_is_quoted() {
        assert_eq!(quoted(r"it's a\b"), r"'it\'s a\\b'");
    }
}
