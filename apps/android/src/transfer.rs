//! Importing from another password manager on this phone (#152) through
//! `TransferPlugin.kt` (Android's Credential Transfer): the export comes
//! straight here, never to the page, and the core reads it into new entries
//! ([pswm_core::cxf]) in a group of their own.

use crate::app::off_main;
use pswm_core::cxf::{self, Skipped};
use pswm_core::session::Session;
use pswm_core::vault::Listing;
use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{AppHandle, Manager, Runtime, Wry};
use zeroize::Zeroizing;

pub struct Transfer<R: Runtime>(PluginHandle<R>);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("transfer")
        .setup(|app, api| {
            app.manage(Transfer(api.register_android_plugin("io.github.olegg90.pswmanager", "TransferPlugin")?));
            Ok(())
        })
        .build()
}

#[derive(Deserialize)]
struct Exported {
    json: String,
}

impl<R: Runtime> Transfer<R> {
    /// The export of the app the user picks; `None` when they went back.
    fn import(&self) -> Result<Option<Zeroizing<String>>, String> {
        match self.0.run_mobile_plugin::<Exported>("importCredentials", ()) {
            Ok(exported) => Ok(Some(Zeroizing::new(exported.json))),
            Err(e) => {
                let message = e.to_string();
                if message.contains("transfer:cancelled") {
                    Ok(None)
                } else if message.contains("transfer:none") {
                    Err("No app on this phone offers its passwords for import".into())
                } else {
                    Err(format!("The import did not work: {message}"))
                }
            }
        }
    }
}

/// What an import brought.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    /// The app it came from.
    pub exporter: String,
    /// The group the entries were put in.
    pub group: String,
    pub added: usize,
    /// What could not be brought over, and why.
    pub skipped: Vec<Skipped>,
    pub listing: Listing,
}

/// Imports from another app on this phone: the system's list of apps, then
/// everything the chosen one exports as new entries in the group
/// `Imported from <app> <date>`, saved and synced as any change. `None` when
/// the user went back.
#[tauri::command]
pub async fn import_from_app(app: AppHandle) -> Result<Option<Imported>, String> {
    off_main(move || {
        let session = app.state::<Session>();
        if !session.is_unlocked() {
            return Err("Unlock the database first".into());
        }
        let Some(json) = app.state::<Transfer<Wry>>().import()? else { return Ok(None) };
        let import = cxf::parse(&json)?;
        drop(json);
        if !session.is_unlocked() {
            return Err("The database locked while the other app was open: unlock it and import again".into());
        }
        let group = import.group_name_today();
        let (added, listing) = session.with_mut(|v| {
            let added = v.import(&import, &group)?;
            Ok((added, v.listing()))
        })?;
        if added > 0 {
            crate::icons::fetch(&app, &listing);
            crate::editing::upload_soon(&app);
        }
        Ok(Some(Imported { exporter: import.exporter.clone(), group, added, skipped: import.skipped.clone(), listing }))
    })
    .await
}
