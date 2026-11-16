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
use crate::remote::{CloudFile, Location, Remote, RemoteError};
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
}

/// Every compiled-in module, for the frontend's tools and sheets; `disabled`
/// holds the module keys (a kind and an id joined by `:`) the user turned off
/// in the settings screen.
pub fn list(disabled: &HashSet<String>) -> Vec<Module> {
    let one = |kind: &'static str, id: &str, label: &str, hint: &str, extensions: &[&str], needs_network: Option<bool>| Module {
        enabled: !disabled.contains(&format!("{kind}:{id}")),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_export::Format;

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
        let all = list(&HashSet::new());
        for kind in ["importers", "exporters", "stores", "fillers"] {
            assert!(all.iter().any(|m| m.kind == kind), "{kind} is listed");
        }
        assert!(all.iter().all(|m| m.enabled));
        assert!(all.iter().all(|m| !m.label.is_empty() && !m.hint.is_empty()), "every module lists a label and a hint");
        let mut disabled = HashSet::new();
        disabled.insert("importers:aegis".to_string());
        let filtered = list(&disabled);
        let aegis = filtered.iter().find(|m| m.kind == "importers" && m.id == "aegis").unwrap();
        assert!(!aegis.enabled);
        let csv = filtered.iter().find(|m| m.kind == "importers" && m.id == "csv").unwrap();
        assert!(csv.enabled);
    }
}
