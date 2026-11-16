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
use crate::vault::{EntryChoice, LoginError, LoginPlace, Vault};
use keepass::db::EntryRef;
use serde_json::Value;
use zeroize::Zeroizing;

/// An import format (CSV, Bitwarden's JSON, CXF, Aegis): it decides whether a
/// file is its format, and reads it into a [crate::cxf::Import]. The core owns
/// what happens next (adding the entries, tagging, wiping, history); an
/// importer owns only `id/label/extensions/detect/parse` and its `skipped`
/// reasons.
pub(crate) trait Importer: Send + Sync {
    /// Phase 1 lists these in the UI; nothing reads them in Phase 0.
    #[allow(dead_code)]
    fn id(&self) -> &'static str;
    #[allow(dead_code)]
    fn label(&self) -> &'static str;
    /// The file name endings this format is known by, a hint for the picker.
    #[allow(dead_code)]
    fn extensions(&self) -> &'static [&'static str];

    /// Whether this is the importer for the file: for the CSV importer, `json`
    /// is `None` and `head` (the file's first bytes) decides — it takes any
    /// file the JSON importers do not; for a JSON importer, `json` is the file
    /// parsed (already, to be wiped by the caller) and its keys decide.
    fn detect(&self, name: &str, head: &[u8], json: Option<&Value>) -> bool;

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
    /// Phase 1 lists these in the UI; nothing reads them in Phase 0.
    #[allow(dead_code)]
    fn id(&self) -> &'static str;
    #[allow(dead_code)]
    fn label(&self) -> &'static str;
    fn extension(&self) -> &'static str;

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
    /// Whether talking to the store needs the network (clouds do). Phase 2
    /// reads this; nothing does in Phase 0.
    #[allow(dead_code)]
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

/// An autofill category (logins now, cards as the pilot): it owns what a
/// category offers, fills, saves and links, per the entry and the place
/// asking. The core's router owns the [crate::autofill] policy — the `Asker`
/// check on every offer and answer, locked answers, limits, confirmations and
/// wiping — and hands each category only its own database work, after that
/// check.
pub(crate) trait CategoryFiller: Send + Sync {
    /// The category's id: `"login"` now, `"card"` and more later. Phase 1
    /// lists it; nothing reads it in Phase 0.
    #[allow(dead_code)]
    fn id(&self) -> &'static str;

    /// The entries to offer for a login from `place`.
    fn offers(&self, vault: &Vault, place: &LoginPlace) -> Vec<EntryChoice>;
    /// The entries a login from `place` can update (save-back choices).
    fn choices(&self, vault: &Vault, place: &LoginPlace) -> Vec<EntryChoice>;
    /// The entries in use matching `query`, to pick one to link.
    fn candidates(&self, vault: &Vault, query: &str) -> Vec<EntryChoice>;
    /// Entry `id`'s login for `place`.
    fn fills(&self, vault: &Vault, id: &str, place: &LoginPlace) -> Result<(Zeroizing<String>, Zeroizing<String>), LoginError>;
    /// Keeps a login: into entry `id` (which must be for `place`), or a new
    /// entry; answered with the entry's id. (Eight arguments: it mirrors
    /// [Vault::save_login], plus the vault it works on.)
    #[allow(clippy::too_many_arguments)]
    fn saves(&self, vault: &mut Vault, id: Option<&str>, place: &LoginPlace, confirmed: bool, title: &str, username: &str, password: &str) -> Result<String, LoginError>;
    /// Links entry `id` to `place`.
    fn links(&self, vault: &mut Vault, id: &str, place: &LoginPlace, confirmed: bool) -> Result<(), LoginError>;
}

#[cfg(test)]
mod tests {
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
            let _ = plugin.needs_network();
        }
    }

    #[test]
    fn category_fillers_start_with_logins() {
        let ids: Vec<&str> = crate::autofill::category_fillers().iter().map(|filler| filler.id()).collect();
        assert_eq!(ids, ["login"]);
    }
}
