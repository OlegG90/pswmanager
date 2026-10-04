# Research: PswManager as a credential provider

Issue #157, October 2026. On the phone PswManager shows the passkeys it has (imported, or made by KeePassXC
on Windows), but it cannot **make** one when a site or app asks, **sign in** with one, or **save** a new
one. All three need the **credential provider** role on Android, the role Google Password Manager,
Bitwarden and KeePassDX have. The same role offers passwords to apps that ask through Credential Manager,
and it is what exporting to another app needs (#153). No code here; the end proposes the stages and issues.

## What Android asks of a provider

- **A service**, `CredentialProviderService` (`androidx.credentials` 1.6), declared with the permission
  `android.permission.BIND_CREDENTIAL_PROVIDER_SERVICE`, the action
  `android.service.credentials.CredentialProviderService` and a `provider.xml` listing its capabilities:
  `androidx.credentials.TYPE_PUBLIC_KEY_CREDENTIAL` (passkeys) and
  `android.credentials.TYPE_PASSWORD_CREDENTIAL` (passwords).
- **Android 14 or later**: the system does not bind third-party providers before. The app keeps
  `minSdk = 29`; the service is simply not used on older phones.
- **The user turns it on** in *Settings → Passwords, passkeys & accounts* (an intent,
  `Settings.ACTION_CREDENTIAL_PROVIDER`, opens it from PswManager's settings).
- **Two phases.** The system asks every enabled provider at once and must get an answer quickly:
  - *begin*: `onBeginCreateCredentialRequest` / `onBeginGetCredentialRequest` return entries (an account
    to save to; the passkeys or passwords that match), each with a `PendingIntent`. **Locked**, the
    provider returns an `AuthenticationAction` ("Unlock PswManager") instead: after unlocking, its activity
    hands the entries back (`PendingIntentHandler.setBeginGetCredentialResponse`);
  - *selection*: the user picks an entry, the provider's activity runs (`PendingIntentHandler`), asks for
    user verification (fingerprint, `BiometricPrompt`), does the work and returns the result.

## Running without the app's window

The service and its activities run in the app's process, but often without the Tauri window: Android
starts the process only for the service. The background upload already works this way (`background.rs`,
`UploadWorker.kt`): Kotlin calls Rust through JNI, and the core gets its secrets (`Keystore.kt`) and
documents (`DocumentIo.kt`) from Kotlin. A provider does the same:

- **Begin while locked**: no key is held, so the answer is always the unlock action. It never shows
  titles or user names without an unlocked database (they are inside the encrypted file).
- **Unlock**: a small activity of PswManager's own, outside the Tauri window: the fingerprint (the key
  sealed for biometric unlock, A3), or the master password when it is due (`passwordEveryDays`) or no
  key is sealed. The database is then unlocked in the Rust core and stays so, as in the app, under the
  same locking rules (in the background, screen off, inactivity).
- **If the app is unlocked already**, the service reads from the same session: no second unlock.

## WebAuthn, done by the core

The authenticator's part of WebAuthn belongs in `crates/core` (Rust: `p256`, `sha2`, `ciborium` for
CBOR), so Windows can use it later:

- **Make a passkey** (`navigator.credentials.create`): an ES256 (P-256) key pair, a random 32-byte
  credential id, attestation **"none"**, authenticator data with the RP id hash, the flags UP, UV, BE and
  BS, and the AAGUID. Stored as KeePassXC stores it; the response JSON goes back to the system.
- **Sign in** (`navigator.credentials.get`): the passkeys whose relying party matches; the assertion signed
  with the private key. The signature counter stays 0, as KeePassXC and other synced passkeys do.
- **The client data.** A browser (a *privileged* caller) sends its own `clientDataHash` and origin, which
  are used as they are. An app gets the origin `android:apk-key-hash:<sha256 of its signing cert>`;
  the relying party proves the app is its own through Digital Asset Links.
- **Which browsers are trusted** to speak for a site: Google's public allowlist of privileged apps
  (`gpm-passkeys-privileged-apps/apps.json`), as KeePassDX uses, checked through
  `CallingAppInfo.getOrigin(allowlist)`. It is kept in the app and updated with releases.

## Storage: KeePassXC's attributes

The passkeys must work in KeePassXC (Windows) and KeePassDX too, so PswManager writes what they write:
`KPEX_PASSKEY_USERNAME`, `_CREDENTIAL_ID`, `_PRIVATE_KEY_PEM`, `_RELYING_PARTY`, `_USER_HANDLE`, plus
`KPEX_PASSKEY_FLAG_BE` / `_FLAG_BS` and `KPEX_PASSKEY_PRF` (KeePassDX), and the tag *Passkey*. Reading
them is there already (the *Passkey* group, `edit::passkey`). A new passkey goes into the entry for the
site and user name when there is one (the user picks it), or a new entry. Saving and syncing are as for
any change.

## Passwords through the same service

Credential Manager also asks for **passwords**, from apps that use it (`GetPasswordOption`), and offers
to **save** them. That is not Android's autofill: most apps and every site still fill passwords through
the separate `AutofillService` (what Keepass2Android does), a stage of its own. Offering passwords here is
little extra work once the provider is there.

## Windows

Windows 11 lets third-party passkey managers in since the November 2025 update (a WebAuthn plugin
authenticator, used by 1Password and Bitwarden). With the WebAuthn core in Rust, PswManager for Windows
could join later; not now.

## Security

- Every passkey use and every new passkey needs a **user verification** (fingerprint or the master
  password) in the selection phase, even with the database unlocked.
- Nothing about entries is told while locked; the begin phase only offers to unlock.
- The private key lives only in the database (protected attribute); it is never logged and is wiped
  after signing.
- The origin a browser gives is trusted only from the allowlist; otherwise the caller's own signing
  certificate makes the origin.
- Exporting (#153) reuses the same role.

## Recommendation and stages

A stage of its own, **A5: credential provider**, after a spike:

1. **Spike** on the phone: the service declared and turned on in Settings; `webauthn.io` in Chrome
   offers PswManager when making a passkey; the begin phase answers in time with the app not running.
2. **Core: WebAuthn authenticator**: make a passkey (attestation "none") and sign an assertion, in Rust,
   with tests against known vectors and KeePassXC's own test values; KeePassXC's attributes written.
3. **Android: unlock from the service**: the unlock activity outside the Tauri window (fingerprint or
   master password), sharing the session with the app.
4. **Android: sign in with a passkey**: the begin entries, the selection, verification, the assertion.
5. **Android: save a new passkey**: into an existing entry or a new one; saved and synced.
6. **Android: passwords through Credential Manager**: offer and save.
7. **Android: export to another app** (#153).

`AutofillService` (passwords in every app and site) stays a separate stage.

## Sources

- Android Developers, [Integrate Credential Manager with your credential provider solution](https://developer.android.com/identity/sign-in/credential-provider)
- [Google's privileged apps allowlist](https://www.gstatic.com/gpm-passkeys-privileged-apps/apps.json)
- KeePassDX: [`PasskeyEntryFields.kt`](https://github.com/Kunzisoft/KeePassDX/blob/master/database/src/main/java/com/kunzisoft/keepass/model/PasskeyEntryFields.kt) (its attributes), passkeys since 4.2.0
- KeePassXC: `src/browser/BrowserPasskeys.cpp` (authenticator data, flags, AAGUID)
- Microsoft, [third-party passkey managers in Windows 11](https://learn.microsoft.com/windows/apps/develop/security/third-party)
