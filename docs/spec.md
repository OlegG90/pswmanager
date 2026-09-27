# PswManager — MVP Specification

A personal, minimal password manager for Windows 11 that lives in the tray. The data is a standard
KeePass database (KDBX 4), so the same file stays usable in the rest of the KeePass ecosystem. The guiding
principle: **fast access to one's own passwords**. Anything that is not finding, copying or editing an entry
stays out.

## Product

| | |
|---|---|
| Display name | PswManager (working name) |
| Executable / CLI | `pswm.exe` |
| Repository / data folder | `pswmanager` |
| Platform | Windows 11 (WebView2 is part of the OS) |
| UI language | English only |
| Distribution | A single portable `pswm.exe`: no installer, not code-signed, no auto-update |
| License | MIT |

### Devices

| Device | Client |
|---|---|
| Windows PCs | PswManager |
| Android phones | Keepass2Android (opens the same file in the same store with its own client, merges on conflict) |
| iPad | **Out of scope for now** |

### Out of scope

Browser autofill and browser extensions, an own sync server, a database shared between people (each person
syncs their own), several people editing one file at the same moment, iPad, creating a new database
(the database comes from `sic2kdbx` or KeePassXC), several open databases at once, importing from other
password managers inside the app (SafeInCloud migration stays with `sic2kdbx.py`), saving changes made to
an opened attachment, Windows Hello unlock, password-health reports, sharing, KDBX 3 writing.

## Database

- One KDBX 4 file, chosen once and remembered: a local file, or a file in a remote store the app syncs
  with (see *Synchronisation with a remote store*). Unlocked with a master password, a key file, or both.
- The file is read with the [`keepass`](https://crates.io/crates/keepass) crate and written as KDBX 4.1
  (`save_kdbx4` feature; 4.1 is the only version it writes, so a 4.0 file becomes 4.1 on its first save —
  KeePassXC 2.7+, KeePass 2.48+ and Keepass2Android read it). The cipher and key derivation are kept.
- A file whose elements are out of KeePass's order (as older `sic2kdbx` versions wrote) is refused with a
  message, because keepass-rs would decrypt its protected values in the wrong order.
- **Nothing is lost on a round trip.** Data the app does not show — attachments, entry history, custom
  attributes, `CustomData` (including the `SafeInCloud` JSON written by `sic2kdbx`), icons, the recycle
  bin, `DeletedObjects` — is written back unchanged.
- The key and every decrypted value stay in the Rust backend. The frontend receives only what it has to
  show; a password reaches the frontend only while it is revealed or being edited. Secrets in Rust are held
  in `zeroize`-on-drop types.

## Window

- Search-first: the search field has focus when the window opens. Search covers title, user name, URL,
  tags and notes (not passwords). Results update as you type.
- A list of entries on the left, the selected entry on the right. A group / tag filter narrows the list.
- The entry view shows title, user name, password (masked, `Ctrl+H` or click to reveal), URL, TOTP code,
  notes, additional attributes and attached files (name and size). Protected attributes are masked like the
  password. **Open** opens a file in the app Windows uses for its type, **Save…** writes it where the user
  chooses, **Replace…** gives it the content of another file, **Rename** renames it, **Remove**
  takes it off the entry (after a confirmation); **Attach file…** adds one (see *Editing*). A file's content goes between the database and the disk in the backend, never through the
  webview.
- Every entry in the list and the entry view shows an icon (see *Entry icons*).
- Standard Windows frame. Closing the window hides it to the tray; the app keeps running.

### Entry icons

- An entry whose URL is an `http(s)` address and whose site has its own icon shows that icon; every
  other entry shows the default icon.
- Order: a custom icon already stored on the entry in the KDBX file → the site's icon → the default icon.
- The site's icon is downloaded **directly from that site** (the `<link rel="icon">` of the start page,
  then `/favicon.ico`), never through a third-party favicon service, so no one else learns the list of
  sites. Only the host goes over the network — never the path, query, user name or anything from the entry.
- Downloads run in the background after unlock and never delay the window; a site that fails is retried
  at most once a week.
- Icons are cached in the data folder (`icons/`, keyed by host), not written into the database, so fetching
  icons never changes the file and never causes a sync. The cache holds no secrets, only host names.
- A setting turns downloading off (on by default); with it off, cached icons are still used.
- Window size and position are remembered.

### Keyboard shortcuts

| Key | Action |
|---|---|
| global hotkey (default `Ctrl+Alt+P`, configurable) | Show / hide the window |
| type anywhere | Search |
| `↑` / `↓` | Move in the list (the selected entry shows on the right) |
| `Ctrl+B` | Copy user name |
| `Ctrl+C` | Copy password |
| `Ctrl+T` | Copy TOTP code |
| `Ctrl+U` | Open URL in the default browser |
| `Ctrl+H` | Reveal / hide password |
| `Ctrl+N` / `Ctrl+E` / `Del` | New / edit / delete entry |
| `Ctrl+L` | Lock |
| `Ctrl+,` | Settings |
| `Esc` | Clear search, close panel, then hide to tray |

## Tray

- The app lives in the notification area. Left click shows / hides the window.
- Menu: Show, Lock, Sync now (with a remote store), Start with Windows (a check mark), Settings, Quit.
- Optional start with Windows (off by default); it starts hidden in the tray, locked. The entry it adds
  under `HKCU\…\Run` launches `pswm --autostart`.
- The global hotkey shows the window, or hides it when it is already in front. A hotkey another app has
  taken is reported on the unlock screen.
- Single instance: launching `pswm` again brings the running window forward.

## Security behaviour

| Setting | Default |
|---|---|
| Lock after inactivity | 5 min (1–60, or never) |
| Lock when Windows locks / the session is switched | on |
| Lock when hidden to tray | off |
| Clear clipboard after copying | 20 s (5–120) |

- Locking drops the key and all decrypted data in the backend; the frontend clears its view.
- The clipboard is cleared only if it still holds the copied value. Copied secrets are excluded from Windows
  clipboard history and cloud clipboard.
- Nothing secret is written to logs, the state file or the console.
- An attachment opened in another app is the one secret written to disk by the app itself, because that
  app needs a file: a read-only copy in a folder of its own (`%TEMP%\pswm-open\<random>\<name>`), which
  is deleted when the database locks, when the app quits and when it starts (after a crash). A copy the other
  app still holds open cannot be deleted on Windows; the next clean-up tries again. Changes made to the copy
  are not saved back.

## Editing

- Create, edit and delete entries. Delete asks for confirmation in the entry view, then moves the entry to
  the recycle bin (as KeePass does; created if missing). A database with the bin turned off is refused —
  removing an entry with keepass-rs can leave other entries' attachments pointing at the wrong data.
- Values the editor only reformats (line breaks an input cannot hold, spaces around a URL, tag and group
  spelling, a TOTP value the app cannot read) are saved as they were, so an untouched entry saves
  unchanged. An entry whose group path did not change stays in its own group, even when group names repeat.
- Editable: title, user name, password, URL, notes, tags, group, TOTP secret, additional attributes
  (add / rename / remove, protected or not).
- **Attachments:** in the entry view a file can be added to an entry, replaced, renamed or removed (up to
  20 MB — the whole database is synced on every change); the change is saved at once, with the previous
  version in history, like an edit. A name the entry already uses gets a number (`scan (2).pdf`) rather
  than replacing the file. A removed file stays in the database for the history version that has it:
  keepass-rs would drop it from the database's file pool, and the gap misnumbers every later file when the
  database is saved, so the file is removed on a copy of the database and only the entry is taken from it.
  keepass-rs cannot rename a file or change its content either: renaming and replacing let the entry go of
  the file the same way and attach the content under the name again (history keeps the old name and
  content). Replacing a file with the same content changes nothing; renaming to a name another file of the
  entry has is refused.
- Every edit pushes the previous version into the entry's history.
- **Password generator** in the editor: length (default 20, 8–64), upper / lower / digits / symbols,
  exclude look-alike characters. Generated with the OS CSPRNG.
- **Strength indicator** (zxcvbn) next to the password field.
- **TOTP**: codes are computed from the entry's `otp` attribute (`otpauth://` URI, as `sic2kdbx` and
  KeePassXC write it), with a countdown.

## Saving and synchronisation

The app always works on a file on this PC. It is either:

- **a local file** the user picked — on this PC, a USB drive, or a folder another program syncs. The app
  keeps that file consistent when something else changes it (below), but it cannot tell when another
  program delivers the file to other devices; or
- **the working copy of a remote file** — kept in the data folder and synced with a LAN folder, Dropbox,
  OneDrive or Google Drive by the app itself (see *Synchronisation with a remote store*). This is the way to
  share the database with a phone.

Saving, change detection and the per-change merge below apply to both.

- **Save:** write to a temporary file in the same folder, then rename it over the original atomically, so
  no reader ever picks up a half-written file. The new file is opened again with the key before it
  replaces the old one. Before each save the previous file is kept as `<name>.kdbx.bak` next to it. Saving
  an entry that did not change does not write the file.
- **Change detection:** the app remembers the file's hash when it loads or saves it. Every save, and every
  time the window is shown from the tray, checks the file on disk first. The file's folder is also watched
  (`notify`; sync clients replace a file by renaming a new one over it), so a change that arrives while the
  app is open is read at once: the list updates in place, and an entry being viewed is shown again.
- **Merge:** the app saves every change as soon as it is made, so it never holds unsaved edits beyond the
  one being saved. Instead of merging two databases, it **makes each change on the file as it is now**:
  - the file is read again if it changed (and again if it changes while saving — up to three tries);
  - the change is applied on top: other devices' changes to other entries are kept as they are;
  - the editor sends the entry as it opened it along with the edit, and only the fields the edit changed
    are applied — what another device changed in the same entry meanwhile (another field, its group, a
    renamed group) stays;
  - where both changed the same field, this change is the newer one and wins; the other version goes into
    the entry's history, so nothing is lost, and the window says which fields;
  - an entry deleted elsewhere (recycle bin or `DeletedObjects`) and edited here comes back with its id —
    the edit is newer than the deletion; deleting an entry already gone elsewhere does nothing;
  - nothing is ever removed from the file (a removed attachment stays for the entry's history), so
    keepass-rs's attachment renumbering on removal never applies.
- **An older file coming back** (a sync client restores a stale copy, or a conflict copy wins): when the
  file is read again, entries changed here later than in the file (`LastModificationTime`), newer moves,
  and entries missing from it without a deletion recorded at or after this device's change are kept and
  written back — the same rule as KeePass's merge: the newer version wins, the other goes to history.
  The newer version's attached files go with it: where this device's version wins, files it has are added
  and files it does not have leave the entry (the other version, with its files, goes to history). Where the
  other version wins, its files stand (keepass-rs cannot give an older version in history its own files).
- Reading the file again (deriving the key) happens without holding the database, so the window, tray and
  hotkey stay responsive while a sync client delivers a change.
- An entry open in the editor is never replaced silently: if another device changed it, the editor says so,
  and saving keeps this version with the other one in history.
- A file that cannot be read (wrong key, corrupt, mid-sync) is never overwritten; the app shows the error
  and keeps the in-memory data; saving works again once the file can be read (the next change is read
  when the sync client finishes).
- Saving checks that the new file reads back with the same content before it replaces the old one. (Old
  versions in history are compared by what the file keeps of them: keepass-rs also remembers in memory
  which group each was made in, which the file does not store.)

### Synchronisation with a remote store

One person uses the database, from a few devices, and rarely edits on two of them at the same moment. So
the app does not keep the remote file live: it keeps a working copy, and at defined moments **compares** it
with the remote file and decides what to do. The window always shows what the last sync did.

**Stores:** a LAN folder (`\\server\share\…`, e.g. a NAS), Dropbox, OneDrive, Google Drive. One remote file
per app; each person uses their own account.

**What the app remembers** (in the state file): where the remote file is, the remote revision it last
synced with (Dropbox `rev`, OneDrive `eTag`, Google Drive `version`, a LAN file's hash), and whether the
working copy has changed since that sync.

**The decision:**

| Remote file since the last sync | Working copy since the last sync | What happens |
|---|---|---|
| unchanged | unchanged | nothing |
| unchanged | changed | upload: the working copy replaces the remote file |
| changed | unchanged | download: the remote file replaces the working copy |
| changed | changed | merge the two, save the result as the working copy, upload it |

- **Merge** follows KeePass's rules: entries and groups are matched by UUID; for each, the version with the
  newer `LastModificationTime` wins and the other goes into its history (histories are joined); moves
  follow `LocationChanged`; an entry deleted on one side (`DeletedObjects`, or moved to the recycle bin)
  stays deleted unless the other side changed it later; new entries from both sides are kept. The window
  lists the entries that were merged. The remote file as it was before is kept as `<name>.remote.bak` in the
  data folder (the clouds also keep their own version history).
- **Upload is conditional:** it succeeds only if the remote file is still at the revision the decision was
  made on (Dropbox `update` mode with `rev`, OneDrive `If-Match`; Google Drive: the revision is checked
  right before the upload; a LAN file: its hash is checked before the rename). Otherwise the sync starts
  again from the decision, so another device's upload is never overwritten.
- A remote file that cannot be read (wrong key, corrupt, half-uploaded) never replaces the working copy
  and is never overwritten; the window says so and the app keeps working on the working copy.
- Only files verified the same way as a local save are uploaded; a download is opened with the key before
  it replaces the working copy.

**When it syncs:**

| Moment | Sync |
|---|---|
| Unlock (including after start) | yes; the working copy shows at once, the list updates when the sync finishes |
| An entry is created, edited, moved or deleted | upload about 10 s after the last such change, so several edits go up as one |
| Only an entry's tags changed | waits for the next moment in this table |
| The window is hidden to the tray | yes |
| Lock (by hand, after inactivity, Windows lock) | an upload only: the key is dropped at once, so a merge waits for the next unlock |
| Quit | an upload only, waiting at most 10 s; what did not go up is synced at the next start |
| **Sync now** (a button in the window, an item in the tray menu) | yes |
| While unlocked | the remote revision is checked every 5 min (1–60, or off); the file is downloaded only when it changed |

- **Offline:** everything works on the working copy; the status says changes are waiting, and the next
  sync moment sends them.
- **Status** in the window and the tray tooltip: *Synced 12:04*, *Syncing…*, *Changes waiting — offline*,
  *Merged 3 entries from Dropbox*, *Sign in to Dropbox again*, or the error. A sync error never blocks
  reading or editing.

**Connecting a cloud account:**

- OAuth 2 with PKCE in the system browser and a loopback redirect (`http://localhost:53134/<store>`). The
  app has no client secret; its public app ids are in the source.
- The refresh token is kept in the Windows Credential Manager (protected for the Windows user), never in
  the state file or logs. **Disconnect** removes it; the working copy stays.
- Each store gets the narrowest access that still reaches a file Keepass2Android can open: Dropbox — its
  app folder (`Apps/PswManager Sync`); OneDrive and Google Drive — settled with their steps of milestone 6.
- Setting up: sign in, then either pick the `.kdbx` in the store or upload the current local database to it
  (**Sync with Dropbox…** on the unlock screen; an upload never replaces a file already there).
  A LAN folder needs no account: **Sync with a folder…** on the unlock screen picks the file (**Open a local file…**
  opens one without syncing).
- **Stop syncing** (on the unlock screen) makes the database a local file again (for a LAN folder, the file
  in it; for a cloud store, the working copy) and signs the account out. Choosing another database or stopping is refused while the working copy has changes the remote
  file lacks — they would be left behind; unlocking syncs them first.
- Only the stores' own APIs are contacted.

**Compatibility gate:** a file saved by PswManager must open in KeePassXC and in Keepass2Android, and a
change made in Keepass2Android must merge back without loss — for a local file and through each supported
store. This is checked before every release.

## Command line

```
pswm [<file.kdbx>] [--data-dir <path>]
pswm --help | --version
```

| Option | Meaning |
|---|---|
| `<file.kdbx>` | Open this database instead of the remembered one |
| `--data-dir <path>` | Use an explicit data location |
| `--help` / `--version` | Print to the console the exe was launched from |

## State

- One JSON file: settings, the database path or remote store, the key file path, the sync state, window
  geometry. **Never** the master password, a cloud token or any secret. Site icons are cached in `icons/`
  beside it; the working copy of a remote file and its backups are in `sync/`.
- **Location:** `pswm.json` next to the exe if that folder is writable; otherwise
  `%APPDATA%\pswmanager\pswm.json`. `--data-dir` overrides both.

## Technology

- **Shell:** Tauri 2 (Rust) + WebView2, tray via Tauri's `tray-icon` feature.
- **Frontend:** plain TypeScript, no framework.
- **Crates:** `keepass` (`save_kdbx4`), `zeroize`, `notify`, `zxcvbn`, `totp-lite`; `ureq` (native TLS)
  for site icons and the cloud APIs; the Windows Credential Manager for tokens.
- **Plugins:** single-instance, global-shortcut, autostart, opener, dialog. Clipboard handling is done in
  Rust, so it can set the history-exclusion formats and clear only its own value.
- **Build:** Tauri bundling (MSI/NSIS) disabled; only `pswm.exe` is produced. `npm run build` builds it for
  the local architecture; `npm run build:x64` / `build:arm64` for a given one.
- **CI:** GitHub Actions builds `pswm.exe` for **ARM64 and x64** on `v*` tags (the tag must match the
  version in `src-tauri/Cargo.toml`) and attaches both, as `pswm-x64.exe` and `pswm-arm64.exe`, to the
  GitHub Release. A manual run builds both as an artifact without publishing.
- The exe is unsigned, so SmartScreen asks for confirmation on first run.

## SafeInCloud migration

`sic2kdbx.py` stays a separate, one-off Python tool (see README). It is not part of the app and not part of
the release.

## Testing

- Automated (Rust): KDBX round trip preserves everything listed under *Database*; merge cases (edit / edit,
  edit / delete, move, new on both sides, history union); atomic save and `.bak`; change detection;
  the sync decision, conditional upload and retry against a fake store; merging two databases;
  generator character sets; TOTP against RFC 6238 vectors; icon choice order and `<link rel="icon">`
  parsing; data-location selection; CLI parsing.
- Automated (TypeScript): search / filter, keyboard handling.
- Test databases are generated by the tests or kept in `tests/`; real exports and databases never go to git.
- By hand, before a release: the compatibility gate above, tray / hotkey / auto-lock / clipboard behaviour.

## Milestones

1. Scaffold, unlock a KDBX file, list / search / view entries, copy with clipboard clearing, entry icons
2. Tray, global hotkey, single instance, auto-lock, start with Windows
3. Editing, generator, strength indicator, TOTP; atomic save with `.bak`
4. Change detection, file watching, merge; compatibility gate with KeePassXC and Keepass2Android
5. Settings panel, CLI flags, portable exe build and release CI
6. Synchronisation with a remote store: the sync mechanism with a LAN folder and Dropbox, then OneDrive,
   then Google Drive (one PR each); can land before 5
