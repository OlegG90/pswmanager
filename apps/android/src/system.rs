//! What Android tells the page (`SystemPlugin.kt`): Back, and the screen
//! turning off; and what the app asks of Android: the background upload,
//! whether its screens may be captured, and the phone's autofill service.

use serde::{Deserialize, Serialize};
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

/// Whether the phone can autofill, and whether PswManager is its autofill service (#236).
#[derive(Serialize, Deserialize)]
pub struct AutofillStatus {
    pub supported: bool,
    pub enabled: bool,
}

#[derive(Serialize)]
struct UploadArgs<'a> {
    state: &'a str,
    cloud: bool,
}

impl<R: Runtime> System<R> {
    /// The app's window follows the *Allow screenshots* setting at once
    /// (other screens read it as they open).
    pub fn apply_screenshots(&self) -> Result<(), String> {
        self.0.run_mobile_plugin::<serde_json::Value>("applyScreenshots", ()).map(|_| ()).map_err(|e| e.to_string())
    }

    pub fn autofill_status(&self) -> Result<AutofillStatus, String> {
        self.0.run_mobile_plugin("autofillStatus", ()).map_err(|e| e.to_string())
    }

    /// Opens Android's choice of autofill service, with PswManager offered.
    pub fn choose_autofill(&self) -> Result<(), String> {
        self.0.run_mobile_plugin::<serde_json::Value>("chooseAutofill", ()).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Asks WorkManager to upload what the state file `state` says is waiting,
    /// once there is a network for a `cloud` store (`UploadWorker.kt`).
    pub fn schedule_upload(&self, state: &str, cloud: bool) -> Result<(), String> {
        self.0.run_mobile_plugin::<serde_json::Value>("scheduleUpload", UploadArgs { state, cloud }).map(|_| ()).map_err(|e| e.to_string())
    }
}
