//! Folders and files the user picks, through Android's Storage Access
//! Framework (`DocumentsPlugin.kt`): the platform's side of the core's
//! [Mirror], which keeps a document and the copy the core works on in step.

use base64::Engine;
use pswm_core::mirror::{Document, InStep, Mirror, MirrorError, Stamp};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{AppHandle, Manager, Runtime, Wry};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// The plugin's handle, kept in the app's state.
pub struct Documents<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("documents")
        .setup(|app, api| {
            let handle = api.register_android_plugin("io.github.olegg90.pswmanager", "DocumentsPlugin")?;
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

#[derive(Deserialize)]
struct Answer {
    uri: Option<String>,
    #[serde(default)]
    name: String,
}

#[derive(Serialize)]
struct Uri<'a> {
    uri: &'a str,
}

#[derive(Deserialize)]
struct Data {
    data: String,
}

/// A document, read and written through the plugin.
pub struct SafDocument<R: Runtime> {
    handle: PluginHandle<R>,
    pub uri: String,
}

impl<R: Runtime> Documents<R> {
    fn call<T: serde::de::DeserializeOwned>(&self, method: &str, args: impl Serialize) -> Result<T, String> {
        self.0.run_mobile_plugin(method, args).map_err(|e| e.to_string())
    }

    fn pick(&self, method: &str) -> Result<Option<Picked>, String> {
        let answer: Answer = self.call(method, ())?;
        Ok(answer.uri.map(|uri| Picked { uri, name: answer.name }))
    }

    pub fn pick_folder(&self) -> Result<Option<Picked>, String> {
        self.pick("pickFolder")
    }

    pub fn pick_file(&self) -> Result<Option<Picked>, String> {
        self.pick("pickFile")
    }

    /// The document named `name` in a picked folder, made when it is not there.
    pub fn child(&self, folder: &str, name: &str) -> Result<SafDocument<R>, String> {
        #[derive(Serialize)]
        struct Args<'a> {
            folder: &'a str,
            name: &'a str,
        }
        #[derive(Deserialize)]
        struct Child {
            uri: String,
        }
        let child: Child = self.call("child", Args { folder, name })?;
        Ok(self.document(child.uri))
    }

    pub fn document(&self, uri: String) -> SafDocument<R> {
        SafDocument { handle: self.0.clone(), uri }
    }
}

impl<R: Runtime> Document for SafDocument<R> {
    fn stamp(&self) -> Result<Stamp, String> {
        self.handle.run_mobile_plugin("stamp", Uri { uri: &self.uri }).map_err(|e| e.to_string())
    }

    fn read(&self) -> Result<Vec<u8>, String> {
        let data: Data = self.handle.run_mobile_plugin("read", Uri { uri: &self.uri }).map_err(|e| e.to_string())?;
        B64.decode(data.data).map_err(|e| e.to_string())
    }

    fn write(&self, bytes: &[u8]) -> Result<(), String> {
        #[derive(Serialize)]
        struct Args<'a> {
            uri: &'a str,
            data: String,
        }
        let _: serde_json::Value = self
            .handle
            .run_mobile_plugin("write", Args { uri: &self.uri, data: B64.encode(bytes) })
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// The plugin waits for Android's main thread, so it is never called from it.
async fn off_main<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<Picked>, String> {
    off_main(move || app.state::<Documents<Wry>>().pick_folder()).await
}

#[tauri::command]
pub async fn pick_file(app: AppHandle) -> Result<Option<Picked>, String> {
    off_main(move || app.state::<Documents<Wry>>().pick_file()).await
}

/// A check of the whole round trip in a picked folder, on a file of its own
/// (`pswm-check.txt`, never a database): written, pulled into a copy, the copy
/// changed and pushed back with a backup; what happened, line by line.
#[tauri::command]
pub async fn check_folder(app: AppHandle, folder: String) -> Result<Vec<String>, String> {
    off_main(move || {
        let documents = app.state::<Documents<Wry>>();
        let copy: PathBuf = app.path().app_data_dir().map_err(|e| e.to_string())?.join("check").join("pswm-check.txt");
        let mut report = Vec::new();
        let document = documents.child(&folder, "pswm-check.txt")?;
        document.write(format!("Written by PswManager at {}\n", now()).as_bytes())?;
        report.push(format!("Written: {:?}", document.stamp()?));
        let mut mirror = Mirror {
            document,
            backup: Some(documents.child(&folder, "pswm-check.txt.bak")?),
            copy: copy.clone(),
            in_step: InStep::default(),
        };
        report.push(format!("Pulled: {}", mirror.pull()?));
        let mut text = std::fs::read_to_string(&copy).map_err(|e| e.to_string())?;
        text.push_str("Changed in the copy\n");
        std::fs::write(&copy, text).map_err(|e| e.to_string())?;
        match mirror.push() {
            Ok(pushed) => report.push(format!("Pushed: {pushed}")),
            Err(MirrorError::Changed) => report.push("Not pushed: the file changed meanwhile".into()),
            Err(MirrorError::Failed(e)) => return Err(e),
        }
        report.push(format!("Now: {}", String::from_utf8_lossy(&mirror.document.read()?).replace('\n', " / ")));
        report.push(format!("Pulled again: {}", mirror.pull()?));
        Ok(report)
    })
    .await
}

fn now() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}
