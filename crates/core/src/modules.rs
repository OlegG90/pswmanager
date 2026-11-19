//! In-tree optional modules (#412): the one pattern every feature that can
//! be present or not follows — import and export formats, cloud stores, icon
//! packs and autofill categories. Nothing here loads code at runtime: each
//! module is compiled in, listed through a registry, and toggled in settings
//! where it makes sense. A missing or disabled module means its UI entries
//! are gone, never an action that breaks.
//!
//! The levels, as `docs/spec.md` states them:
//!
//! - **L0 core (never modular):** the KDBX read/write, crypto and key, TOTP,
//!   passkeys, the autofill router's policy (who asks, limits, confirmations,
//!   linking), icon-fetch policy, lock/clipboard/screenshots.
//! - **L1 built-in modules:** [Importer], [Exporter], [StorePlugin],
//!   [CategoryFiller], behind registries, compiled in.
//! - **L2 data packs:** icon sets and field vocabularies, validated data only.
//!
//! The traits stay `pub(crate)` until Phase 1 settles them; each category's
//! handlers and its registry live in the module that owns the behaviour
//! (`file_import`, `file_export`, `remote`, `autofill`).

use crate::cxf::Import;
use crate::file_export::Export;
use crate::file_export::Format;
use crate::remote::{Cloud, CloudFile, Location, Remote, RemoteError};
use crate::settings::Settings;
use crate::store::Store;
use keepass::db::EntryRef;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

/// An import format (CSV, Bitwarden's JSON, CXF, Aegis): it decides whether a
/// file is its format, and reads it into a [crate::cxf::Import]. The core owns
/// what happens next (adding the entries, tagging, wiping, history); an
/// importer owns only `id/label/extensions/detect/parse` and its `skipped`
/// reasons.
pub(crate) trait Importer: Send + Sync {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    /// The file name endings this format is known by, a hint for the picker.
    fn extensions(&self) -> &'static [&'static str];

    /// Whether this is the importer for the file: for the CSV importer, `json`
    /// is `None` and `head` (the file's first bytes) decides — it takes any
    /// file the JSON importers do not; for a JSON importer, `json` is the file
    /// parsed (already, to be wiped by the caller) and its keys decide.
    fn detect(&self, name: &str, head: &[u8], json: Option<&Value>) -> bool;

    /// What the importer reads, for its row in the tools: one line, no secrets.
    fn hint(&self) -> &'static str;

    /// Reads the file as this format: `json` for a JSON importer (the file
    /// parsed, wiped by the caller), `text` for the CSV importer (the file's
    /// text), `password` for an encrypted export that needs one.
    fn parse(&self, name: &str, json: Option<&Value>, text: &str, password: Option<&str>) -> Result<Import, String>;
}

/// An export format (CSV, CXF): it writes the entries in use to one file. The
/// core owns the `write_new` safety (create-new, no symlink, no half-written
/// file), the file name and the warnings; an exporter owns
/// `format/extension/export` and what its format leaves out.
pub(crate) trait Exporter: Send + Sync {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn extension(&self) -> &'static str;
    /// What the format carries, for its row in the tools: one line, no secrets.
    fn hint(&self) -> &'static str;

    /// The entries as this format, every secret in clear at full size, wiped
    /// when dropped. `account` is the database's name (CXF names the account
    /// with it). The core fills in the file name afterwards.
    fn export<'a>(&self, account: &str, entries: Vec<EntryRef<'a>>) -> Export;
}

/// A store a synced database lives in besides this device (a folder, Dropbox,
/// Google Drive, OneDrive, a platform document): it lists, creates and opens
/// the remote files. The core owns the sync decision, merge, the
/// `.bak`/`.remote.bak` copies, the working-copy hash, background uploads
/// without the key, and where tokens are kept; a store plugin owns
/// `id/list/create/open`, its OAuth provider and its capabilities.
pub(crate) trait StorePlugin: Send + Sync {
    /// The store's id: `"folder" | "dropbox" | "google" | "onedrive" | "document"`.
    fn id(&self) -> &'static str;
    /// What the user sees: "Folder", "Dropbox".
    fn title(&self) -> &'static str;
    /// What the store holds, for its row in the settings and the sync setup:
    /// one line, no secrets.
    fn hint(&self) -> &'static str;
    /// Whether talking to the store needs the network (clouds do).
    fn needs_network(&self) -> bool;
    /// Whether the store can be turned off. A cloud can; a folder and a
    /// platform document are how the app reaches a file at all (there is no
    /// connection of their own to turn off), so they cannot.
    fn optional(&self) -> bool {
        true
    }

    /// The databases the store holds; a folder or a document has none to list.
    fn list(&self) -> Result<Vec<CloudFile>, RemoteError>;
    /// A new file in the store, answered with where it is and its revision; a
    /// folder or a document does not create one.
    fn create(&self, name: &str, bytes: &[u8]) -> Result<(Location, String), RemoteError>;
    /// The remote file `location` (this plugin's kind) names.
    fn open(&self, location: &Location) -> Box<dyn Remote>;

    /// The OAuth provider a cloud signs in with; `None` for a folder or a
    /// document.
    fn provider(&self) -> Option<&'static crate::oauth::Provider>;
    /// Where the database is synced to, for the window.
    fn describe(&self, location: &Location) -> String;
    /// For status lines: "the folder", "Dropbox".
    fn name(&self) -> &'static str;
}

/// An autofill category (logins now, cards as the pilot): what the settings
/// and the sync sheets list. The category's own behaviour lives beside it:
/// the login decisions on [crate::autofill::LoginFiller], the card ones in
/// [crate::card]. The core's router owns the [crate::autofill] policy — the `Asker`
/// check on every offer and answer, locked answers, limits, confirmations
/// and wiping — and reads which fillers are turned off: a filler turned off
/// in the settings offers nothing and refuses, through the router.
pub(crate) trait CategoryFiller: Send + Sync {
    /// The category's id: `"login"` now, `"card"` and more later.
    fn id(&self) -> &'static str;
    /// What the user sees: "Logins", "Cards".
    fn label(&self) -> &'static str;
    /// What the category fills, for its row in the settings: one line, no secrets.
    fn hint(&self) -> &'static str;
}

/// One module as the frontend lists it (a Tools entry, a store, a filler).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Module {
    /// The module's own id: `"csv"`, `"dropbox"`, `"login"`.
    pub id: String,
    /// `"importers" | "exporters" | "stores" | "fillers"`.
    pub kind: &'static str,
    /// What the user sees: "CSV", "Dropbox", "Logins".
    pub label: String,
    /// What the module does, for its row in the tools and the settings: one
    /// line, no secrets. The single source: the frontend keeps no copy of it.
    pub hint: String,
    /// Off in the settings screen; the frontend leaves it out.
    pub enabled: bool,
    /// The file name endings an importer or exporter is known by; empty for
    /// stores and fillers.
    pub extensions: Vec<String>,
    /// Whether talking to the store needs the network; `None` unless it is a
    /// store.
    pub needs_network: Option<bool>,
    /// Whether it can be turned off: a folder and a platform document cannot.
    pub optional: bool,
    /// A store a database here syncs with: it cannot be turned off till that
    /// database stops syncing there.
    pub in_use: bool,
}

/// Every compiled-in module, for the frontend's tools and sheets, with
/// whether each is turned off on this device (the module keys, a kind and an
/// id joined by `:`, the user turned off in the settings screen) and, for a
/// store, whether it can be turned off and whether a database syncs with it.
pub fn list(store: &Store) -> Vec<Module> {
    let in_use = stores_in_use(store);
    let disabled = Settings::of(store).disabled_modules();
    let one = |kind: &'static str, id: &str, label: &str, hint: &str, extensions: &[&str], needs_network: Option<bool>| Module {
        enabled: !off_in(&disabled, kind, id),
        optional: kind != "stores" || store_optional(id),
        in_use: kind == "stores" && in_use.contains(id),
        id: id.to_string(),
        kind,
        label: label.to_string(),
        hint: hint.to_string(),
        extensions: extensions.iter().map(|e| e.to_string()).collect(),
        needs_network,
    };
    let mut modules = Vec::new();
    for importer in crate::file_import::importers() {
        modules.push(one("importers", importer.id(), importer.label(), importer.hint(), importer.extensions(), None));
    }
    for exporter in crate::file_export::exporters() {
        modules.push(one("exporters", exporter.id(), exporter.label(), exporter.hint(), &[exporter.extension()], None));
    }
    for store in crate::remote::store_plugins() {
        modules.push(one("stores", store.id(), store.title(), store.hint(), &[], Some(store.needs_network())));
    }
    for filler in crate::autofill::category_fillers() {
        modules.push(one("fillers", filler.id(), filler.label(), filler.hint(), &[], None));
    }
    modules
}

/// Whether the module `kind:id` is turned off on this device; never a store
/// that cannot be (a folder, a platform document), whatever the setting says.
pub fn is_off(store: &Store, kind: &str, id: &str) -> bool {
    off_in(&Settings::of(store).disabled_modules(), kind, id)
}

/// [is_off], with the keys turned off read already.
fn off_in(disabled: &HashSet<String>, kind: &str, id: &str) -> bool {
    (kind != "stores" || store_optional(id)) && disabled.contains(&format!("{kind}:{id}"))
}

/// Whether the store `id` can be turned off ([StorePlugin::optional]).
fn store_optional(id: &str) -> bool {
    crate::remote::store_plugins().iter().find(|plugin| plugin.id() == id).is_none_or(|plugin| plugin.optional())
}

/// The stores the databases here sync with, by id.
fn stores_in_use(store: &Store) -> HashSet<&'static str> {
    store.read(|s| s.databases.iter().filter_map(|d| d.remote.as_ref()).map(|r| r.location.kind()).collect())
}

/// Refuses a new `disabledModules` value (`turned_off`) that turns off a
/// store a database here syncs with (stop syncing there first), or one that
/// cannot be turned off. Keys turned off already stay as they are.
pub(crate) fn check_turning_off(store: &Store, turned_off: &Value) -> Result<(), String> {
    let before = Settings::of(store).disabled_modules();
    let in_use = stores_in_use(store);
    let new = turned_off.as_array().into_iter().flatten().filter_map(Value::as_str).filter(|key| !before.contains(*key));
    for id in new.filter_map(|key| key.strip_prefix("stores:")) {
        let title = crate::remote::store_plugins().iter().find(|plugin| plugin.id() == id).map_or(id, |plugin| plugin.title());
        if !store_optional(id) {
            return Err(format!("{title} cannot be turned off"));
        }
        if in_use.contains(id) {
            return Err(format!("A database here syncs with {title}: stop syncing it there first"));
        }
    }
    Ok(())
}

/// Refuses the file `name` (`content`) when the importer it needs is turned
/// off on this device, with that said; a file no importer takes is left for
/// the parser to refuse. Both apps ask before the file is parsed or held for
/// its password.
pub fn check_importer(store: &Store, name: &str, content: &[u8]) -> Result<(), String> {
    match crate::file_import::detect(name, content) {
        Some(id) if is_off(store, "importers", id) => {
            let label = crate::file_import::filter(id).map_or(id, |(label, _)| label);
            Err(format!("This file is a {label} export, but that importer is turned off in Settings › Modules"))
        }
        _ => Ok(()),
    }
}

/// Refuses an export to `format` when its exporter is turned off on this device.
pub fn check_exporter(store: &Store, format: Format) -> Result<(), String> {
    if is_off(store, "exporters", format.id()) {
        return Err(format!("The {} format is turned off in Settings › Modules", format.label()));
    }
    Ok(())
}

/// Refuses to start a sign-in to `cloud`, or to make a file there, when the
/// store is turned off on this device: nothing new links to it. The core
/// refuses the link itself ([crate::sync]); asking first means no browser
/// opens and no remote file is made for nothing.
pub fn check_cloud(store: &Store, cloud: Cloud) -> Result<(), String> {
    if is_off(store, "stores", cloud.id()) {
        return Err(store_turned_off(cloud.provider().name));
    }
    Ok(())
}

/// Why a store turned off is refused, `store` naming it as a sentence starts.
pub(crate) fn store_turned_off(store: &str) -> String {
    format!("{store} is turned off in Settings › Modules: turn it back on to sync there")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_module_turned_off_is_refused_with_where_to_turn_it_on() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path().join("pswm.json"));
        let bitwarden = br#"{"items": []}"#;
        assert!(check_importer(&store, "bw.json", bitwarden).is_ok());
        assert!(check_exporter(&store, Format::Cxf).is_ok());
        assert!(check_cloud(&store, Cloud::Dropbox).is_ok());
        Settings::of(&store).set("disabledModules", serde_json::json!(["importers:bitwarden", "exporters:cxf", "stores:dropbox"])).unwrap();
        for refused in [check_importer(&store, "bw.json", bitwarden), check_exporter(&store, Format::Cxf), check_cloud(&store, Cloud::Dropbox)] {
            assert!(refused.unwrap_err().contains("turned off in Settings › Modules"));
        }
        // Another format, or a file no importer takes, is not this check's to refuse.
        assert!(check_importer(&store, "x.csv", b"name,password\n").is_ok());
        assert!(check_importer(&store, "other.json", br#"{"foo": 1}"#).is_ok());
        assert!(check_exporter(&store, Format::Csv).is_ok());
    }

    #[test]
    fn importers_are_listed_in_reading_order() {
        let ids: Vec<&str> = crate::file_import::importers().iter().map(|importer| importer.id()).collect();
        assert_eq!(ids, ["aegis", "bitwarden", "cxf", "csv"]);
        for importer in crate::file_import::importers() {
            assert!(!importer.label().is_empty());
            assert!(!importer.extensions().is_empty());
        }
    }

    #[test]
    fn exporters_cover_both_formats() {
        let ids: Vec<&str> = crate::file_export::exporters().iter().map(|exporter| exporter.id()).collect();
        assert_eq!(ids, ["csv", "cxf"]);
        for format in [Format::Csv, Format::Cxf] {
            let exporter = format.exporter();
            assert!(!exporter.label().is_empty());
            assert_eq!(exporter.extension(), format.extension());
        }
    }

    #[test]
    fn store_plugins_cover_every_kind() {
        let ids: Vec<&str> = crate::remote::store_plugins().iter().map(|plugin| plugin.id()).collect();
        assert_eq!(ids, ["folder", "dropbox", "google", "onedrive", "document"]);
        for plugin in crate::remote::store_plugins() {
            assert!(!plugin.name().is_empty());
            assert!(!plugin.title().is_empty());
            assert!(!plugin.hint().is_empty());
            let _ = plugin.needs_network();
        }
    }

    #[test]
    fn category_fillers_start_with_logins_then_cards() {
        let ids: Vec<&str> = crate::autofill::category_fillers().iter().map(|filler| filler.id()).collect();
        assert_eq!(ids, ["login", "card"]);
        for filler in crate::autofill::category_fillers() {
            assert!(!filler.label().is_empty());
            assert!(!filler.hint().is_empty());
        }
    }

    #[test]
    fn list_reflects_disabled_modules() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path().join("pswm.json"));
        let all = list(&store);
        for kind in ["importers", "exporters", "stores", "fillers"] {
            assert!(all.iter().any(|m| m.kind == kind), "{kind} is listed");
        }
        assert!(all.iter().all(|m| m.enabled));
        assert!(all.iter().all(|m| !m.label.is_empty() && !m.hint.is_empty()), "every module lists a label and a hint");
        Settings::of(&store).set("disabledModules", serde_json::json!(["importers:aegis"])).unwrap();
        let filtered = list(&store);
        let aegis = filtered.iter().find(|m| m.kind == "importers" && m.id == "aegis").unwrap();
        assert!(!aegis.enabled);
        let csv = filtered.iter().find(|m| m.kind == "importers" && m.id == "csv").unwrap();
        assert!(csv.enabled);
    }

    #[test]
    fn a_folder_a_document_and_a_store_in_use_cannot_be_turned_off() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::load(dir.path().join("pswm.json"));
        let stores = |store: &Store| list(store).into_iter().filter(|m| m.kind == "stores").map(|m| (m.id, m.optional, m.in_use)).collect::<Vec<_>>();
        assert_eq!(stores(&store), [
            ("folder".into(), false, false), ("dropbox".into(), true, false), ("google".into(), true, false), ("onedrive".into(), true, false), ("document".into(), false, false),
        ]);
        let refused = Settings::of(&store).set("disabledModules", serde_json::json!(["stores:folder"])).unwrap_err();
        assert_eq!(refused, "Folder cannot be turned off");
        // A database here syncs with Dropbox: Dropbox stays on till it stops syncing there.
        store.update(|s| s.databases.push(crate::store::Known {
            file: dir.path().join("base.kdbx"),
            key_file: None,
            remote: Some(crate::store::Remote { location: Location::Dropbox { path: "/base.kdbx".into() }, revision: None, synced: None }),
            name: None,
            description: None,
            backup: None,
        })).unwrap();
        assert!(stores(&store).contains(&("dropbox".into(), true, true)));
        let refused = Settings::of(&store).set("disabledModules", serde_json::json!(["stores:dropbox"])).unwrap_err();
        assert_eq!(refused, "A database here syncs with Dropbox: stop syncing it there first");
        assert!(Settings::of(&store).set("disabledModules", serde_json::json!(["stores:google"])).is_ok());
        // A folder turned off by an older setting is on all the same.
        store.update(|s| { s.settings.insert("disabledModules".into(), serde_json::json!(["stores:folder"])); }).unwrap();
        assert!(!is_off(&store, "stores", "folder"));
    }
}
