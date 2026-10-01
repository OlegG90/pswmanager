//! Folders and files the user picks, through Android's Storage Access
//! Framework (`DocumentsPlugin.kt`). The core reaches documents through
//! [pswm_core::documents], which this installs at start.

use base64::Engine;
use pswm_core::documents::{self, DocumentStore};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{Manager, Runtime};

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

/// A file the user picked; its access is kept across runs.
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

#[derive(Deserialize)]
struct PickAnswer {
    uri: Option<String>,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct Content {
    data: Option<String>,
}

impl<R: Runtime> Documents<R> {
    fn call<T: DeserializeOwned>(&self, method: &str, args: impl Serialize) -> Result<T, String> {
        self.0.run_mobile_plugin(method, args).map_err(|e| e.to_string())
    }

    /// A `.kdbx` the user picks; `None` when they cancelled.
    pub fn pick_file(&self) -> Result<Option<Picked>, String> {
        let answer: PickAnswer = self.call("pickFile", ())?;
        Ok(answer.uri.map(|uri| Picked { uri, name: answer.name }))
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
