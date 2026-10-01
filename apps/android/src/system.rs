//! What Android tells the page (`SystemPlugin.kt`): Back, and the screen
//! turning off.

use tauri::plugin::{Builder, TauriPlugin};
use tauri::Runtime;

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("system")
        .setup(|_app, api| {
            api.register_android_plugin("io.github.olegg90.pswmanager", "SystemPlugin")?;
            Ok(())
        })
        .build()
}
