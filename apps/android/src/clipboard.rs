//! Copying a secret through `ClipboardPlugin.kt`: marked sensitive and cleared
//! after a while (see there).

use serde::Serialize;
use std::time::Duration;
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{Manager, Runtime};

pub struct Clipboard<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("clipboard")
        .setup(|app, api| {
            app.manage(Clipboard(api.register_android_plugin("io.github.olegg90.pswmanager", "ClipboardPlugin")?));
            Ok(())
        })
        .build()
}

impl<R: Runtime> Clipboard<R> {
    pub fn copy(&self, text: &str, clear_after: Duration) -> Result<(), String> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Args<'a> {
            text: &'a str,
            clear_after_ms: u64,
        }
        let args = Args { text, clear_after_ms: clear_after.as_millis() as u64 };
        self.0.run_mobile_plugin::<serde_json::Value>("copy", args).map(|_| ()).map_err(|e| e.to_string())
    }
}
