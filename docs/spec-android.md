# PswManager for Android — Specification

PswManager on an Android phone: the same KeePass database (KDBX 4) as on Windows, the same groups, tags,
merge and sync rules, in a phone's shape. The guiding principle stays **fast access to one's own
passwords**.

This document says what is **different or new on Android**. Everything it does not mention follows
[`spec.md`](spec.md): the file format and what survives a round trip (*Databases*), the merge rules
(*Saving and synchronisation*, *Merge*), the sync decision and conditional upload (*Synchronisation with a
remote store*), groups and tags, entry history, entry icons, TOTP and the generator. Those rules live in one
Rust crate shared by both apps (see *Technology*), so they cannot drift apart.

The look is in [`design/android/`](design/android/) (mockups from Claude Design). This spec describes
behaviour, the mockups describe the look; where they disagree, this spec wins. The mockups also show
things that come in later stages (the *Database* and *Backup* settings tabs, Templates and Trash, a
database switch); see *Stages*.

## Product

| | |
|---|---|
| Display name | PswManager |
| Application id | `io.github.olegg90.pswmanager` |
| Platform | Android 10 (API 29) or newer; phones, portrait only |
| Reference device | Samsung Galaxy S24 Ultra |
| UI language | English only |
| Distribution | A signed APK attached to the GitHub Release; installed by hand. No Google Play, no F-Droid, no auto-update |
| License | MIT |

### Role

At first PswManager **complements** Keepass2Android on the phone: quick search, viewing and copying, and
(from stage A2) editing. Keepass2Android stays for what PswManager does not do yet, autofill above all.
Both open the same file in the same store and merge each other's changes. Replacing Keepass2Android
completely is a goal for later stages.

### Out of scope for now

Autofill (Android's `AutofillService`; a later stage of its own), several databases in the list (one
database on the phone), creating a new database, *Database settings* (name, history limits, key and
encryption changes — they are done on Windows or in Keepass2Android), backup, the Templates and Trash
groups, managing tags (rename, merge, remove), password health, restoring a history version, a LAN folder as a store, tablets and landscape, Google Play.

## The database on the phone

One database at a time. It is either **synced** with a cloud store (Dropbox in stage A1) or a **local
file** without a cloud.

Android's file access gives documents, not paths, and a document cannot be renamed over. So in both cases
the core works, as on Windows, on a **working copy** in the app's private storage (with its `.bak` and
`.remote.bak` beside it), and the rules of *Saving and synchronisation* in `spec.md` apply to it unchanged.

### A synced database and its visible copy

The working copy syncs with the remote file in the store, as on Windows. Besides it, the database is kept
as a **visible file** the user can see and back up:

- It lives in a folder the user picks **once** with Android's folder picker (Storage Access Framework);
  the picker starts in `Documents`. The app keeps the permission for that folder (a persisted URI
  permission), so it is never asked again — after a reinstall the folder is picked again. No broad storage
  permission (`MANAGE_EXTERNAL_STORAGE`) is asked for.
- After every save and every sync that changed the working copy, the working copy is written there (in
  place, read back). Offline changes reach it at once, so it always holds the latest state, also what has
  not reached the store yet.
- It is a **copy for the user**, never read back: Keepass2Android and the PC use the store itself. If
  something else changed it since the app last wrote it, the app does not overwrite it and says so.
- A file of the same name already in the folder at first run is kept as `<name>.kdbx.bak` before it is
  replaced, and the screen says so first.

### A local file

**Open a local file** picks an existing `.kdbx` anywhere Android's file picker reaches (the phone, an SD
card, a folder another program syncs), with a persisted permission for that one file. That document is the
database's **remote file**, synced with the working copy by the same rules as a cloud store or a LAN folder
on Windows (*Synchronisation with a remote store* in `spec.md`):

- Its revision is the hash of its content (Android's modification times and sizes cannot always tell an
  edit), so a change made elsewhere — Keepass2Android, Syncthing, another app — is seen at the next sync.
- It syncs at the moments in *Synchronisation* below: unlock, coming to the front, after a change, going
  to the background, and pull-down. No network is needed, so it is always "online".
- An upload writes the document in place (mode `wt`) and reads it back, once more when it did not read
  back as written; it never writes over a change it has not merged (the revision is checked first).
- `.remote.bak` (the document as it was before a merge) is kept in the app's private storage: with access
  to one file only, the app cannot write beside it.

## First run

The screens in the mockups `1a`–`1d`:

1. **Choose your database:** *Sync with Dropbox* or *Open a local file*.
2. **Dropbox:** sign in (see *Connecting a cloud account*), then the `.kdbx` files in the app folder
   `Apps/PswManager Sync` are listed with size and date; pick one. PswManager sees only its app folder, so a
   file to be shared with Keepass2Android and PswManager for Windows must be there (Keepass2Android, with
   full Dropbox access, opens it there).
3. **Where to keep it on this phone:** the folder for the visible copy (see above) and the file name; the
   file is downloaded, and sync is on from the start.
4. **Unlock** with the master password and/or the key file (picked with the file picker, permission kept).
5. After the first unlock the app offers **biometric unlock** (from stage A3).

A local file skips steps 2–3.

## Unlock

- **Master password and/or key file**, as on Windows. The unlock screen shows the database's name and
  description (as last unlocked), whether it syncs, and where its file is.
- **Biometric unlock** (stage A3): after a successful unlock with the master password, the database's key
  (the master password and the key file's content) is encrypted with a key in the **Android Keystore**
  that needs a strong biometric (`BIOMETRIC_STRONG`) to use and is invalidated when a new fingerprint or
  face is enrolled. The next unlocks ask for the fingerprint or face.
- The **master password is asked again every 14 days** (a setting: 1–90 days), after the Keystore key is
  invalidated, and when the database's key changed on another device (the stored key no longer opens it).
  Then the stored key is replaced.
- Turning biometric unlock off deletes the stored key.
- A key change made on another device is handled as in *Database settings → With sync* in `spec.md`: the
  app asks once for the other file's key to merge it.

## Screens

As in the mockups `1e`–`1j`.

- **List**, laid out as in Keepass2Android: a toolbar on top (☰ for the drawer, the group or tag shown,
  *Sync now*, *Settings*, *Lock*), the entries with their icons, and floating buttons at the
  bottom right: **search** (the search field takes the toolbar's place; Back or ← closes it and clears it;
  search as on Windows: title, user name, URL, tags, notes, within the chosen group or tag) and, from
  stage A2, **+** under it for a new entry. The sync status line stays at the bottom; only the entries scroll. Pull
  down to sync.
- **Drawer** (☰ or a swipe from the left edge): the database's name and sync, the groups *All*,
  *Favorites*, *Expired*, *2FA*, *Passkey* (as defined in `spec.md`), then the tags with their counts;
  *Settings* and *Lock* at the bottom.
- **Entry view:** title, user name, password (masked), TOTP with its countdown, URL, notes, additional
  attributes (protected ones masked), attachments, when the entry was last changed, and **History (N)**
  (stage A2). As in Keepass2Android, each line ends in **⋮**, a menu of its commands (*Copy*, *Show / hide*
  for a secret, *Open in the browser* for the URL); tapping a value copies it too. A snackbar confirms the copy and says when the clipboard clears.
  From stage A2 the toolbar has the **star** (filled for a favorite; it toggles it), the **pencil** (the
  editor) and **⋮** with *Delete*.
- **Long press** on an entry in the list: a menu with *Copy user name*, *Copy password*, *Copy TOTP code*.
- **Attachments:** listed with name and size; tapping one, or **Open** in its ⋮ menu, hands the file to
  another app (Android's chooser), see *Security behaviour*. From stage A2 files are added, renamed and
  removed in the editor, as on Windows (`spec.md` *Editing*); a file to attach is picked with
  Android's file picker and read once, without keeping access to it.
- **Entry icons:** as in `spec.md` (custom icon → standard icon → site icon → key); site icons are
  downloaded the same way and cached in the app's private storage, with the same hashed file names (the
  hashing key kept in the Keystore). The *Download site icons* setting turns it off.
- **Theme:** light or dark, or as the system is set (the default).
- **Editor** (stage A2): as in `spec.md` *Editing*: title, user name, password with the generator and the
  strength indicator, URL, notes, tags as chips, favorite, expiry date, TOTP secret, additional attributes,
  files (saved with the entry). A full screen, each label above its field; the toolbar has ← (cancel; it asks before discarding changes,
  as Back does) and ✓ (save, then the entry is shown). A new entry is blank (not from a template), in the
  top group, with the database's default user name. Delete moves an entry to the recycle bin after a
  confirmation.
- **History** (stage A2): the list of older versions with what changed, and a version shown read only, its
  values copied as in the entry view. No restoring.

### Settings

In tabs, as on Windows (mockup `1j`):

| Tab | Settings |
|---|---|
| General | locking and the clipboard (see *Security behaviour*); biometric unlock and how often the master password is asked (stage A3) |
| Appearance | theme; download site icons |
| Sync | where it syncs and the visible copy's folder; how often to check for changes; **Sync now**. From stage A4, with more stores: **Stop syncing** (the database stays as a local file, e.g. when the store cannot be reached or the account has a problem) and the store's account with **Disconnect**. Until then another database is *Use another database…* on the unlock screen (it signs the store's account out) |
| About | version; database format |

Settings belong to this phone; they are not synced with the PC. Their names, defaults and limits are the
PC's where both have one (`crates/core/src/settings.rs`). The screen opens from the toolbar's ⚙ and the drawer.

## Security behaviour

| Setting | Default |
|---|---|
| Lock when the app goes to the background | after 30 s (at once, 30 s, 1 min, 5 min, never) |
| Lock when the screen turns off | on |
| Lock after inactivity while in front | 5 min (1–60, or never) |
| Clear clipboard after copying | 20 s (5–120) |

- Locking drops the key and every decrypted value in the backend, as on Windows.
- **Clipboard:** a copied secret is marked sensitive (`ClipDescription.EXTRA_IS_SENSITIVE`, Android 13+),
  so the keyboard's clipboard suggestions and the system's copy preview do not show it. It is cleared after
  the set time only if it still holds the copied value, as far as Android lets the app tell (an app in the
  background cannot read the clipboard; it then relies on the change it last saw).
- **Screens are never captured:** `FLAG_SECURE` is always on, so screenshots and screen recording show
  nothing and the recent-apps thumbnail is blank.
- **An opened attachment** is copied to a folder of its own in the app's cache (`cache/open/<random>/`)
  and handed to the other app through a content URI (`FileProvider`) with read permission only. The folder
  is deleted when the database locks and when the app starts. APKs and scripts are not opened.
- The app's private storage is **excluded from Android's backup** (Auto Backup and device-to-device
  transfer): it holds the encrypted tokens and the icon cache, which another device could not decrypt
  anyway.
- Nothing secret is written to logs (`logcat`), the state file or crash reports.

## Synchronisation

The sync decision, the merge and the conditional upload are those of `spec.md`, run by the same code.
What differs is **when**: Android stops apps in the background, so there is no polling from the
background.

| Moment | Sync |
|---|---|
| Unlock (including after start) | yes; the working copy shows at once, the list updates when the sync finishes |
| The app comes back to the front, unlocked, more than 1 min after the last sync | yes |
| An entry is created, edited or deleted (stage A2) | upload about 10 s after the last such change (tags and the star alone wait for the next sync, as on Windows) |
| The app goes to the background with changes not uploaded | an upload at once; if Android stops it, a WorkManager job (needs a network) uploads the working copy later |
| Lock (by hand or on its own) | an upload only, as on Windows: a merge waits for the next unlock |
| Pull down on the list, or **Sync now** | yes |
| While the app is in front and unlocked | the remote revision is checked every 5 min (1–60, or off), as on Windows |

- The background upload never needs the database's key: it uploads the encrypted working copy, conditional
  on the remote revision. If the remote file changed meanwhile, the upload stops and the merge waits for the
  next unlock.
- **Offline:** everything works on the working copy; the status says changes are waiting.
- **Status line** (top of the list) with the same texts as on Windows: *Synced 12:04*, *Syncing…*,
  *Changes waiting — offline*, *Merged 3 entries from Dropbox*, *Sign in to Dropbox again*, or the error.
  Tapping it opens the **Sync** sheet: the last result, the remote file, the working copy, the entries the
  last merge touched, and *Sync now*. A sync error never blocks reading or editing.

### Connecting a cloud account

- OAuth 2 with PKCE, in the default browser, with no client secret. A loopback redirect does
  not work on a phone, so each store needs an Android redirect registered for the app:
  - **Dropbox** (stage A1): **open question**, decided as the first step of A1 — either a custom-scheme
    redirect (`io.github.olegg90.pswmanager://dropbox`) registered in the Dropbox app console beside the
    localhost one, or Dropbox's Android SDK used **for sign-in only** (its `db-<app key>` scheme and the
    hand-off to the Dropbox app), handing the refresh token to the Rust code, which does everything else as
    on Windows. If neither works, the code Dropbox shows is pasted into the app by hand.
  - **OneDrive** (stage A4): an Android platform in the existing Entra registration (the package name and
    the signing key's hash; the redirect `msauth://io.github.olegg90.pswmanager/<hash>`).
  - **Google Drive** (stage A4): an Android OAuth client for the package name and the signing key's SHA-1.
- The same narrow access as on Windows (Dropbox's app folder, OneDrive's app folder, Google's `drive.file`).
- The refresh token is kept in the app's private storage, **encrypted with a key in the Android Keystore**
  (not bound to a biometric, so the background upload can use it). **Disconnect** deletes it; the working
  copy stays.

## State

- One JSON file in the app's private storage: settings, the database (its file's URI, key file URI, sync —
  the remote file and the sync state — and its name and description as last unlocked). Never the master
  password, a token or any secret.
- The encrypted tokens, the encrypted key for biometric unlock and the icon cache are beside it, in the
  app's private storage.

## Technology

- **Shell:** Tauri 2 mobile (Android), the same plain-TypeScript frontend approach as on Windows, with
  layouts of its own for the phone.
- **Shared core:** the KDBX handling, merge, sync decision, stores, TOTP, generator and icon fetching move
  from `src-tauri` into a crate `crates/core` that both apps use (stage A0), with no change in behaviour on
  Windows. Windows-only parts (tray, hotkey, Credential Manager, file watching, autostart) stay in the
  Windows app.
- **Kotlin plugins** for what only Android offers: file and folder access (SAF), the Keystore and
  biometric prompt, the sensitive clipboard, `FLAG_SECURE`, the app's lifecycle (front, background, screen
  off), WorkManager, opening attachments; later the autofill service.
- **Build:** only in CI. The Android SDK, NDK and JDK have no Windows-on-ARM64 builds, so the APK is built
  on GitHub Actions (Ubuntu): every push to a branch with Android changes builds an APK as an artifact,
  a release build signed with the app's key, so it installs over the released app and keeps its data, and
  what is tried is what is released (a run without the key's secret, a fork's or Dependabot's, builds a
  debug APK). A `v*` tag builds the **release APK** the same way and attaches it to the GitHub Release beside
  the Windows exes. Both apps have one version (the root `Cargo.toml`), which the tag must match.
- **Signing key:** made once: a PKCS#12 store (alias `pswmanager`, one password for the store and the key),
  kept in the CI secrets (base64) with its password, and a copy is kept by the owner outside the repo (as an
  attachment in the PswManager database). Any workflow in the repo can use the secrets, so a change to a
  workflow is reviewed before it runs on a branch. Losing it means the
  app cannot be updated in place, and the stores' Android registrations (which use its hash) must be made
  again.
- **On the PC:** only `adb` (platform-tools), to install the APK on the phone and read `logcat`. The phone
  has USB or wireless debugging on. There is no emulator on this PC (no ARM64 emulation); the app is tested
  on the reference phone.

## Testing

- **Automated:** everything the core does is tested on the PC as now (`cargo test`), once it is in
  `crates/core`; the Windows app's tests keep passing after A0 unchanged. The Android-only parts (SAF save,
  lifecycle triggers, Keystore) are thin and tested by hand.
- **By hand on the phone, before a release:** first run with Dropbox and with a local file; unlock;
  search, view, copy and the clipboard clearing; TOTP; lock on background, screen off and inactivity;
  screenshots blocked; opening an attachment and its clean-up; sync at each moment of the table, offline and
  back; biometric unlock and its invalidation (stage A3).
- **Compatibility gate** (as in `spec.md`), with the phone in it: a change made in PswManager on the phone
  reaches PswManager on Windows and Keepass2Android, and theirs reach the phone, without loss — through
  each supported store and for a local file.

### Known issues

- **A FORTIFY abort at exit.** In about a third of the closes with Back or a swipe from Recents,
  `adb logcat -b crash` shows one line,
  `F libc: FORTIFY: pthread_mutex_lock called on a destroyed mutex`,
  yet the process ends on its own with `exited cleanly (0)`, and no tombstone or backtrace is written
  (the exit wins the race). The exit is the Tauri runtime's: when the last window is destroyed, the event
  loop ends unless the app prevents it, and tao calls `std::process::exit` on the thread it runs the app
  on. The faulting thread was `RenderThread` (Android's hardware UI renderer, HWUI) in 3 of 3 traced
  runs, and the mutex has the same address in every run, so it lives in a library zygote preloads; most
  likely `exit` runs that library's C++ static destructors while `RenderThread` still works. The same
  abort happens in builds from before the background upload, and there is no sign of the app's own code
  in it. Nothing shows to the user. Left to Tauri/tao: ending the process with `_exit` in `RunEvent::Exit`
  would hide it, but would skip Tauri's own clean-up, which runs after that event.

## Stages

Each stage is a GitHub issue and lands in one or more PRs.

- **A0 — Shared core:** move the platform-free code into `crates/core`; the Windows app uses it with no
  change in behaviour; its tests run against the crate.
- **A1 — First usable app:** the Tauri Android project and CI (a signed APK per push and on
  tags); the signing key; Dropbox (the sign-in question decided first); a local file through SAF;
  the visible copy's folder; first run; unlock with a master password and/or key file; list, search, drawer
  (groups and tags), entry view, copying with the sensitive clipboard, TOTP, attachments opened, site
  icons; sync with every trigger and the status line and sheet; locking and `FLAG_SECURE`; settings
  (General, Appearance, Sync, About).
- **A2 — Editing:** the editor with the generator and strength indicator, new entry, delete to the recycle
  bin, the favorite star, tags as chips, expiry date, TOTP secret, files changed in the editor; entry
  history read only.
- **A3 — Biometric unlock:** the Keystore key, the biometric prompt, the master password every 14 days.
- **A4 — More stores:** OneDrive, then Google Drive (one PR each), with their Android registrations.
- **Later:** autofill; several databases and creating one; database settings; backup; Templates and Trash;
  managing tags; restoring a history version; password health.
