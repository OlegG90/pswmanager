//! PswManager for Android (`docs/spec-android.md`): the phone's shell around
//! the shared core. For now it only shows that the core runs on the phone,
//! and checks the access to a folder the user picks.

#[cfg(mobile)]
mod documents;

use pswm_core::generator::{self, Options};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct About {
    version: &'static str,
    /// A password made by the core's generator, as a sign that it works here.
    sample: String,
}

#[tauri::command]
fn about() -> Result<About, String> {
    let options = Options { length: 20, upper: true, lower: true, digits: true, symbols: true, exclude_look_alikes: true };
    let sample = generator::generate(&options)?;
    Ok(About { version: env!("CARGO_PKG_VERSION"), sample: sample.to_string() })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(mobile)]
    let builder = builder.plugin(documents::init()).invoke_handler(tauri::generate_handler![
        about,
        documents::pick_folder,
        documents::pick_file,
        documents::check_folder
    ]);
    #[cfg(not(mobile))]
    let builder = builder.invoke_handler(tauri::generate_handler![about]);
    builder
        .run(tauri::generate_context!())
        .expect("PswManager could not start");
}
