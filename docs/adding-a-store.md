# Adding a store

A *store* is where a synced database lives besides this device: a LAN folder or a NAS, Dropbox,
Google Drive, OneDrive, or a platform document (Android). Each store is an in-tree module — a
[StorePlugin] behind a registry, compiled in. A cloud store can be turned off in the settings screen
(`optional`, the default), except while a database syncs with it; a folder and a platform document
cannot (`optional` false): they are how the app reaches a file at all, with no connection of their own. Nothing is loaded at
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
   `GooglePlugin`, `OneDrivePlugin`, `DocumentPlugin`) each implement `StorePlugin`: `id`, `title`,
   `hint`, `needs_network`, `list`, `create`, `open`, `provider`, `describe`, `name`, and
   `optional` where it cannot be turned off. A new store adds one
   plugin and registers it in `store_plugins()`, in the order the sync setup should offer it. The
   `title` and `hint` are what the settings and the sync setup show — the single source, kept in the
   core, with no copy in the frontend.

3. **Where a synced file lives.** [Location] is the serialized form the state file keeps. A new store
   needs a `Location` variant (and its serde shape) plus a `kind()` id matching the plugin's `id()`;
   the existing variants show the pattern. The state file is read by every released copy, so a variant
   is added, never renamed or reshaped, and old variants keep reading.

4. **The UI.** Windows' sync setup and choose screen list the stores from the registry
   (`modules::list`, kind `stores`): the label and hint are the plugin's; `src/sync-setup.ts`
   (`STORE_CLOUDS`) only maps a store's id to the command that signs in to it. Android's first run and
   Sync tab keep their own table (`STORES` in `apps/android/ui/src/sync.ts`: the name and the app's
   folder in the store, for the phone's longer texts) and leave out a store turned off. Settings ›
   Modules lists the cloud stores from the registry on both apps (`src/modules-settings.ts`). A new
   cloud store is added to `STORE_CLOUDS` and `STORES` too.

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
