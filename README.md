# PswManager

A minimal tray password manager for Windows on top of a KeePass (KDBX 4) file. The same file opens in
KeePassXC and Keepass2Android. See [docs/spec.md](docs/spec.md) for the specification.

## Download and first run

1. From the [latest release](https://github.com/OlegG90/pswmanager/releases/latest), download
   `pswm-x64.exe` (most PCs) or `pswm-arm64.exe` (Windows on ARM, e.g. Snapdragon laptops), and rename it
   `pswm.exe` if you like. There is no installer: put it in any folder.
2. The exe is not code-signed, so on first run Windows SmartScreen asks: choose *More info → Run anyway*.
3. Choose the database: **Create a new database**, **Open a local file** (a `.kdbx` on this PC), or open
   one from a folder such as a NAS share, from Dropbox, Google Drive or OneDrive (signing in through the
   browser); a database opened from a store is kept in a file on this PC that stays in step with it. Sync
   for the open database is set up or stopped in *Settings → Sync*. Several databases can be in the list;
   one is open at a time. A SafeInCloud export converts with [sic2kdbx](tools/sic2kdbx/README.md).
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
npm run build            # this machine's architecture: target/release/pswm.exe
npm run build:x64        # target/x86_64-pc-windows-msvc/release/pswm.exe
npm run build:arm64      # target/aarch64-pc-windows-msvc/release/pswm.exe
```

The code: `src-tauri` is the Windows app (Tauri), `crates/core` the core it shares with the Android app
(the database, merging, syncing and the stores; its tests run on the PC). One Cargo workspace, built into
`target/`.

The Android app (`apps/android`, see [docs/spec-android.md](docs/spec-android.md)) is built only in CI: the
Android SDK and NDK have no Windows-on-ARM64 builds. Each push that touches it builds an APK signed with the release key (workflow *Android*, artifact
`pswmanager-apk`), so it installs over the released app and keeps its data:
`adb install -r <the .apk>` (`adb` is in Google's platform-tools). Its page is in
`apps/android/ui` (`npm run build:android-web`).

Releases: bump `version` in the root `Cargo.toml` (`[workspace.package]`, both apps), merge, then push a tag `v<version>` from `main`.
GitHub Actions ([release.yml](.github/workflows/release.yml)) runs the tests, builds both exes and
attaches them to the release; *Run workflow* by hand builds them as an artifact without publishing.

The app icon is generated from [src-tauri/icons/app-icon.svg](src-tauri/icons/app-icon.svg) with
`npx tauri icon src-tauri/icons/app-icon.svg -o src-tauri/icons` (then delete the non-Windows files).

## sic2kdbx — SafeInCloud XML → KeePass (KDBX 4)

A one-off converter from a SafeInCloud export to a KeePass database, kept apart from the app in
[tools/sic2kdbx](tools/sic2kdbx/README.md).
