//! What Android tells the page (`SystemPlugin.kt`): Back, and the screen
//! turning off; and what the app asks of Android: the background upload, and
//! whether its screens may be captured.

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

#[derive(Serialize)]
struct ScreenshotArgs {
    allowed: bool,
}

impl<R: Runtime> System<R> {
    /// Lets screenshots and screen recording show the app's screens, or not
    /// (`FLAG_SECURE`), at once and for every screen opened from now on.
    pub fn allow_screenshots(&self, allowed: bool) -> Result<(), String> {
        self.0.run_mobile_plugin::<serde_json::Value>("allowScreenshots", ScreenshotArgs { allowed }).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Asks WorkManager to upload what the state file `state` says is waiting,
    /// once there is a network for a `cloud` store (`UploadWorker.kt`).
    pub fn schedule_upload(&self, state: &str, cloud: bool) -> Result<(), String> {
        self.0.run_mobile_plugin::<serde_json::Value>("scheduleUpload", UploadArgs { state, cloud }).map(|_| ()).map_err(|e| e.to_string())
    }
}
