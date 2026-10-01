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
| Platform | Windows 11 (WebView2 is part of the OS); Android phones — see [`spec-android.md`](spec-android.md) |
| UI language | English only |
| Distribution | A single portable `pswm.exe`: no installer, not code-signed, no auto-update |
| License | MIT |

### Devices

| Device | Client |
|---|---|
| Windows PCs | PswManager |
| Android phones | PswManager for Android ([`spec-android.md`](spec-android.md)), beside Keepass2Android (which opens the same file in the same store with its own client and merges on conflict) until PswManager replaces it |
| iPad | **Out of scope for now** |

### Out of scope

Browser autofill and browser extensions (Android's autofill is a later stage of the Android app), an own sync server, a database shared between people (each person
syncs their own), several people editing one file at the same moment, iPad, several databases unlocked
at once (one is open at a time; switching locks the other), importing from other
password managers inside the app (SafeInCloud migration stays with `sic2kdbx.py`), saving changes made to
an opened attachment, Windows Hello unlock, sharing, KDBX 3 writing.

## Databases

A **database** is a KDBX 4 file on this PC — always a visible file the user can see, back up and open in
KeePassXC. **Syncing** is a separate property of a database: it keeps that file paired with a file
elsewhere (see *Synchronisation with a remote store*). The two are never mixed: choosing a database never
uploads anything, and setting up sync never picks another database.

- **Several databases, one open.** The app remembers a list of databases; the unlock screen shows which
  one it opens and switches between them. Each database has its own key file (if any) and its own sync.
  Only one is unlocked at a time; opening another locks the current one first.
- **Adding a database** (the choose-database screen) — three explicit actions:
  - **Create a new database**: a new, empty KDBX 4 file (AES-256 with Argon2id at 64 MiB, like KeePassXC's
    default, the recycle bin on) where the user chooses (the dialog starts in `Documents\PswManager`),
    protected by a master password (typed twice, with the strength indicator) and/or an existing key file.
    It is never made over a file already there.
  - **Open a local file**: an existing `.kdbx`.
  - **Open from a store** (a LAN folder, Dropbox, Google Drive, OneDrive): sign in, pick an
    existing file there, choose where its local file goes (by default `Documents\PswManager\<name>.kdbx`).
    The file is downloaded there and sync with that remote file is on from the start. A file the user
    chose to replace is kept as `<name>.kdbx.bak`. A synced database whose file went missing is downloaded
    to it again at the next unlock.
- **Removing a database from the list** forgets it (and its sync); the file itself stays where it is.
- Unlocked with a master password, a key file, or both. The unlock screen shows the database's key file with
  *Key file…* and *Remove*; for a database without one, a link *Use a key file…* and a line saying what it is:
  only for a database set up with one, a `.keyx` or `.key` file kept apart from it, new ones made in
  *Settings → Database*.
- The file is read with the [`keepass`](https://crates.io/crates/keepass) crate and written as KDBX 4.1
  (`save_kdbx4` feature; 4.1 is the only version it writes, so a 4.0 file becomes 4.1 on its first save —
  KeePassXC 2.7+, KeePass 2.48+ and Keepass2Android read it). The cipher and key derivation are kept
  unless changed in *Database settings*.
- A file whose elements are out of KeePass's order (as older `sic2kdbx` versions wrote) is refused with a
  message, because keepass-rs would decrypt its protected values in the wrong order.
- **Nothing is lost on a round trip.** Data the app does not show or change — entry history, custom
  attributes, `CustomData` (including the `SafeInCloud` JSON written by `sic2kdbx`), icons, the recycle
  bin, `DeletedObjects`, and attachments the user did not change — is written back unchanged.
- The key and every decrypted value stay in the Rust backend. The frontend receives only what it has to
  show; a password reaches the frontend only while it is revealed or being edited. Secrets in Rust are held
  in `zeroize`-on-drop types.

### Database settings

What belongs to the database itself, not to this PC, is kept **in the file** (KeePass's `Meta` and header),
so KeePassXC and Keepass2Android see the same values and change them too. *Settings → Database*, for the
open database (unlocked: every change is a save), next to *Settings → Sync*.

| Setting | Stored as | Used by the app |
|---|---|---|
| Name | `DatabaseName` | the unlock screen and the list of databases show it (as last unlocked), with the file under it, and the window title while it is unlocked; the file name when empty |
| Description | `DatabaseDescription` | shown under the name on the unlock screen |
| Default user name | `DefaultUserName` | filled in on a new blank entry (not one from a template) |
| History: versions per entry | `HistoryMaxItems` (10 by default, 0–100) | see *Entry history* |
| History: size per entry | `HistoryMaxSize` (6 MiB by default, as KeePass; 1–64 MiB) | the oldest versions go first when an entry's history is larger |
| Master password and key file | the key | see below |
| Encryption | the header: cipher (AES-256 or ChaCha20) and key derivation (Argon2id, Argon2d or AES-KDF with its rounds / memory / parallelism) | see below |

- **Renaming the database** changes its name, not its file. **Rename file…** (on the choose-database screen,
  beside *Remove from the list*, while locked) renames the local file in its folder, and its `.bak` and
  `.remote.bak` with it — all of them or none; refused when the file is missing (it is opened, or downloaded
  again, first), when a file of any of those names is there, when another database in the list has it, or
  when it is the remote file a folder-synced database syncs with. A synced database keeps syncing with the same remote file, whose name stays.
- **History limits:** lowering one trims every entry's history at once (saved and synced like an edit), after
  a confirmation that says how many versions go. A version's size counts its fields, tags and files. Every
  save trims to the limits, so a merge does not bring trimmed versions back, and the files only the trimmed
  versions used leave the file (the keepass-rs fork's `Database::remove_unused_attachments`). A limit another
  client set that the lists do not offer (no limit, say) shows as one more choice.
- **The recycle bin** is not a setting: the app needs it (see *Editing*), and **Empty trash** is in the trash.
- **Changing the master password or key file** (*Change…* in *Settings → Database*, a dialog): the current
  master password is asked first (checked with the key file the database uses now); the new one is typed
  twice, with the strength indicator, and left empty for none. A key file can be added, replaced (an existing
  file, or a new one the app makes — KeePass's XML key file, version 2.0, a `.keyx` which KeePassXC and
  Keepass2Android read — where the user chooses, never over a file already there) or removed, as long as a
  password or a key file remains. The list of databases then uses the new key file. `MasterKeyChanged` is set. The confirmation warns
  that other devices need the new key, and that the old one still opens the `.bak` files and the store's
  version history.
- **Changing the encryption** (*Change…* beside *Encryption* in *Settings → Database*, a dialog): the cipher
  (AES-256 or ChaCha20; a database with Twofish, which KeePassXC offers, keeps it) and the key derivation
  (Argon2id, Argon2d or AES-KDF) with its iterations or rounds, and Argon2's memory and threads; a kind the
  database does not have yet starts from KeePassXC's defaults. Argon2 memory above 256 MiB or an unlock
  slower than about 2 s on this PC (a **Test** button measures it: an empty database is written with these settings,
  which derives the key as an unlock does, and timed; saving measures it too when Test did not) warns that a phone may be slow or run out of
  memory, and asks before saving. Saved and synced like an edit; a sync merge keeps this device's encryption
  unless the other device changed its key later (see *Merge*).
- **With sync**, a change of key syncs first, then the file with the new key goes up at once. While the
  database stays unlocked the app keeps the old key in memory to read a remote file another device changed
  meanwhile; after a lock, a remote file that does not open with the current key is reported, and the app
  asks once for that file's key to merge it. The result's key follows the newer `MasterKeyChanged` (see
  *Merge* below): when this device changed its key later, the result goes up with it; when another device
  did (the password was changed in Keepass2Android, say), the app takes that key — the working copy is saved
  with it, the result goes up with it, and the next unlock asks for it — so a key change is never undone
  silently. A working copy another program replaced with a file that no longer opens asks for its key the
  same way. It is asked in a dialog once per unlock (*Not now* leaves an *Enter key…* button beside the
  sync status); a key given is kept in memory only, and when the database takes it, the list takes its key
  file. A copy on an older key (it came back from before the change) is never taken byte for byte: it is
  merged like an older file coming back, takes this device's key time and the encryption that goes with the
  key, and is written again with this device's key (a remote one kept as `.remote.bak` first).
- **Merge:** each setting is matched by its own time (`DatabaseNameChanged`, `DatabaseDescriptionChanged`,
  `DefaultUserNameChanged`, `SettingsChanged` for the history limits); the newer wins. The key follows
  `MasterKeyChanged` the same way, and the encryption goes with the key; with equal times, or none on the
  other side, the key and the encryption stay this device's. A file on this PC that another program changed on
  the same key is taken as it is now, its encryption too.

## Window

- Search-first: the search field has focus when the window opens. Search covers title, user name, URL,
  tags and notes (not passwords). Results update as you type. Before it, a small keyboard button shows the
  keyboard shortcuts (see *Keyboard shortcuts*).
- Three columns: the **sidebar** (the fixed groups, then the tags), the **list** of entries the sidebar's
  choice shows, and the selected **entry** (see *Groups and tags*). There is no filter menu.
- The entry view shows title, user name, password (masked, `Ctrl+H` or click to reveal), URL, TOTP code,
  notes, additional attributes, attached files (name and size) and when the entry was last changed
  (KeePass's modification time, as a date and time on this PC); an entry with older versions also offers
  **History** (see *Entry history*). Protected attributes are masked like the
  password. **Open** next to a file opens it in the app Windows uses for its type; the **⋯** menu beside it
  has **Save…** (writes it where the user chooses), **Replace…** (gives it the content of another file),
  **Rename…** and **Remove…** (after a confirmation). **Attach file…** adds one (see *Editing*). A file's
  content goes between the database and the disk in the backend, never through the webview.
- Clicking a value in the entry view copies it, as its Copy button does (clipboard clearing included); a
  mouse selection inside a value stays a selection.
- Every entry in the list and the entry view shows an icon (see *Entry icons*).
- Standard Windows frame. Closing the window hides it to the tray; the app keeps running.
- Theme: light or dark, or as Windows is set (the default); a setting.

### Groups and tags

The sidebar has two parts. **Groups** are fixed: an entry is in one because of what it is or what was done
to it, never by picking the group. **Tags** are the user's own way to sort entries; every tag of the
entries in use (not templates, not the trash) is listed under the groups, with its number of entries.

| Group | Shows |
|---|---|
| All | every entry except the deleted ones and the templates |
| Favorites | entries marked with the star (the entry view and the list toggle it); stored as the tag `Favorite`, which `sic2kdbx` also writes for SafeInCloud's star and other clients see as a tag — it is not listed among the tags |
| Expired | entries whose expiry date has passed; those expiring within 14 days are listed too, marked as soon |
| 2FA | entries with a TOTP secret |
| Passkey | entries with a passkey KeePassXC stored (its `KPEX_PASSKEY_*` attributes); shown, not made by the app |
| Templates | the database's templates (KeePass's templates group) |
| Trash | the deleted entries (the recycle bin) |

- The KDBX groups an entry sits in are not shown or edited any more; entries stay where they are in the
  file, and new ones go to its top group (new templates to the templates' group). Search runs within the chosen group or
  tag.
- **Several entries at once:** Ctrl+click and Shift+click select several entries in the list; a bar then
  offers **Add tag**, **Remove tag**, **Favorite** / **Not favorite** and **Delete** for all of them, saved as one
  change (each entry keeps its previous version in history).
- **A tag in the sidebar** can be **renamed** (in every entry; to a name another tag has, the two merge)
  or **removed** from every entry, after a confirmation. The entries themselves stay.
- **Trash:** **Restore** puts an entry back where it was (the group it was deleted from, which KDBX 4.1
  keeps, a template among the templates; the top group when that group is gone, unknown or in the trash
  itself, and for an entry of a group deleted into the trash); **Delete permanently** (after a confirmation)
  removes it, its history and the files only it used, and records the deletion for other devices; **Empty
  trash** does that for everything in the trash, groups deleted into it included. A deletion for good
  stands in a merge and when an older file comes back, unless the other side changed the entry later. Several chosen entries in
  the trash can be restored or deleted permanently at once. (keepass-rs 0.15 misnumbered the other
  entries' files after a removal and kept the files only a removed entry's history used; the app uses a
  fork with the fixes, offered upstream as sseemayer/keepass-rs#374 and #375.)
- **Templates:** **New entry** (`Ctrl+N`) asks for a blank entry or one of the templates. An entry made
  from a template gets its fields (names, values, protection), icon and tags, not its title, expiry, TOTP
  secret (each entry has its own) or star; a
  template's own view also offers **New entry from it**. Templates are edited like entries in the Templates
  group (and deleted into the trash, restored among the templates), and **New template** (the list's button
  and `Ctrl+N` there) makes one there, in KeePass's templates group, made when the database has none.
  They are never listed
  under All, and nothing else treats them as entries (search, health report, favorites).

### Entry history

Every change to an entry keeps its previous version in the entry's history (KeePass's own history, which
KeePassXC and Keepass2Android keep too, so their changes show here as well), up to the database's limits
(`HistoryMaxItems`, 10 when the database does not say, and `HistoryMaxSize`; see *Database settings*).

- **History (N)** in the entry view lists the older versions, newest first: when each was saved and what
  changed from it to the next newer version — the names of the fields (title, user name, password, URL,
  notes, TOTP, additional attributes), tags, icon, expiry and files. Never the values.
- Opening a version shows it like the entry view, read only: protected values stay masked until
  revealed, **Copy** works as in the entry view (clipboard clearing included), and the version's files
  can be opened or saved (its URL is not opened from there, nor its TOTP codes shown: the secret is).
  Under it, **The entry now** lists the fields whose current value differs: an unprotected one with its
  current value, a protected one only saying that it differs.
- **Restore this version** (after a confirmation) makes it the entry's current content (fields, tags, icon,
  expiry, files); the version it replaces goes into the history, as with an edit. It is saved and synced
  like an edit. If another device's change has shifted the history meanwhile, it is refused and the
  history opens again.
  Offered for entries in use and templates, not in the trash.
- Versions are not deleted one by one; the history is trimmed to the database's limit.
- As everywhere, the window gets the dates and the names of what changed; a value leaves the backend
  only when it is shown, revealed or copied.

### Entry icons

- An entry whose URL is an `http(s)` address and whose site has its own icon shows that icon; every
  other entry shows the default icon.
- Order: a custom icon stored on the entry in the KDBX file → a standard icon chosen for it → the site's
  icon → the default icon (the key).
- The editor chooses the icon: **Auto** (the site's icon, else the key), one of the app's own drawings
  (key, web site, account, e-mail, bank, card, identity, phone, computer, server, wireless, terminal, home,
  certificate, note, star — stored as KeePass standard icon numbers, so other clients show their own
  picture for them), or an **image file** (PNG, JPEG, GIF or WebP up to 256 KB — formats Keepass2Android can
  show) kept in the database as the entry's custom icon; an image already there is shared, not stored
  twice. Changing the icon is an edit: the previous version keeps its icon in history.
- The site's icon is downloaded **the way a browser opening the site gets it**: the start page (following
  its redirects, also to another domain), the icons its `<link rel="icon">` declares (wherever the site keeps
  them, e.g. its CDN), then `/favicon.ico`. Only https addresses on named hosts are fetched (no plain http,
  no IP addresses). Never through a third-party favicon service, so no one else learns the list of sites.
  Only the host goes over the network — never the path, query, user name or anything from the entry.
- Certificates are checked against the Windows trust store. Sites behind a bot check (a "Just a moment…"
  page) answer no program but a browser and keep the default icon.
- Downloads run in the background after unlock and never delay the window; a site that fails is retried
  at most once a week.
- Icons are cached in the data folder (`icons/`), not written into the database, so fetching icons never
  changes the file and never causes a sync. A cache file is named by its host hashed with a key
  (HMAC-SHA256), so the folder does not say which sites the databases hold: the key is 32 random bytes kept
  in the Windows Credential Manager for the Windows user, made on first use. Without it the icons are kept in
  memory for the session only. Files earlier versions named by host are renamed, or removed.
- A setting turns downloading off (on by default); with it off, cached icons are still used.
- Window size and position are remembered.

### Keyboard shortcuts

| Key | Action |
|---|---|
| global hotkey (default `Ctrl+Alt+P`; changed in the settings by pressing the new combination, which needs Ctrl, Alt or Win; one another app holds is refused) | Show / hide the window |
| type anywhere | Search |
| `↑` / `↓` | Move in the list (the selected entry shows on the right) |
| `Ctrl+B` | Copy user name |
| `Ctrl+C` | Copy password |
| `Ctrl+T` | Copy TOTP code |
| `Ctrl+U` | Open URL in the default browser |
| `Ctrl+H` | Reveal / hide password |
| `Ctrl+N` / `Ctrl+E` / `Del` | New entry (blank or from a template; a new template in the Templates group) / edit / delete (in the trash: delete permanently) |
| `Ctrl+L` | Lock |
| `Ctrl+,` | Settings |
| `Esc` | Clear search, close panel, then hide to tray |
| `F1` | These shortcuts |

A keyboard button before the search field (and `F1`) shows every shortcut in a dialog, by group (window,
list, entry), with the global hotkey as set now. The Ctrl shortcuts the window reacts to are read from the
same table the dialog shows.

### Settings

*Settings* (`Ctrl+,`, or the button) shows its groups in tabs, one tab at a time: **General** (locking and the
clipboard), **Window** (theme, global hotkey, start with Windows, site icons), **Database** (the open
database's own settings, see *Database settings*; disabled while locked), **Sync**, **Backup** (see
*Backup*) and **About**. ←/→ move
between the tabs, Home and End to the first and last. The tab shown stays while the app runs, through the
redraw after each change (every change is saved at once). Esc closes the settings.

### Backup

*Settings → Backup*, for each database in the list (kept in the list, beside its key file and sync; it works
while the database is locked):
- **Back up every** *Never* (the default), *2 days*, *7 days* or *30 days*, and a **backup folder**
  (*Select…*, *Show* opens it in Explorer). Without a folder there is no backup.
- A backup is the database file as it is on this PC — encrypted, byte for byte; for a synced database, the
  working copy. **One copy** is kept: `base.kdbx` → `base.backup.kdbx` in that folder, replaced each time,
  written beside it first so it is never half-written. A folder another database of the same file name keeps
  its copy in is refused; a new folder gets a copy at once. One backup runs at a time.
- It runs at start and every hour while the app runs (tray included) for each database whose interval has
  passed since its last backup, and at once when a folder or interval is set and one is due. *Backup now*
  makes one whatever the interval. The tab shows the last and the next backup; a failure (folder gone, drive
  offline) is shown there and tried again at the next check.

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
  are not saved back. Programs and scripts (`.exe`, `.bat`, `.ps1`, `.lnk`, `.hta` and the like) are not
  opened: Windows would run them, without the warning a downloaded file gets. They can still be saved.

## Editing

- Create, edit and delete entries. Delete asks for confirmation in the entry view, then moves the entry to
  the recycle bin (as KeePass does; created if missing). A database with the bin turned off is refused:
  entries are removed for good only from the trash.
- Values the editor only reformats (line breaks an input cannot hold, spaces around a URL, tag
  spelling, a TOTP value the app cannot read) are saved as they were, so an untouched entry saves
  unchanged. An edited entry stays in its KDBX group.
- Editable: title, user name, password, URL, notes, tags, favorite, expiry date (KeePass's *Expires*, set to the start of the chosen day on this PC; a time another client set stays while the day is unchanged; none
  by default), TOTP secret, additional attributes (add / rename / remove, protected or not). The group is
  not edited (see *Groups and tags*).
- Tags are chips: each has a remove button; new ones are typed (Enter or a comma adds one) with the
  database's other tags offered.
- **Attachments:** in the entry view a file can be added to an entry, replaced, renamed or removed (up to
  20 MB — the whole database is synced on every change); the change is saved at once, with the previous
  version in history, like an edit. A name the entry already uses gets a number (`scan (2).pdf`) rather
  than replacing the file. A removed file stays in the database for the history version that has it:
  keepass-rs would drop it from the database's file pool although that version still uses it, so the file
  is removed on a copy of the database and only the entry is taken from it.
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

## Password health

- A report, opened from the toolbar, of passwords worth changing among the entries in use (not the
  recycle bin): **reused** (the same password in more than one entry), **weak** (zxcvbn score 0–1, as the
  strength indicator rates it) and **unchanged for over a year**. Each entry is listed once, under the
  first of these that applies.
- A password's age runs from when it was last set: the oldest version in the entry's history with the
  same password, so an edit to anything else does not make it newer.
- Worked out in the backend on demand: the window gets titles and reasons, never a password, and nothing
  is sent anywhere or stored.
- Each listed entry opens in the editor with the password field focused.

## Saving and synchronisation

The app always works on the database's own file on this PC. Without sync the app keeps that file
consistent when something else changes it (below) but cannot tell when another program delivers it to
other devices; with sync (a LAN folder, Dropbox, OneDrive or Google Drive — see *Synchronisation with a
remote store*) the app itself keeps it paired with the remote file. That is the way to share the database
with a phone. Saving, change detection and the per-change merge below apply either way.

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
  - nothing is removed from the file except by deleting for good in the trash (a removed attachment
    stays for the entry's history).
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
the app does not keep the remote file live: the database's local file is the working copy, and at defined
moments the app **compares** it with the remote file and decides what to do. The window always shows what
the last sync did.

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
  lists the entries that were merged. The remote file as it was before is kept as `<name>.kdbx.remote.bak`
  beside the database's file (the clouds also keep their own version history).
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
  the state file or logs. **Disconnect** removes it; the database's local file stays.
- Each store gets the narrowest access that still reaches a file Keepass2Android can open: Dropbox — its
  app folder (`Apps/PswManager Sync`); Google Drive — `drive.file`, the files the app created itself, in a
  `PswManager` folder the app makes at the top of the Drive, which the user may move anywhere (e.g. into an
  `Apps` folder, like Dropbox's; so a database gets there by uploading it from the app);
  OneDrive — `Files.ReadWrite.AppFolder`, its app folder (`Apps/PswManager`, named after the app's
  registration), for personal Microsoft accounts. A file is downloaded from the short-lived address
  OneDrive gives for it, so the access token is never sent to another host; Microsoft hands out a new
  refresh token on each renewal, and the app keeps the newest.
- Google Drive's OAuth client secret (not a secret for installed apps, but not kept in the source either)
  is given at build time as `PSWM_GOOGLE_CLIENT_SECRET`; a build without it offers no Google Drive. While
  the Google app is in testing, Google asks to sign in again every 7 days; the status says so and the
  changes wait meanwhile.
- One account per store: databases synced with the same store share its sign-in. A store's account is
  signed out when no database in the list uses it any more.

**Setting up sync for a database** (*Settings → Sync*, for the open database; opening a database from a
store sets it up too):

- **Upload to a store**: creates a new file there from this database (Dropbox's app folder, OneDrive's
  app folder, Google Drive's PswManager folder, a file in a LAN folder) and syncs with it. It never replaces a file already there.
- **Link to an existing remote file**: pick a file in the store. When it and the local file differ, the
  app asks what to do: **Merge both** (the usual KeePass merge, then upload), **Use the remote file** (it
  replaces the local file; the local one is kept as `<name>.kdbx.bak`), or **Keep the local file** (it
  replaces the remote file; the remote one is kept as `<name>.kdbx.remote.bak`). Linking never happens
  silently.
- **Stop syncing**: the database stays where it is, as a plain local file; the remote file is left alone.
  Refused while the local file has changes the remote file lacks — unlocking syncs them first.
- A LAN folder needs no account; the file in it is picked with the file dialog.
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
| `<file.kdbx>` | Open this database; a file not in the list is added to it (without sync) |
| `--data-dir <path>` | Use an explicit data location |
| `--help` / `--version` | Print to the console the exe was launched from |

A second launch with a file hands it to the running app, which opens it when it is locked (and not
waiting to sync changes); otherwise the window says why not. An unknown option prints the usage and exits
with code 2.

## State

- One JSON file: settings, the list of databases (for each: its file, key file, sync — the remote
  file and the sync state — and its name and description as last unlocked, for the unlock screen, which
  cannot read the encrypted file), which one opens next, window geometry. **Never** the master password, a
  cloud token or any secret. Site icons are cached in `icons/` beside it.
- An older state file (one database, a working copy in `sync/`) is read as a list of one; its working
  copy stays where it is and keeps syncing.
- **Location:** `pswm.json` next to the exe if that folder is writable; otherwise
  `%APPDATA%\pswmanager\pswm.json`. `--data-dir` overrides both.

## Technology

- **Shell:** Tauri 2 (Rust) + WebView2, tray via Tauri's `tray-icon` feature.
- **Code:** one Cargo workspace: `crates/core` holds what does not depend on the platform (the database,
  editing, merging, syncing and the stores, TOTP, the generator, password health, site icons) and is
  shared with the Android app (see `spec-android.md`); `src-tauri` is the Windows app around it (the
  window, tray, hotkey, clipboard, file watching, the Credential Manager, which the core reaches through
  its secret-store hook).
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

`sic2kdbx.py` stays a separate, one-off Python tool, in `tools/sic2kdbx` (see its README). It is not part of the app and not part of
the release.

## Testing

- Automated (Rust): KDBX round trip preserves everything listed under *Database*; merge cases (edit / edit,
  edit / delete, move, new on both sides, history union); atomic save and `.bak`; change detection;
  the sync decision, conditional upload and retry against a fake store; merging two databases;
  database settings (merge by their times, history trimmed by count and size, a key change synced with a
  remote file still on the old key, a key changed on another device taken over by the newer
  `MasterKeyChanged`);
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
7. Databases and sync apart: a list of databases (one open), create a new database, open a local file,
   open from a store into a visible local file; sync set up per database (upload, link with an explicit
   merge / use remote / keep local choice, stop); state migration
8. Groups and tags: the three-column window with fixed groups (All, Favorites, Expired, 2FA, Passkey,
   Templates, Trash) and tags in the sidebar; the favorite star and expiry date; tags on several entries
   at once, renaming and removing a tag; restore, delete permanently and empty in the trash; new entries
   from templates; no group field
9. Entry history: when an entry was last changed in the entry view; its older versions listed with what
   changed; a version shown read only, with its files; restoring a version
10. Database settings: name, description and default user name kept in the file and merged by their
    times; renaming the local file; history limits by count and size; changing the master password / key
    file and the encryption, also with sync (the old key kept for the remote file, asked for after a lock)
