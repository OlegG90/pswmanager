//! What Android tells the page: Back (`BackPlugin.kt`) and the screen
//! turning off (`ScreenPlugin.kt`).

use tauri::plugin::{Builder, TauriPlugin};
use tauri::Runtime;

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("system")
        .setup(|_app, api| {
            api.register_android_plugin("io.github.olegg90.pswmanager", "BackPlugin")?;
            api.register_android_plugin("io.github.olegg90.pswmanager", "ScreenPlugin")?;
            Ok(())
        })
        .build()
}
