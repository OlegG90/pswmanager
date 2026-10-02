//! PswManager for Android (`docs/spec-android.md`): the phone's shell around
//! the shared core.

#[cfg(mobile)]
mod app;
#[cfg(mobile)]
mod background;
#[cfg(mobile)]
mod clipboard;
#[cfg(mobile)]
mod documents;
#[cfg(mobile)]
mod dropbox;
#[cfg(mobile)]
mod editing;
#[cfg(mobile)]
mod icons;
#[cfg(mobile)]
mod secrets;
#[cfg(mobile)]
mod system;
#[cfg(mobile)]
mod visible;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(mobile)]
    let builder = app::setup(builder);
    builder.run(tauri::generate_context!()).expect("PswManager could not start");
}
