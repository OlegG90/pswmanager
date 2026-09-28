//! OneDrive: a file in the app's own folder (`Apps/PswManager`, named after
//! the app's registration) as a remote store, through Microsoft Graph.
//! Signing in is [crate::oauth].

use crate::oauth::{self, Access, Provider};
use crate::remote::{Remote, RemoteError};
use serde_json::Value;
use std::sync::Mutex;
use url::Url;

const FILE_LIMIT: u64 = 100 * 1024 * 1024;
const GRAPH: &str = "https://graph.microsoft.com/v1.0/me/drive";
/// What the app folder is shown as; OneDrive names it after the app.
pub const FOLDER: &str = "Apps/PswManager";

static ACCESS: Access = Mutex::new(None);

/// The app at Microsoft: a public client for personal accounts (PKCE, no
/// secret), its redirect registered as `http://localhost:53134/onedrive`, and
/// only its own folder (`Files.ReadWrite.AppFolder`).
pub const ONEDRIVE: Provider = Provider {
    name: "OneDrive",
    authorize_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize",
    token_url: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
    path: "/onedrive",
    redirect_host: "localhost",
    client_id: "f6413fbc-5e7d-4004-a564-e2104be62597",
    client_secret: None,
    extra: &[("scope", "Files.ReadWrite.AppFolder offline_access")],
    credential: "onedrive",
    access: &ACCESS,
};

/// A file in the app folder, by its item id.
pub struct OneDrive {
    pub id: String,
}

/// An answer from Graph: its status and body.
struct Answer {
    status: u16,
    body: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Result<Value, RemoteError> {
        serde_json::from_slice(&self.body).map_err(|e| RemoteError::Failed(format!("OneDrive answered oddly: {e}")))
    }

    /// Graph's error code, e.g. `itemNotFound`.
    fn error(&self) -> String {
        self.json().ok().and_then(|v| v["error"]["code"].as_str().map(str::to_string)).unwrap_or_default()
    }

    fn failed(&self, what: &str) -> RemoteError {
        let message = format!("OneDrive could not {what} ({} {})", self.status, self.error());
        if oauth::is_temporary(self.status) {
            RemoteError::Offline(message)
        } else {
            RemoteError::Failed(message)
        }
    }

    /// The item's revision (its eTag, which uploads are checked against).
    fn etag(&self) -> Result<String, RemoteError> {
        self.json()?["eTag"].as_str().map(str::to_string).ok_or_else(|| RemoteError::Failed("OneDrive sent no revision".into()))
    }
}

/// Calls Graph; a token it no longer takes is renewed once.
fn call(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<Answer, RemoteError> {
    let answer = send(method, url, headers, body)?;
    if answer.status != 401 {
        return Ok(answer);
    }
    ONEDRIVE.forget_access();
    match send(method, url, headers, body)? {
        again if again.status == 401 => Err(RemoteError::SignIn(ONEDRIVE.sign_in_again())),
        again => Ok(again),
    }
}

fn send(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Result<Answer, RemoteError> {
    let token = ONEDRIVE.access_token()?;
    let bearer = format!("Bearer {}", token.as_str());
    let sent = match method {
        "PUT" => {
            let mut request = oauth::agent().put(url).header("Authorization", &bearer).header("Content-Type", "application/octet-stream");
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            request.send(body.unwrap_or_default())
        }
        _ => {
            let mut request = oauth::agent().get(url).header("Authorization", &bearer);
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            request.call()
        }
    };
    read(sent)
}

fn read(sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Answer, RemoteError> {
    let mut response = sent.map_err(|e| ONEDRIVE.transport(e))?;
    let status = response.status().as_u16();
    let body = response.body_mut().with_config().limit(FILE_LIMIT).read_to_vec().map_err(|e| ONEDRIVE.transport(e))?;
    Ok(Answer { status, body })
}

fn item_url(id: &str) -> String {
    format!("{GRAPH}/items/{}", encode(id))
}

/// A path segment or id as a URL wants it.
fn encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect::<String>().replace('+', "%20")
}

/// The `.kdbx` files in the app folder, as (id, name).
pub fn list_databases() -> Result<Vec<(String, String)>, RemoteError> {
    let mut files = Vec::new();
    let mut next = Some(format!("{GRAPH}/special/approot/children?$select=id,name,file&$top=200"));
    while let Some(url) = next {
        let answer = call("GET", &url, &[], None)?;
        if answer.status != 200 {
            return Err(answer.failed("list the app folder"));
        }
        let page = answer.json()?;
        let items = page["value"].as_array().into_iter().flatten().filter(|item| item["file"].is_object());
        files.extend(items.filter_map(|item| Some((item["id"].as_str()?.to_string(), item["name"].as_str()?.to_string()))).filter(|(_, name)| name.to_lowercase().ends_with(".kdbx")));
        // Only Graph's own next page is followed.
        next = page["@odata.nextLink"].as_str().filter(|link| link.starts_with(GRAPH)).map(str::to_string);
    }
    files.sort_by_key(|(_, name)| name.to_lowercase());
    Ok(files)
}

/// Uploads a new file into the app folder and returns its id and revision;
/// a file of that name already there is never replaced ([RemoteError::Changed]).
pub fn create(name: &str, bytes: &[u8]) -> Result<(String, String), RemoteError> {
    let url = format!("{GRAPH}/special/approot:/{}:/content?@microsoft.graph.conflictBehavior=fail", encode(name));
    let answer = call("PUT", &url, &[], Some(bytes))?;
    match answer.status {
        200 | 201 => {
            let id = answer.json()?["id"].as_str().map(str::to_string).ok_or_else(|| RemoteError::Failed("OneDrive sent no file id".into()))?;
            Ok((id, answer.etag()?))
        }
        409 => Err(RemoteError::Changed),
        _ => Err(answer.failed("upload the file")),
    }
}

impl Remote for OneDrive {
    fn revision(&self) -> Result<Option<String>, RemoteError> {
        let answer = call("GET", &format!("{}?$select=id,eTag,file,deleted", item_url(&self.id)), &[], None)?;
        match answer.status {
            200 => {
                let item = answer.json()?;
                if item["deleted"].is_object() {
                    return Ok(None);
                }
                if !item["file"].is_object() {
                    return Err(RemoteError::Failed("The OneDrive item is not a file".into()));
                }
                Ok(item["eTag"].as_str().map(str::to_string))
            }
            404 => Ok(None),
            _ => Err(answer.failed("look at the file")),
        }
    }

    /// The item first, for its revision and a short-lived download address
    /// that needs no token (the token never goes to another host); then the
    /// content from there. Should the file change in between, the revision is
    /// older than the content and the next upload is refused, which starts the
    /// sync over.
    fn download(&self) -> Result<(Vec<u8>, String), RemoteError> {
        let answer = call("GET", &item_url(&self.id), &[], None)?;
        if answer.status != 200 {
            return Err(answer.failed("look at the file"));
        }
        let item = answer.json()?;
        let etag = answer.etag()?;
        let address = item["@microsoft.graph.downloadUrl"].as_str().ok_or_else(|| RemoteError::Failed("OneDrive sent no download address".into()))?;
        let address = Url::parse(address).map_err(|_| RemoteError::Failed("OneDrive sent an odd download address".into()))?;
        if address.scheme() != "https" {
            return Err(RemoteError::Failed("OneDrive sent a download address that is not https".into()));
        }
        let content = read(oauth::agent().get(address.as_str()).call())?;
        if content.status != 200 {
            return Err(content.failed("download the file"));
        }
        Ok((content.body, etag))
    }

    /// Graph checks the revision itself (`If-Match`), so no other device's
    /// upload is ever overwritten.
    fn upload(&self, bytes: &[u8], expected: Option<&str>) -> Result<String, RemoteError> {
        let Some(expected) = expected else {
            // The file is gone (deleted in OneDrive): there is nothing to replace by id.
            return Err(RemoteError::Failed("The file is no longer in OneDrive; set up sync again".into()));
        };
        let answer = call("PUT", &format!("{}/content", item_url(&self.id)), &[("If-Match", expected)], Some(bytes))?;
        match answer.status {
            200 | 201 => answer.etag(),
            412 => Err(RemoteError::Changed),
            404 => Err(RemoteError::Failed("The file is no longer in OneDrive; set up sync again".into())),
            _ => Err(answer.failed("upload the file")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_ids_are_encoded_for_the_path() {
        assert_eq!(encode("Паролі & co.kdbx"), "%D0%9F%D0%B0%D1%80%D0%BE%D0%BB%D1%96%20%26%20co.kdbx");
        assert_eq!(encode("ABC!123"), "ABC%21123");
    }
}
