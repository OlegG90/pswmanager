# Research: filling in passwords on Windows

October 2026. On Android PswManager fills logins into other apps and browsers through the system's
autofill service (#236). This note asks what the same takes on Windows, where no such service exists,
and proposes the stages and issues. No code here.

## What Windows offers

- **No system autofill for passwords.** Android (`AutofillService`, Credential Manager), iOS and macOS
  (AutoFill credential provider extensions) let a password manager register and have the system offer
  its logins in other apps' fields. Windows has nothing of the kind for passwords: a desktop app cannot
  see or fill another app's fields through the system.
- **A system way for passkeys only.** Since the November 2025 update Windows 11 (24H2 / 25H2, build
  26100 / 26200 with minor version 6725 or later) accepts third-party passkey managers as **plugin
  authenticators** (`WebAuthNPluginAddAuthenticator` and the other `WebAuthNPlugin*` calls in
  `webauthn.dll`). Windows' own WebAuthn dialog, which browsers and apps use, then offers their passkeys;
  1Password and Bitwarden use it. The user turns a provider on in *Settings → Accounts → Passkeys →
  Advanced options*.

So passwords on Windows are filled in one of two ways that do not depend on the system: typed into the
active window, or handed to a browser extension.

## Ways to fill

### Auto-Type (any app)

KeePass's and KeePassXC's way: a global hot key, the active window's title matched against the entries,
and a sequence such as `{USERNAME}{TAB}{PASSWORD}{ENTER}` typed as key presses (`SendInput`).

- **Works everywhere**, native apps and browsers alike, with no extension.
- **Already in the file.** The sequence and window associations are part of KDBX (`AutoType` of an
  entry, `DefaultAutoTypeSequence` and `EnableAutoType` of a group); the core reads and keeps them, so
  what is set in KeePassXC carries over.
- **Matches by title, not by address.** A page titled like a bank's gets the bank's login. A browser's
  address can be read from its address bar through UI Automation, as some KeePass plugins do, which
  narrows this for browsers.
- **Limits** KeePass documents: a window running as administrator gets nothing unless PswManager runs so
  too (Windows' integrity levels, UIPI); a remote desktop or VM window must use the same keyboard
  layout. Typing Unicode characters (`KEYEVENTF_UNICODE`) avoids most layout trouble locally.

### Browser extension (web pages)

Filling a page by its fields and its URL takes an extension inside the browser. The extension talks to
the desktop app through **native messaging**: the browser starts a small host process named in a
manifest registered per browser (registry keys for Chrome, Edge and Firefox) and talks to it over
stdin / stdout; the host passes messages on to the app, KeePassXC through a named pipe.

- **Our own extension** means three store listings (Chrome Web Store, Edge Add-ons, Firefox Add-ons),
  their reviews, and an extension to keep up with browser changes.
- **The KeePassXC-Browser protocol** is documented (`keepassxc-protocol.md` in
  keepassxreboot/keepassxc-browser): `associate`, `get-logins`, `set-login`, `generate-password`,
  `get-totp`, `passkeys-get` / `-register` and the rest, encrypted with NaCl box. Speaking it lets the
  existing KeePassXC-Browser extension work with PswManager, with no extension of our own. Open
  questions: the host name the extension expects against an installed KeePassXC, and keeping the
  association keys where KeePassXC keeps them in the file, so one association serves both.

### Passkey provider (Windows 11)

The plugin authenticator above: PswManager registers a COM server, keeps the list of its passkeys in
the system (`WebAuthNPluginAuthenticatorAddCredentials`), and makes or signs with a passkey when a site
or app asks, after the user's verification (Windows Hello, `WebAuthNPluginPerformUserVerification`).
The core already makes and signs passkeys (it does so on Android). Open question: Microsoft's sample
registers the COM server through `Package.appxmanifest`, which suggests the plugin needs **package
identity** (MSIX, or a sparse package beside the plain `pswm-*.exe`), a change to how PswManager is
released.

## Proposal

Three stages, cheapest and widest first. Each issue starts with research and moves to implementation
only after the maintainer agrees its scope.

| Stage | Issue | Covers | Cost |
|---|---|---|---|
| 1. Auto-Type | #272 | passwords in any app and browser, by window title (and the browser's address) | small: the tray app, a hot key, `SendInput`; the data is already in the file |
| 2. KeePassXC-Browser protocol | #273 | passwords (and passkeys) in web pages, by URL and field | medium: a native messaging host, the protocol, NaCl; no extension of our own |
| 3. Passkey provider | #274 | passkeys in every browser and app on Windows 11 | medium to large: COM, `webauthn.dll`, probably MSIX packaging |

Browser autofill and extensions are out of scope in `spec.md` today; the stage that adds them changes
that line.

## Sources

- KeePass, *Auto-Type*: https://keepass.info/help/base/autotype.html
- KeePassXC-Browser protocol: https://github.com/keepassxreboot/keepassxc-browser/blob/develop/keepassxc-protocol.md
- Microsoft, *Plugin passkey manager support*: https://learn.microsoft.com/windows/apps/develop/security/third-party
- Microsoft, Passkey Manager sample: https://github.com/microsoft/Windows-classic-samples/tree/main/Samples/PasskeyManager
