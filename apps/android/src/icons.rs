//! Site icons on the phone: the core's cache ([pswm_core::icons]) in the
//! app's data folder, its file names hashed with a key from the Keystore.
//! Made on first use off the main thread (reading that key goes through a
//! plugin, which waits for the main thread).

use crate::app::off_main;
use pswm_core::icons::{self, Cache};
use pswm_core::store::Store;
use pswm_core::vault::{Kind, Listing};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Default)]
pub struct Icons(OnceLock<Cache>);

/// Runs `f` with the cache, made on first use.
fn with_cache<T>(app: &AppHandle, f: impl FnOnce(&Cache) -> T) -> T {
    let icons = app.state::<Icons>();
    f(icons.0.get_or_init(|| Cache::in_data_dir(app.state::<Store>().dir())))
}

/// Fetches the listed sites' missing icons in the background, and tells the
/// page about each one that arrives (`icon-ready`).
pub fn fetch(app: &AppHandle, listing: &Listing) {
    // Only the entries in use: not the recycle bin's or the templates' sites.
    let mut hosts: Vec<String> = listing.entries.iter().filter(|e| e.kind == Kind::Entry).filter_map(|e| e.host.clone()).collect();
    hosts.sort();
    hosts.dedup();
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_cache(&app, |cache| {
            cache.fetch_missing(hosts, |host| {
                let _ = app.emit("icon-ready", host);
            })
        });
    });
}

/// A site's icon as a `data:` URL, if the cache has one.
#[tauri::command]
pub async fn icon(app: AppHandle, host: String) -> Result<Option<String>, String> {
    off_main(move || Ok(with_cache(&app, |cache| cache.get(&host)).and_then(|bytes| icons::data_url(&bytes)))).await
}
