//! Google Drive: a file in the `PswManager` folder at the top of the Drive as
//! a remote store. The app asks only for `drive.file`: it sees the files it
//! created itself, nothing else in the Drive. Signing in is [crate::oauth].

use crate::oauth::{self, Access, Provider};
use crate::remote::{Remote, RemoteError};
use serde_json::{json, Value};
use std::sync::Mutex;
use url::Url;

/// The desktop client (Windows): it signs in through a loopback address.
#[cfg(not(target_os = "android"))]
const CLIENT_ID: &str = "488783310128-a29rg9cu85a3ki2kdibuvvruapgc7ua3.apps.googleusercontent.com";
/// Google wants one for installed apps though it is no secret there; it is
/// given at build time (`PSWM_GOOGLE_CLIENT_SECRET`) rather than kept in the
/// source. Without it the build offers no Google Drive.
#[cfg(not(target_os = "android"))]
const CLIENT_SECRET: Option<&str> = match option_env!("PSWM_GOOGLE_CLIENT_SECRET") {
    // CI gives an empty value when the repository has no such secret.
    Some(secret) if !secret.is_empty() => Some(secret),
    _ => None,
};
/// The Android client (the package and the release key's SHA-1, with its
/// custom URI scheme on): it comes back to `io.github.olegg90.pswmanager:/oauth2redirect`
/// and has no secret.
#[cfg(target_os = "android")]
const CLIENT_ID: &str = "488783310128-cbvgi85j606j5903d8sheqvhqtahefn1.apps.googleusercontent.com";
#[cfg(target_os = "android")]
const CLIENT_SECRET: Option<&str> = None;
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
    // Without its secret the desktop client cannot sign in; the Android one needs none.
    client_id: match (CLIENT_SECRET, cfg!(target_os = "android")) {
        (Some(_), _) | (None, true) => CLIENT_ID,
        (None, false) => "",
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
        let error = self.json().map(|v| v["error"].clone()).unwrap_or_default();
        let reason = error["message"].as_str().unwrap_or_default();
        let message = format!("Google Drive could not {what} ({} {reason})", self.status);
        // Drive answers a rate limit with 403 `rateLimitExceeded` / `userRateLimitExceeded`.
        let rate_limited = error["errors"][0]["reason"].as_str().is_some_and(|r| r.ends_with("ateLimitExceeded"));
        if oauth::is_temporary(self.status) || rate_limited {
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
    let with_body = |request: ureq::RequestBuilder<ureq::typestate::WithBody>| {
        let request = request.header("Authorization", &bearer);
        match body {
            Some((content_type, bytes)) => request.header("Content-Type", content_type).send(bytes),
            None => request.send_empty(),
        }
    };
    let sent = match method {
        Method::Get => agent.get(url).header("Authorization", &bearer).call(),
        Method::Post => with_body(agent.post(url)),
        Method::Patch => with_body(agent.patch(url)),
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

/// Files matching a Drive search, as `(id, name)`, every page of them.
fn find(query: &str) -> Result<Vec<(String, String)>, RemoteError> {
    let mut found = Vec::new();
    let mut page_token = String::new();
    loop {
        let mut params = vec![("q", query), ("fields", "nextPageToken,files(id,name)"), ("spaces", "drive")];
        if !page_token.is_empty() {
            params.push(("pageToken", &page_token));
        }
        let answer = call(Method::Get, &url(FILES, &params), None)?;
        if answer.status != 200 {
            return Err(answer.failed("search the Drive"));
        }
        let page = answer.json()?;
        let files = page["files"].as_array().into_iter().flatten();
        found.extend(files.filter_map(|f| Some((f["id"].as_str()?.to_string(), f["name"].as_str()?.to_string()))));
        match page["nextPageToken"].as_str() {
            Some(next) => page_token = next.to_string(),
            None => return Ok(found),
        }
    }
}

/// The app's folder, created at the top of the Drive the first time. The
/// user may move it anywhere (into their own Apps folder, say): with
/// `drive.file` the app sees only what it created, so it is found wherever it is.
fn folder() -> Result<String, RemoteError> {
    let query = format!("name = {} and mimeType = '{FOLDER_TYPE}' and trashed = false", quoted(FOLDER));
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

/// Uploads a new file into the app's folder and returns its id and revision;
/// a file of that name already there is never replaced ([RemoteError::Changed]).
pub fn create(name: &str, bytes: &[u8]) -> Result<(String, String), RemoteError> {
    let folder = folder()?;
    if !find(&format!("name = {} and {} in parents and trashed = false", quoted(name), quoted(&folder)))?.is_empty() {
        return Err(RemoteError::Changed);
    }
    let boundary = format!("pswm-{}", oauth::random_text(8));
    let meta = json!({ "name": name, "parents": [folder] });
    let mut body = format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let content_type = format!("multipart/related; boundary={boundary}");
    let answer = call(Method::Post, &url(UPLOAD, &[("uploadType", "multipart"), ("fields", "id,headRevisionId")]), Some((&content_type, &body)))?;
    if answer.status != 200 {
        return Err(answer.failed("upload the file"));
    }
    let meta = answer.json()?;
    let field = |name: &str| meta[name].as_str().map(str::to_string).ok_or_else(|| RemoteError::Failed(format!("Google Drive sent no {name}")));
    Ok((field("id")?, field("headRevisionId")?))
}

impl GoogleDrive {
    fn file_url(&self, query: &[(&str, &str)]) -> String {
        url(&format!("{FILES}/{}", self.id), query)
    }
}

impl Remote for GoogleDrive {
    /// Drive's `headRevisionId`: it changes with the content only (`version`
    /// also moves with metadata and sharing changes).
    fn revision(&self) -> Result<Option<String>, RemoteError> {
        let answer = call(Method::Get, &self.file_url(&[("fields", "headRevisionId,trashed")]), None)?;
        match answer.status {
            200 => {
                let meta = answer.json()?;
                if meta["trashed"] == true {
                    return Ok(None);
                }
                Ok(meta["headRevisionId"].as_str().map(str::to_string))
            }
            404 => Ok(None),
            _ => Err(answer.failed("look at the file")),
        }
    }

    /// The revision is read before and after, so the bytes are known to be
    /// that revision (the download itself does not say).
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

    /// Drive has no conditional upload: the revision is checked right before,
    /// which leaves a moment in which another device's upload is not seen.
    fn upload(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, RemoteError> {
        let Some(expected) = expected else {
            return Err(RemoteError::Failed("The file is gone from Google Drive: stop syncing and set it up again".into()));
        };
        if self.revision()?.as_deref() != Some(expected) {
            return Err(RemoteError::Changed);
        }
        let upload = url(&format!("{UPLOAD}/{}", self.id), &[("uploadType", "media"), ("fields", "headRevisionId")]);
        let answer = call(Method::Patch, &upload, Some(("application/octet-stream", bytes)))?;
        if answer.status != 200 {
            return Err(answer.failed("upload the file"));
        }
        answer.json()?["headRevisionId"].as_str().map(str::to_string).ok_or_else(|| RemoteError::Failed("Google Drive sent no revision".into()))
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
