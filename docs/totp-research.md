# Research: TOTP gaps

Issue #252, October 2026. PswManager computes TOTP codes from the entry's `otp` attribute (an
`otpauth://totp/` URI or a bare base32 secret; SHA-1 / SHA-256 / SHA-512, any period, 6–10 digits), shows
them with a countdown, copies them, edits the secret, and carries it through import and export. This note
looks at what other KeePass clients and authenticator apps handle that PswManager does not, and recommends
what to build. No code here; the end of this note lists the issues opened from it.

## How the clients store TOTP

| Format | Where | KeePass 2 | KeePassXC | Keepass2Android | PswManager |
| --- | --- | --- | --- | --- | --- |
| Key URI (`otpauth://totp/…`) | `otp` | No (plugins only) | Reads, writes | Reads | Reads, writes |
| KeeOtp (`key=…&step=…&size=…&otpHashMode=…`) | `otp` | KeeOtp plugin | Reads; writes back in the same form | Reads | No |
| KeeTrayTOTP | `TOTP Seed` (base32) and `TOTP Settings` (`period;digits`, digits `S` for Steam) | KeeTrayTOTP plugin | Reads first of all; writes back in the same form | Reads | No (hides `TOTP Seed` as a secret) |
| KeePass 2 native (2.47+) | `TimeOtp-Secret` / `-Hex` / `-Base32` / `-Base64`, `TimeOtp-Length` (≤ 8), `TimeOtp-Period`, `TimeOtp-Algorithm` (`HMAC-SHA-1`, `-256`, `-512`) | Reads, writes (`{TIMEOTP}`) | Reads `-Base32` only, last of all; never writes | Reads `TimeOtp-Secret(-XXX)` | No (hides the secret) |
| KeePass 2 HOTP | `HmacOtp-Secret*`, `HmacOtp-Counter` | Reads, raises the counter on use (`{HMACOTP}`) | No | No | No |
| Steam | `encoder=steam` in the URI, or `S` in `TOTP Settings` | No | Reads, writes | Reads (`S`) | No |

- **KeePassXC's order** (`Entry::updateTotp`): `TOTP Settings` + `TOTP Seed`, then `otp`, then
  `TimeOtp-Secret-Base32` with its length, period and algorithm. Setting a new TOTP removes `otp`,
  `TOTP Seed` and `TOTP Settings` and writes one of them; it leaves the `TimeOtp-*` attributes alone.
- **Keepass2Android** reads all four layouts, writes one, and scans a QR code in its *Configure TOTP* dialog.
- **KeePass 2 itself** reads only its own fields; the `otp` URI that KeePassXC, Keepass2Android and
  PswManager share needs a plugin there. Its databases made since 2.47 hold TOTP only in `TimeOtp-*`.

So an entry made in KeePass 2, or in KeePass 2 with KeeTrayTOTP, shows a code in KeePassXC and Keepass2Android
but none in PswManager.

## Gaps

Numbered as in #252.

### 1. Other storage layouts (KeePass 2, KeeTrayTOTP, KeeOtp)

PswManager already recognises their secrets (`TOTP Seed`, `TimeOtp-Secret*`), but only to keep them out of
the summary. Reading them is a parser change in the core (`otp.rs`) plus a lookup of which attributes an
entry has:

- the same order as KeePassXC, so the two apps show the same code for the same entry, with the KeePass 2
  layout widened to every `TimeOtp-Secret*` encoding (UTF-8, hex, base32, base64) as Keepass2Android reads;
- `TimeOtp-Length` up to 8, `TimeOtp-Algorithm` by KeePass's names;
- a KeeOtp string in `otp` (`key=` instead of an `otpauth:` scheme).

**Writing.** As KeePassXC does, the editor writes a changed TOTP back in the layout it was read from, so
the plugin that wrote it keeps working: KeeTrayTOTP's `TOTP Seed` and `TOTP Settings`, a KeeOtp string, or
the `otp` URI. A new TOTP, or one a layout cannot hold (KeeOtp has no Steam), goes to `otp` as a URI, the
layout three of the four apps read, and the KeeTrayTOTP attributes are removed so they cannot shadow it in
KeePassXC. `TimeOtp-*` attributes come last in every reader, so they never shadow anything; but KeePass 2
reads nothing else, so when an entry has them, the editor writes the same secret, period, length and
algorithm there too when they fit (at most 8 digits, not Steam). Clearing the TOTP removes all of them. An
entry whose TOTP is not touched keeps its attributes exactly as they were.

The 2FA filter, the copy action and the entry view take whether an entry has a TOTP, and its code, from the
core (`vault.rs`), so they follow with no change of their own. The editor shows the TOTP it finds as a URI;
the export (CSV, CXF) and the merge of similar entries take that same TOTP instead of `otp` alone.

**Recommendation: do.** It is small, all in the core, and it fixes entries that other clients already show.

### 2. Scanning a QR code

Today the secret is pasted. Setting up 2FA on a site shows a QR code; the secret in text is usually behind a
*Can't scan?* link.

- **From an image, both apps.** A QR code decoded from an image by the core with
  [`rqrr`](https://crates.io/crates/rqrr) (pure Rust, MIT / Apache-2.0): on Windows from the clipboard (a
  `Win+Shift+S` snip of the site's QR code) or a picked image file; on Android from a picked image (a
  screenshot). No new permission and no new platform code beyond handing over the image bytes. Reading the
  screen directly on Windows (finding a QR code in any window) needs screen capture and a picker of its own
  for what the snipping tool already does, so not that.
- **Camera, Android.** Three ways:
  - **CameraX with ZXing** (Apache-2.0; what Aegis and Keepass2Android use): asks for the camera permission
    when the user first scans; the secret never leaves the app; works offline and on phones without Google
    Play services; adds about a megabyte to the APK.
  - **CameraX with bundled ML Kit**: also inside the app and offline, but a closed-source Google library and
    a bigger model in the APK, for no gain on a QR code shown on a screen.
  - **Google code scanner** (`play-services-code-scanner`): no camera permission and almost no size, but the
    scanning runs in Google Play services, which downloads the module on first use (so the first scan needs
    the network, where the rest of the app works offline), and the decoded secret passes through that
    process.

  The camera is where the secret should not leave the app, so **CameraX with ZXing**.
- **Google Authenticator export** (`otpauth-migration://offline?data=…`): its QR codes carry a protobuf
  (`MigrationPayload`) with many accounts, split over several codes for a large export. Once QR codes are
  read, decoding it is a small parser (secret, name, issuer, algorithm, digits, type); each account becomes a
  new entry, as an import does. Google publishes no schema; the field numbers in use come from community
  reconstructions and match across them.

**Recommendation: do** the image decoder with the editor's *Scan QR code* (Windows clipboard / file, Android
image), then the Android camera; **later** the Google Authenticator export, once scanning is in use.

### 3. Steam Guard

Steam's codes are TOTP (HMAC-SHA-1, 30 seconds) whose truncated value is written as 5 characters of the
alphabet `23456789BCDFGHJKMNPQRTVWXY` instead of decimal digits. KeePassXC writes `encoder=steam` in the URI
(and `S` in `TOTP Settings`); Keepass2Android reads `S`; Aegis has a `steam` type and shows its codes like any
other. Few people have such a secret at all: Steam's own app does not show it, so it comes from tools that
take it out of the app, and how many do is not known. `totp-lite` returns only decimal codes, so the core
needs its own dynamic truncation (`hmac` and `sha1` are already in `Cargo.lock`).

**Recommendation: do, with gap 1:** rare, but it touches the same parser, shares the tests and costs little.
The editor needs no new control: a pasted `otpauth://…&encoder=steam` URI is kept as typed.

### 4. HOTP

Counter-based codes change the database every time a code is shown: the counter goes up and has to be
saved and synced. Between two devices that both show a code before syncing, the merge keeps one counter and
the server may already have moved past it. KeePassXC does not support HOTP at all; Keepass2Android does not
either. Aegis does, but it keeps its vault on one phone: a refresh button shows the next code and saves the
raised counter, with no second device to disagree. HOTP tokens are rare outside hardware tokens and a few
banks.

**Recommendation: no.** Keep the current behaviour: a `hotp` URI or `HmacOtp-*` attributes are saved as they
were, and the entry shows no code. Revisit only if users ask.

### 5. Import from Aegis

[Aegis](https://github.com/beemdevelopment/Aegis) is a popular open-source authenticator for Android. It is
not a way to close the gaps above by itself: it exposes no codes to other apps, it runs only on Android, its
code is GPLv3 (PswManager is MIT, so none of it can be copied in), and keeping the secrets there would split
them from the KDBX file. Moving its tokens into PswManager is what helps, and its export format is documented
(`docs/vault.md` in its repository):

- a JSON object `{version: 1, header, db}`; **plain** when `header.slots` and `header.params` are null
  (`db` is then an object), **encrypted** otherwise (`db` is base64 of AES-256-GCM ciphertext);
- the master key is wrapped in slots; a password slot derives its key with scrypt (`n` = 2^15, `r` 8, `p` 1,
  a 256-bit salt); only password slots apply to an exported file (the others are raw and biometric keys);
- `db` holds `entries` and `groups`; an entry has `type` (`totp`, `hotp`, `steam`, `motp`, `yandex`), `name`,
  `issuer`, `note`, `favorite`, `groups`, an optional icon (`icon` in base64, `icon_mime`, `icon_hash`), and
  `info` with `secret` (base32), `algo`, `digits`, `period` / `counter`, `pin`.

Mapping, through the same `Import` as the other file imports (new entries in a group of their own):

| Aegis | Entry |
| --- | --- |
| `issuer` (or `name` when empty) | title |
| `name` | user name |
| `info` (`totp`, `steam`) | `otp` as an `otpauth://totp/` URI (`encoder=steam` for Steam) |
| `info` (`hotp`) | `otp` as an `otpauth://hotp/` URI, kept for other clients (gap 4: no code shown) |
| `motp`, `yandex` | not imported; the summary names them |
| `note` | notes |
| `favorite` | the `Favorite` tag |
| `groups` (names) | tags |
| `icon` (JPEG or PNG) | the entry's custom icon, as PNG; an SVG icon is left out |

The tokens usually belong to logins already in the database. After the import, the *Similar entries* tool
merges each into its login (an imported entry's `otp` fills the kept entry's empty one). Matching them during
the import instead would guess at issuers and titles, so the import stays an import.

Encrypted exports need `scrypt` and `aes-gcm` (RustCrypto, MIT / Apache-2.0); the import page asks for the
export's password. Plain exports work with neither.

**Recommendation: do** after gap 3 (Steam tokens come over with their codes), plain and encrypted.

## Issues opened

- **#278 TOTP: read KeePass 2, KeeTrayTOTP and KeeOtp layouts, and Steam codes** — gaps 1 and 3: the lookup
  order, the parsers, Steam's encoding, writing back as described; RFC 6238 and Steam test vectors.
- **#279 Scan a TOTP QR code from an image (Windows, Android)** — `rqrr` in the core; the editor's action on
  both apps (clipboard and file on Windows, a picked image on Android).
- **#280 Android: scan a TOTP QR code with the camera** — CameraX and ZXing, the permission asked on first use.
- **#281 Import Google Authenticator's export** (`otpauth-migration://`, later) — needs QR scanning; many accounts from
  one or several QR codes into new entries.
- **#282 Import from an Aegis export (plain and encrypted)** — through the file import, both apps.
- Not opened: HOTP (gap 4).

## Sources

- KeePass, [Placeholders: one-time passwords](https://keepass.info/help/base/placeholders.html#otp)
- KeePassXC, [`src/core/Totp.cpp`](https://github.com/keepassxreboot/keepassxc/blob/develop/src/core/Totp.cpp),
  [`Totp.h`](https://github.com/keepassxreboot/keepassxc/blob/develop/src/core/Totp.h),
  [`Entry.cpp`](https://github.com/keepassxreboot/keepassxc/blob/develop/src/core/Entry.cpp)
- Keepass2Android wiki, [Generating TOTPs](https://github.com/PhilippC/keepass2android/wiki/Generating-TOTPs)
- Aegis, [vault format](https://github.com/beemdevelopment/Aegis/blob/master/docs/vault.md)
- Google, [code scanner](https://developers.google.com/ml-kit/vision/barcode-scanning/code-scanner)
- [`rqrr`](https://docs.rs/crate/rqrr/latest); the `MigrationPayload` schema as reconstructed in
  [GNOME Authenticator](https://world.pages.gitlab.gnome.org/Authenticator/authenticator/backup/google/protobuf/struct.MigrationPayload.html)
