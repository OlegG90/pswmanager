# Adding a store

A *store* is where a synced database lives besides this device: a LAN folder or a NAS, Dropbox,
Google Drive, OneDrive, or a platform document (Android). Each store is an in-tree optional module —
a [StorePlugin] behind a registry, compiled in, toggled in the settings screen. Nothing is loaded at
runtime: adding a store is a code change that goes through the normal branch + PR workflow, with the
same tests and the same security review as any other change.

A store is not a plugin API for third parties. The `StorePlugin` trait stays `pub(crate)` in the shared
core (`crates/core`), and the store's network code lives in the core beside the others.

## The pieces

1. **The store's client.** The cloud stores each have a module of their own: `dropbox.rs`,
   `google.rs`, `onedrive.rs`. A new cloud store gets one too. It implements [Remote] (`revision`,
   `download`, `upload`) and the store's own list/create calls, as `dropbox.rs`'s `list_databases` and
   `create` do. Sign-in goes through [crate::oauth] with a [Provider] describing the service.

2. **The [StorePlugin].** In `remote.rs`, the existing plugins (`FolderPlugin`, `DropboxPlugin`,
   `GooglePlugin`, `OneDrivePlugin`, `DocumentPlugin`) each implement `StorePlugin`: `id`,
   `needs_network`, `list`, `create`, `open`, `provider`, `describe`, `name`. A new store adds one
   plugin and registers it in `store_plugins()`, in the order the sync setup should offer it.

3. **Where a synced file lives.** [Location] is the serialized form the state file keeps. A new store
   needs a `Location` variant (and its serde shape) plus a `kind()` id matching the plugin's `id()`;
   the existing variants show the pattern. The state file is read by every released copy, so a variant
   is added, never renamed or reshaped, and old variants keep reading.

4. **The UI.** The sync setup lists the stores from the registry (`modules::list`, kind `stores`), and
   the store's picker label and hint live in `src/sync-setup.ts` (`STORE_LABELS`) keyed by the store's
   id. The sync settings, the first-run sheet and the Android drawer/sheets render from the same
   registry, so a store turned off in Settings disappears everywhere.

5. **Docs and specs.** `docs/spec.md` (Windows and what both share) and `docs/spec-android.md` describe
   the store's behaviour, its sign-in and its error states (`Offline`, `SignIn`, `Changed`), and the
   store is added to the compatibility gate — a database synced through it must survive
   phone ↔ PC ↔ Keepass2Android.

## What the core owns, whatever the store

The sync decision, the merge, the `.bak`/`.remote.bak` copies, the working-copy hash, background
uploads without the key, and where the tokens are kept (Windows Credential Manager / Android Keystore)
are the core's, not the store's. A store only lists, creates and opens files, and says how it is
reached. Secrets stay in the core, wiped on drop; a store's bytes are the encrypted database file,
never an entry's plain value.

## Checklist

- `npm test`, both `tsc` projects and `cargo clippy --all-targets` green.
- The store's full cycle tried on the device: sign in, list, create, open, sync, a conflict (`Changed`),
  offline, signing out / a revoked token (`SignIn`).
- The compatibility gate passes through the new store.
- Specs updated in the same branch, and the security review in the PR answered: what the new code can
  read, where cleartext lives and when it is wiped, what leaves the device.
