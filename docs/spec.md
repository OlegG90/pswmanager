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
| Android phones | Keepass2Android (reads and writes the same file, merges on conflict) |
| iPad | **Out of scope for now** |

### Out of scope

Browser autofill and browser extensions, an own sync server or cloud APIs, iPad, creating a new database
(the database comes from `sic2kdbx` or KeePassXC), several open databases at once, importing from other
password managers inside the app (SafeInCloud migration stays with `sic2kdbx.py`), editing attachments,
Windows Hello unlock, password-health reports, sharing, KDBX 3 writing.

## Database

- One KDBX 4 file, chosen once and remembered. Unlocked with a master password, a key file, or both.
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
  notes and additional attributes. Protected attributes are masked like the password.
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
- Menu: Show, Lock, Start with Windows (a check mark), Settings, Quit.
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

## Editing

- Create, edit and delete entries. Delete asks for confirmation in the entry view, then moves the entry to
  the recycle bin (as KeePass does; created if missing). A database with the bin turned off is refused —
  removing an entry with keepass-rs can leave other entries' attachments pointing at the wrong data.
- Values the editor only reformats (line breaks an input cannot hold, spaces around a URL, tag and group
  spelling, a TOTP value the app cannot read) are saved as they were, so an untouched entry saves
  unchanged. An entry whose group path did not change stays in its own group, even when group names repeat.
- Editable: title, user name, password, URL, notes, tags, group, TOTP secret, additional attributes
  (add / rename / remove, protected or not).
- Every edit pushes the previous version into the entry's history.
- **Password generator** in the editor: length (default 20, 8–64), upper / lower / digits / symbols,
  exclude look-alike characters. Generated with the OS CSPRNG.
- **Strength indicator** (zxcvbn) next to the password field.
- **TOTP**: codes are computed from the entry's `otp` attribute (`otpauth://` URI, as `sic2kdbx` and
  KeePassXC write it), with a countdown.

## Saving and synchronisation

The file lives in a synced folder (OneDrive, with "Always keep on this device" for that folder). The app does
not talk to any cloud; it keeps the file consistent when another device changes it.

- **Save:** write to a temporary file in the same folder, then rename it over the original atomically, so
  the sync client never picks up a half-written file. The new file is opened again with the key before it
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
  - where both changed the same entry, this change is the newer one and wins; the other version goes into
    the entry's history, so nothing is lost;
  - an entry deleted elsewhere (recycle bin or `DeletedObjects`) and edited here comes back with its id —
    the edit is newer than the deletion; deleting an entry already gone elsewhere does nothing;
  - nothing is ever removed from the file, so keepass-rs's attachment renumbering on removal never applies.
- An entry open in the editor is never replaced silently: if another device changed it, the editor says so,
  and saving keeps this version with the other one in history.
- A file that cannot be read (wrong key, corrupt, mid-sync) is never overwritten; the app shows the error,
  keeps the in-memory data, and saves again once the file can be read.
- Saving checks that the new file reads back with the same content before it replaces the old one. (Old
  versions in history are compared by what the file keeps of them: keepass-rs also remembers in memory
  which group each was made in, which the file does not store.)

**Compatibility gate:** a file saved by PswManager must open in KeePassXC and in Keepass2Android, and a
change made in Keepass2Android must merge back without loss. This is checked before every release.

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

- One JSON file: settings, the database path, the key file path, window geometry. **Never** the master
  password or any secret. Site icons are cached in `icons/` beside it.
- **Location:** `pswm.json` next to the exe if that folder is writable; otherwise
  `%APPDATA%\pswmanager\pswm.json`. `--data-dir` overrides both.

## Technology

- **Shell:** Tauri 2 (Rust) + WebView2, tray via Tauri's `tray-icon` feature.
- **Frontend:** plain TypeScript, no framework.
- **Crates:** `keepass` (`save_kdbx4`), `zeroize`, `notify`, `zxcvbn`, `totp-lite`; an HTTP client for
  site icons.
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
