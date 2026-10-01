//! Android's Back goes to the page first (`BackPlugin.kt`).

use tauri::plugin::{Builder, TauriPlugin};
use tauri::Runtime;

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("back")
        .setup(|_app, api| {
            api.register_android_plugin("io.github.olegg90.pswmanager", "BackPlugin")?;
            Ok(())
        })
        .build()
}
