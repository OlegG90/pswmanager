# Working on PswManager

Instructions for coding agents (and people) changing this repository. `CLAUDE.md` adds what is specific
to Claude Code.

## The project

A password manager on top of a KeePass (KDBX 4) file that also opens in KeePassXC and Keepass2Android:
a tray app for Windows and an app for Android. The specifications are the source of truth:
[docs/spec.md](docs/spec.md) (Windows and what both share) and [docs/spec-android.md](docs/spec-android.md).

| Path | What |
|---|---|
| `crates/core` | Rust core shared by both apps: the database, editing, sync, import (CXF), WebAuthn |
| `src-tauri` | Windows app (Tauri 2), the `pswm` binary |
| `src` | Windows UI (TypeScript, no framework); the parts both UIs share live here too |
| `apps/android/src` | Android app's Rust side (Tauri 2 commands, JNI) |
| `apps/android/ui/src` | Android UI; imports the shared parts from `src/` |
| `apps/android/gen/android` | Gradle project and Kotlin plugins (made by `tauri android init`, kept and edited by hand) |
| `docs` | specs, research notes, design mockups |
| `tools/sic2kdbx` | separate SafeInCloud → KDBX converter |

## Rules

- **The repository is public.** Before every commit and push, check each changed file for private data:
  no real databases, exports, passwords, key files, tokens, personal paths or e-mail addresses. Tests and
  examples use made-up data only. Secrets never go into issues, PRs, logs or chat.
- **English** for everything in the repository: code, comments, docs, commit messages, PRs, issues.
- **Specs change with the code.** A change in behaviour updates `docs/spec.md` and / or
  `docs/spec-android.md` in the same branch, with the issue number (`#123`) where it helps.
- Write like the surrounding code: its naming, comment density and style (comments say why, in full
  sentences). Keep each file's line endings (some files are CRLF).
- Delete or overwrite files, branches or data only when the maintainer has confirmed it.

## Workflow

Every change goes through a branch and a pull request; nothing is committed to `main` directly.

1. **Sync:** `git fetch`, switch to `main`, `git pull --ff-only`. Uncommitted changes in the checkout:
   stop and ask; never stash or discard them silently.
2. **Branch** from the updated `main`: `feat/…`, `fix/…`, `docs/…`, `chore/…`. When another agent may be
   working in the same checkout, use a separate `git worktree` instead of switching its branch.
3. **Implement** on that branch, with tests where the core changes, and the specs updated.
4. **Review** the branch diff for bugs and fix what the review confirms.
5. **Simplify:** a quality pass over the diff, with no change in behaviour.
6. **Pull request** against `main`: what changed and why, how it was verified (tests, and what was
   checked by hand on which device), and any breaking change (renamed commands, settings migrations).
   Reference the issue (`Closes #123`).
7. **Merging is the maintainer's call.** After the merge: sync `main` and delete the branch, locally and
   on GitHub.

## Building and testing

```
npm install
npm test                      # vitest, then cargo test
npx tsc --noEmit              # Windows UI types
npx tsc --noEmit -p apps/android/ui   # Android UI types
cargo clippy --all-targets
npm run build                 # Windows: target/release/pswm.exe
```

- `npm run build` fails while `pswm.exe` runs (the tray icon included): close it first.
- CI (`.github/workflows/ci.yml`) runs `tsc` (the Windows UI only), `npm test` and `clippy` on every pull
  request and every push to `main`; `audit.yml` runs `cargo audit` and `npm audit` there and weekly. Type-check
  the Android UI yourself.
- **The Android APK is built only in GitHub Actions** (workflow *Android*): its native dependencies need
  the NDK's clang, so `cargo check` for the Android target does not run on a Windows PC. A push to any
  branch that touches the Android app, the core or the root manifests and lockfiles starts the build by
  itself; starting it by hand as well cancels one of the two runs. The artifact `pswmanager-apk` holds
  `universal/release/app-universal-release.apk` (a debug APK where the signing secrets are missing, as
  on forks); install it with `adb install -r <apk>`.
- Check changes to the Android UI or its Kotlin plugins on a real phone before the pull request.

## Releases

Version tags and releases are cut from `main` after the merge, never from a branch.

1. A pull request from `chore/version-X.Y.Z` raises `version` in the root `Cargo.toml`
   (`[workspace.package]`); `Cargo.lock` follows.
2. After it is merged: `git tag vX.Y.Z` on `main` and `git push origin vX.Y.Z`.
3. The *Release* workflow checks the tag matches the version and publishes `pswm-x64.exe`,
   `pswm-arm64.exe` and `pswmanager-android.apk`. Make sure all three are on the release.
