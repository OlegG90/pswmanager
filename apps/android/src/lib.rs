//! PswManager for Android (`docs/spec-android.md`): the phone's shell around
//! the shared core. For now it only shows that the core runs on the phone.

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
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![about])
        .run(tauri::generate_context!())
        .expect("PswManager could not start");
}
