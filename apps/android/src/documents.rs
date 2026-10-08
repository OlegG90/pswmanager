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
    create: bool,
}

#[derive(Deserialize)]
struct ChildAnswer {
    uri: Option<String>,
}

#[derive(Deserialize)]
struct PickAnswer {
    uri: Option<String>,
    #[serde(default)]
    name: String,
    /// Bytes, for a file picked to read; -1 when the provider does not say.
    #[serde(default)]
    size: i64,
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

    /// Hands a file in the app's cache to another app.
    pub fn open_file(&self, path: &std::path::Path) -> Result<(), String> {
        #[derive(Serialize)]
        struct Args<'a> {
            path: &'a str,
        }
        self.call::<serde_json::Value>("openFile", Args { path: &path.to_string_lossy() }).map(|_| ())
    }

    /// A `.kdbx` the user picks; `None` when they cancelled.
    pub fn pick_file(&self) -> Result<Option<Picked>, String> {
        self.pick("pickFile")
    }

    /// A file the user picks to attach, read once: its name and content;
    /// `None` when they cancelled. A file too big to attach is refused before
    /// it is read, when the provider tells its size.
    pub fn pick_to_read(&self) -> Result<Option<(String, Vec<u8>)>, String> {
        let answer: PickAnswer = self.call("pickToRead", ())?;
        let Some(uri) = answer.uri else { return Ok(None) };
        if let Ok(size) = u64::try_from(answer.size) {
            pswm_core::edit::check_size(size)?;
        }
        let content = self.read(&uri)?.ok_or(format!("Cannot read {}", answer.name))?;
        Ok(Some((answer.name, content)))
    }

    /// Where to save a new file, picked with Android's save picker (`name`
    /// suggested); `None` when they cancelled. No lasting access is kept.
    pub fn pick_to_save(&self, name: &str) -> Result<Option<String>, String> {
        #[derive(Serialize)]
        struct Args<'a> {
            name: &'a str,
        }
        Ok(self.call::<PickAnswer>("pickToSave", Args { name })?.uri)
    }

    /// A new document the user names with Android's save picker (`name`
    /// suggested), its access kept; `None` when they cancelled.
    pub fn pick_to_create(&self, name: &str) -> Result<Option<Picked>, String> {
        #[derive(Serialize)]
        struct Args<'a> {
            name: &'a str,
        }
        let answer: PickAnswer = self.call("pickToCreate", Args { name })?;
        Ok(answer.uri.map(|uri| Picked { uri, name: answer.name }))
    }

    /// A folder the user picks; `None` when they cancelled.
    pub fn pick_folder(&self) -> Result<Option<Picked>, String> {
        self.pick("pickFolder")
    }

    /// The document named `name` in a picked folder, made when it is not there.
    pub fn child(&self, folder: &str, name: &str) -> Result<String, String> {
        let answer: ChildAnswer = self.call("child", ChildArgs { folder, name, create: true })?;
        answer.uri.ok_or_else(|| format!("Cannot make {name} in the folder"))
    }

    /// The document named `name` in a picked folder, if it is there.
    pub fn find(&self, folder: &str, name: &str) -> Result<Option<String>, String> {
        Ok(self.call::<ChildAnswer>("child", ChildArgs { folder, name, create: false })?.uri)
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
