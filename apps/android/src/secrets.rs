//! The core's secret store on Android (`SecretsPlugin.kt`): secrets encrypted
//! with a key in the Android Keystore. Installed at start.

use pswm_core::secrets::{self, SecretStore};
use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::Runtime;
use zeroize::Zeroizing;

struct Keystore<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("secrets")
        .setup(|_app, api| {
            let handle = api.register_android_plugin("io.github.olegg90.pswmanager", "SecretsPlugin")?;
            secrets::install(Box::new(Keystore(handle)));
            Ok(())
        })
        .build()
}

#[derive(Serialize)]
struct Name<'a> {
    name: &'a str,
}

#[derive(Serialize)]
struct Value<'a> {
    name: &'a str,
    secret: &'a str,
}

#[derive(Deserialize)]
struct Answer {
    secret: Option<String>,
}

impl<R: Runtime> SecretStore for Keystore<R> {
    fn write(&self, name: &str, secret: &str) -> Result<(), String> {
        self.0.run_mobile_plugin::<serde_json::Value>("write", Value { name, secret }).map(|_| ()).map_err(|e| e.to_string())
    }

    /// A secret that cannot be read (the plugin failed, the Keystore key is
    /// gone) is as good as none: the store asks to sign in again.
    fn read(&self, name: &str) -> Option<Zeroizing<String>> {
        let answer: Answer = self.0.run_mobile_plugin("read", Name { name }).ok()?;
        answer.secret.map(Zeroizing::new)
    }

    fn delete(&self, name: &str) {
        let _ = self.0.run_mobile_plugin::<serde_json::Value>("delete", Name { name });
    }
}
