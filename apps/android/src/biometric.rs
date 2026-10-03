//! Biometric unlock (`BiometricPlugin.kt`, stage A3): the database's key,
//! sealed with a Keystore key that a strong biometric unlocks.

use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{Manager, Runtime};
use zeroize::Zeroizing;

pub struct Biometric<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("biometric")
        .setup(|app, api| {
            let handle = api.register_android_plugin("io.github.olegg90.pswmanager", "BiometricPlugin")?;
            app.manage(Biometric(handle));
            Ok(())
        })
        .build()
}

#[derive(Deserialize)]
pub struct Status {
    /// A strong biometric is enrolled on this phone.
    pub available: bool,
    /// A sealed key is kept.
    pub stored: bool,
}

/// Why the key could not be had.
pub enum Failure {
    /// The user chose the master password instead.
    Cancelled,
    /// A new fingerprint or face was enrolled: the sealed key is gone.
    Invalidated,
    /// Nothing is stored.
    None,
    Other(String),
}

#[derive(Serialize)]
struct StoreArgs<'a> {
    secret: &'a str,
    title: &'a str,
}

#[derive(Serialize)]
struct PromptArgs<'a> {
    title: &'a str,
}

#[derive(Deserialize)]
struct Retrieved {
    secret: String,
}

impl<R: Runtime> Biometric<R> {
    pub fn status(&self) -> Result<Status, String> {
        self.0.run_mobile_plugin("status", ()).map_err(|e| e.to_string())
    }

    /// Seals `secret` after the user confirms with a fingerprint or face.
    pub fn store(&self, secret: &str) -> Result<(), Failure> {
        let args = StoreArgs { secret, title: "Unlock with your fingerprint next time" };
        self.0.run_mobile_plugin::<serde_json::Value>("store", args).map(|_| ()).map_err(failure)
    }

    /// The sealed secret, after a fingerprint or face.
    pub fn retrieve(&self) -> Result<Zeroizing<String>, Failure> {
        let retrieved: Retrieved = self.0.run_mobile_plugin("retrieve", PromptArgs { title: "Unlock PswManager" }).map_err(failure)?;
        Ok(Zeroizing::new(retrieved.secret))
    }

    pub fn forget(&self) -> Result<(), String> {
        self.0.run_mobile_plugin::<serde_json::Value>("forget", ()).map(|_| ()).map_err(|e| e.to_string())
    }
}

fn failure(error: tauri::plugin::mobile::PluginInvokeError) -> Failure {
    let message = error.to_string();
    if message.contains("biometric:cancelled") {
        Failure::Cancelled
    } else if message.contains("biometric:invalidated") {
        Failure::Invalidated
    } else if message.contains("biometric:none") {
        Failure::None
    } else {
        Failure::Other(message)
    }
}
