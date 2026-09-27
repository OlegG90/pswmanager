# PswManager

A minimal tray password manager for Windows on top of a KeePass (KDBX 4) file. The same file opens in
KeePassXC and Keepass2Android. See [docs/spec.md](docs/spec.md) for the specification.

## Download and first run

1. From the [latest release](https://github.com/OlegG90/pswmanager/releases/latest), download
   `pswm-x64.exe` (most PCs) or `pswm-arm64.exe` (Windows on ARM, e.g. Snapdragon laptops), and rename it
   `pswm.exe` if you like. There is no installer: put it in any folder.
2. The exe is not code-signed, so on first run Windows SmartScreen asks: choose *More info → Run anyway*.
3. Choose the database: a local `.kdbx` file (**Open a local file…**), a file in a folder such as a NAS
   share (**Sync with a folder…**), or a file in Dropbox (**Sync with Dropbox…**, which signs in through the
   browser). New databases come from KeePassXC or from `sic2kdbx` below.
4. The app lives in the notification area; `Ctrl+Alt+P` shows or hides the window. Settings: `Ctrl+,`.

Settings, the chosen database and cached site icons are kept in `pswm.json` beside the exe when that
folder is writable (portable), otherwise in `%APPDATA%\pswmanager`. Nothing secret is stored there; a
cloud sign-in is kept in the Windows Credential Manager.

```
pswm [<file.kdbx>] [--data-dir <path>]
pswm --help | --version
```

## Development

Requires Node.js, Rust (MSVC toolchain) and Visual Studio Build Tools with the C++ workload.

```
npm install
npx tauri dev            # dev mode
npm test                 # frontend (vitest) and backend unit tests
npm run build            # this machine's architecture: src-tauri/target/release/pswm.exe
npm run build:x64        # src-tauri/target/x86_64-pc-windows-msvc/release/pswm.exe
npm run build:arm64      # src-tauri/target/aarch64-pc-windows-msvc/release/pswm.exe
```

Releases: bump `version` in `src-tauri/Cargo.toml`, merge, then push a tag `v<version>` from `main`.
GitHub Actions ([release.yml](.github/workflows/release.yml)) runs the tests, builds both exes and
attaches them to the release; *Run workflow* by hand builds them as an artifact without publishing.

The app icon is generated from [src-tauri/icons/app-icon.svg](src-tauri/icons/app-icon.svg) with
`npx tauri icon src-tauri/icons/app-icon.svg -o src-tauri/icons` (then delete the non-Windows files).

## sic2kdbx — SafeInCloud XML → KeePass (KDBX 4)

An offline converter from a SafeInCloud export to a KeePass database, which opens in PswManager,
KeePassXC and Keepass2Android. It is a one-off migration tool, not part of the app.

```
python -m venv .venv
.venv\Scripts\python -m pip install -r requirements.txt
.venv\Scripts\python sic2kdbx.py export.xml base.kdbx
```

The script asks for the master password twice. Options: `--keyfile`, `--no-password` (key file only),
`--skip-deleted` (leave deleted cards out; by default they go to the recycle bin), `--drop-empty` (leave
empty fields out, except in templates), `--force`.

Elements are written in the order KeePass uses. Databases converted before that change store some
fields out of order; PswManager refuses them rather than risk misreading protected values. Convert again,
or open and save once in KeePassXC.

`;` and `,` in label names become spaces in tags (KeePass separates tags with them); the group name keeps
the original. Damaged attachments are skipped with a warning.

| SafeInCloud | KDBX |
|---|---|
| first login (or e-mail) / password / website / OTP field | UserName / Password / URL / otp |
| other fields | additional attributes; password, pin and secret ones are protected |
| notes | Notes |
| labels | tags; the first label is the group |
| image, file | attachments |
| field history | entry history |
| templates | the Templates group, marked as KeePass's templates group |
| star | the Favorite tag |
| field types, order, symbol, colour | CustomData `SafeInCloud` (JSON) |

Tests: `.venv\Scripts\python -m unittest discover -s tests`

After converting, delete the XML export: it holds the passwords in plain text.
