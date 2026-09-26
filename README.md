# PswManager

A minimal tray password manager for Windows on top of a KeePass (KDBX 4) file — in development.
See [docs/spec.md](docs/spec.md) for the MVP specification.

## Development

Requires Node.js, Rust (MSVC toolchain) and Visual Studio Build Tools with the C++ workload.

```
npm install
npx tauri dev            # dev mode
npm test                 # frontend (vitest) and backend unit tests
npm run build            # this machine's architecture: src-tauri/target/release/pswm.exe
```

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
