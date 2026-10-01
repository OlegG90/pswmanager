//! Folders and files the user picks, through Android's Storage Access
//! Framework (`DocumentsPlugin.kt`). The core reaches documents through
//! [pswm_core::documents], which this installs at start.

use base64::Engine;
use pswm_core::documents::{self, DocumentStore};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{AppHandle, Manager, Runtime, Wry};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// The plugin, kept in the app's state and installed as the core's document store.
pub struct Documents<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("documents")
        .setup(|app, api| {
            let handle = api.register_android_plugin("io.github.olegg90.pswmanager", "DocumentsPlugin")?;
            documents::install(Box::new(Documents(handle.clone())));
            app.manage(Documents(handle));
            Ok(())
        })
        .build()
}

/// A folder or file the user picked; its access is kept across runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Picked {
    pub uri: String,
    pub name: String,
}

#[derive(Serialize)]
struct UriArgs<'a> {
    uri: &'a str,
}

#[derive(Serialize)]
struct WriteArgs<'a> {
    uri: &'a str,
    data: String,
}

#[derive(Serialize)]
struct ChildArgs<'a> {
    folder: &'a str,
    name: &'a str,
}

#[derive(Deserialize)]
struct PickAnswer {
    uri: Option<String>,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct UriAnswer {
    uri: String,
}

#[derive(Deserialize)]
struct Content {
    data: Option<String>,
}

impl<R: Runtime> Documents<R> {
    fn call<T: DeserializeOwned>(&self, method: &str, args: impl Serialize) -> Result<T, String> {
        self.0.run_mobile_plugin(method, args).map_err(|e| e.to_string())
    }

    fn pick(&self, method: &str) -> Result<Option<Picked>, String> {
        let answer: PickAnswer = self.call(method, ())?;
        Ok(answer.uri.map(|uri| Picked { uri, name: answer.name }))
    }

    /// The document named `name` in a picked folder, made when it is not there.
    pub fn child(&self, folder: &str, name: &str) -> Result<String, String> {
        Ok(self.call::<UriAnswer>("child", ChildArgs { folder, name })?.uri)
    }
}

impl<R: Runtime> DocumentStore for Documents<R> {
    fn read(&self, uri: &str) -> Result<Option<Vec<u8>>, String> {
        let content: Content = self.call("read", UriArgs { uri })?;
        content.data.map(|data| B64.decode(data).map_err(|e| e.to_string())).transpose()
    }

    fn write(&self, uri: &str, bytes: &[u8]) -> Result<(), String> {
        self.call::<serde_json::Value>("write", WriteArgs { uri, data: B64.encode(bytes) }).map(|_| ())
    }
}

/// The plugin waits for Android's main thread, so it is never called from it.
async fn off_main<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<Picked>, String> {
    off_main(move || app.state::<Documents<Wry>>().pick("pickFolder")).await
}

#[tauri::command]
pub async fn pick_file(app: AppHandle) -> Result<Option<Picked>, String> {
    off_main(move || app.state::<Documents<Wry>>().pick("pickFile")).await
}

/// Temporary, until the first-run screens use a picked folder (#118): a check
/// of a picked folder as a database's remote file, on a file of its own
/// (`pswm-check.txt`, never a database); what happened, line by line.
#[tauri::command]
pub async fn check_folder(app: AppHandle, folder: String) -> Result<Vec<String>, String> {
    use pswm_core::remote::{Remote, RemoteError};
    off_main(move || {
        let uri = app.state::<Documents<Wry>>().child(&folder, "pswm-check.txt")?;
        let file = documents::DocumentFile { uri };
        let message = |e: RemoteError| e.message();
        let before = file.revision().map_err(message)?;
        let written = file.upload(b"Written by PswManager\n", before.as_deref()).map_err(message)?;
        let (bytes, revision) = file.download().map_err(message)?;
        Ok(vec![
            format!("Uploaded over the revision there: {}", revision == written),
            format!("Read back: {}", String::from_utf8_lossy(&bytes).trim()),
            format!("An upload over an old revision is refused: {}", file.upload(b"stale", Some("old")) == Err(RemoteError::Changed)),
        ])
    })
    .await
}
