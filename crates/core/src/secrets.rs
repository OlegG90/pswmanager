//! Secrets kept between runs (a cloud account's refresh token, the icon
//! cache's key), never in the state file. Where they are kept belongs to the
//! platform: the app installs its store once at start (the Windows Credential
//! Manager on Windows). Without one, nothing is kept: a sign-in lasts until
//! the app quits, and site icons are cached in memory only.

use std::sync::OnceLock;
use zeroize::Zeroizing;

pub trait SecretStore: Send + Sync {
    fn write(&self, name: &str, secret: &str) -> Result<(), String>;
    /// The secret, or `None` when there is none.
    fn read(&self, name: &str) -> Option<Zeroizing<String>>;
    fn delete(&self, name: &str);
}

static STORE: OnceLock<Box<dyn SecretStore>> = OnceLock::new();

/// Sets where secrets are kept; only the first call counts.
pub fn install(store: Box<dyn SecretStore>) {
    let _ = STORE.set(store);
}

pub fn write(name: &str, secret: &str) -> Result<(), String> {
    STORE.get().ok_or("This device has nowhere to keep the sign-in")?.write(name, secret)
}

pub fn read(name: &str) -> Option<Zeroizing<String>> {
    STORE.get()?.read(name)
}

pub fn delete(name: &str) {
    if let Some(store) = STORE.get() {
        store.delete(name);
    }
}
