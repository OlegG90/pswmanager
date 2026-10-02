//! What Android tells the page (`SystemPlugin.kt`): Back, and the screen
//! turning off; and what the app asks of Android: the background upload.

use serde::Serialize;
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{Manager, Runtime};

pub struct System<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("system")
        .setup(|app, api| {
            let handle = api.register_android_plugin("io.github.olegg90.pswmanager", "SystemPlugin")?;
            app.manage(System(handle));
            Ok(())
        })
        .build()
}

#[derive(Serialize)]
struct UploadArgs<'a> {
    state: &'a str,
    cloud: bool,
}

impl<R: Runtime> System<R> {
    /// Asks WorkManager to upload what the state file `state` says is waiting,
    /// once there is a network for a `cloud` store (`UploadWorker.kt`).
    pub fn schedule_upload(&self, state: &str, cloud: bool) -> Result<(), String> {
        self.0.run_mobile_plugin::<serde_json::Value>("scheduleUpload", UploadArgs { state, cloud }).map(|_| ()).map_err(|e| e.to_string())
    }
}
