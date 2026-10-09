# Research: Credential Exchange (CXF / CXP)

Issue #140, October 2026. Can PswManager use the FIDO Alliance's **Credential Exchange** standards to move
passwords, passkeys and TOTP secrets in from (and out to) other password managers, and how? No code here; the
end of this note proposes the issues to open.

## The standards

| | What it is | Status (October 2026) |
| --- | --- | --- |
| **CXF** — Credential Exchange Format | The data: a JSON document of accounts, collections, items and their credentials | **Proposed Standard v1.0** (August 2025), errata of 9 March 2026; a final review was due Q3 2026 |
| **CXP** — Credential Exchange Protocol | How two apps hand the data over securely (key agreement, HPKE encryption, an archive for files) | **Working Draft v1.0** of October 2024, not updated since |

So the **format is stable enough to build on; the protocol is not**. The platforms did not wait for CXP:
Apple (iOS / macOS 26) and Google (Android) each ship their **own same-device transfer** that carries CXF
JSON, with the operating system as the trusted go-between instead of CXP's encryption.

CXF itself defines no file and no encryption. It says the exporter must encrypt the data itself or rely on an
orchestrator (the OS) to keep it safe. A "CXF file" is therefore a plain JSON file holding every secret in
clear, unless a tool wraps it in something of its own.

### The data model, in short

- **Header**: version, the exporter's id and name, a timestamp, and **accounts**.
- **Account**: the owner (user name, e-mail), **collections** (nested, holding item references) and **items**.
- **Item**: id, title, subtitle, created / modified times, favorite, tags, **scope** (URLs and Android app
  ids), and a list of **credentials**.
- **Credential types** (17): `basic-auth`, `passkey`, `totp`, `note`, `file`, `custom-fields`,
  `generated-password`, `api-key`, `ssh-key`, `wifi`, `credit-card`, `address`, `person-name`, `passport`,
  `drivers-license`, `identity-document`, `item-reference`. Values are typed *editable fields* (string,
  concealed string, e-mail, number, date, …) with labels.
- **No history**: an item is a snapshot. **Files** are only described (name, size, SHA-256); their bytes
  travel through the protocol (CXP's archive), not inside the JSON.

## Who supports it

| App | Credential Exchange |
| --- | --- |
| Apple Passwords (iOS / macOS 26) | Import and export, app to app on one device (OS transfer); no file |
| Google Password Manager (Android) | Import and export through Android's Credential Transfer (launched September 2026) |
| Bitwarden | iOS 26 and Android (14+ with current Play services), import and export, passkeys included |
| 1Password, Dashlane | Co-authors; support on iOS and Android |
| KeePassXC | Not released: an open PR ([#13343](https://github.com/keepassxreboot/keepassxc/pull/13343)) reads and writes CXF JSON for passkeys (and could do basic-auth, note, ssh-key, TOTP); no protocol, no file handling. Tracked in [#11363](https://github.com/keepassxreboot/keepassxc/issues/11363) |
| Keepass2Android | Nothing found (no passkeys either: [#2099](https://github.com/PhilippC/keepass2android/issues/2099)) |
| SafeInCloud | Nothing found |

The live route today is the **OS transfer on a phone**; file interchange in CXF between desktop managers
does not exist yet in practice.

## Mapping to KDBX

KDBX already holds almost everything CXF carries, partly by convention (KeePassXC's attributes).

| CXF | KDBX (as PswManager reads it) | Notes |
| --- | --- | --- |
| Item: title, times | Title, creation / modification times | |
| `favorite` | the tag `Favorite` (the app's star) | |
| `tags` | Tags | |
| `scope.urls` | URL; more URLs as additional attributes (`KP2A_URL_1`, … as Keepass2Android reads them) | |
| `scope.androidApps` | more `KP2A_URL_n` attributes as `androidapp://<package>`, as Keepass2Android reads them | Never the entry's own URL |
| `basic-auth` | UserName, Password | A second login in one item becomes `UserName (2)`, `Password (2)` |
| `totp` | `otp` (an `otpauth://` URI built from secret, period, digits, algorithm, issuer) | What the app reads already |
| `passkey` | KeePassXC's `KPEX_PASSKEY_*` attributes: credential id, relying party, user name, user handle, private key; a second passkey in one item gets an entry of its own (KeePassXC keeps one per entry) | CXF's key is PKCS#8 DER (base64url); KeePassXC keeps PEM — a re-encoding. PRF / large-blob extensions only with KeePassXC's newer attributes |
| `note` | Notes | |
| `custom-fields`, `api-key`, `wifi`, `credit-card`, `address`, identity documents, `person-name` | additional attributes, the field label as the name, concealed fields protected | Types (date, country, …) become plain text |
| `ssh-key` | an attachment or attributes | KeePassXC's KeeAgent settings are not in CXF |
| `generated-password` | an additional attribute | |
| `file` | attachments | Only with the bytes, which the JSON lacks |
| collections (nested) | groups (nested) | An item in several collections lands in one group |
| `item-reference` | — | Dropped |

**Lost going out (KDBX → CXF):** history (the past versions the app shows), icons, expiry, colours,
auto-type, group notes, the recycle bin, and attachments unless the transfer carries files.
**Lost coming in (CXF → KDBX):** field types, membership of more than one collection, item references.
Neither loss touches what PswManager is used for (logins, TOTP, passkeys, notes).

**A login and a passkey for the same site** (#182). CXF lets an item hold several related credentials, so a
`basic-auth` and a `passkey` in **one item** become one entry: the user name and password in the standard
fields, the passkey in its `KPEX_PASSKEY_*` attributes, as KeePassXC keeps them. The specification lets an
importer split an item, but says nothing about joining separate ones, and how to group them is the
exporter's choice. Google Password Manager sends a site's password and its passkey as **separate items**, so
they come in as two entries: one with the login, one with the passkey. The import keeps them that way.
Every item stays a new entry, and nothing is guessed from matching URLs or user names, which can differ between the two (a passkey's user name is whatever the site gave it). Joining the
two is left to the user, with *Find similar* (#176) or by hand.

## Platforms

### Android: Credential Transfer (Credential Manager)

- Library `androidx.credentials.providerevents` (**1.0.0-beta01**, September 2026; alpha since May 2025),
  backed by Google Play services; on Google-certified devices with Android 8+ according to the guide
  (Bitwarden says Android 14+).
- **Import:** `ProviderEventsManager.importCredentials(activity, ImportCredentialsRequest(types, extensions))`
  shows the system's list of apps that can export; the user picks one and confirms there; the importer gets
  the CXF JSON (`ImportCredentialsResponse.responseJson`) and the exporter's package name. The guide calls
  the importer *a credential provider or a setup wizard*; whether an app that is not an enabled credential
  provider may call it **needs a spike on the phone**.
- **Export:** the app registers what it can export (`registerExport` with `ExportEntry`s: a secret random id,
  names, an icon, the types) and declares an exported activity for
  `androidx.identitycredentials.action.IMPORT_CREDENTIALS`. When another app imports, that activity gets the
  request, checks the id, the caller (Play services) and the destination, **asks the user to confirm
  (fingerprint or PIN)**, and writes the CXF JSON to a content URI. An exporter is a credential provider, so
  export fits the later autofill / credential provider stage, when PswManager has that role anyway.
- Fits the app's structure: the Kotlin side only moves the JSON; mapping it to and from entries is the
  shared core's (Rust), as for everything else.

### Windows

No system transfer and no public API for one as of October 2026. On Windows, Credential Exchange can only
be a **file**, and no manager writes a CXF file yet except KeePassXC's unreleased PR (plain JSON). Not worth
doing now.

### Apple

Out of scope (no iPad / iPhone app).

## Security

- CXF JSON is every secret in clear. Through the Android transfer it never touches storage the user can see:
  the OS hands it over through a content URI, and it should be held in memory only (zeroized), never
  logged, and dropped once merged.
- A **file** export in CXF would be a plain-text dump of the database, passkey private keys included
  (KeePassXC has the same complaint: [#10407](https://github.com/keepassxreboot/keepassxc/issues/10407)).
  PswManager should not write one; the KDBX file already is the encrypted, portable export.
- An import adds entries to an unlocked database and saves it the usual way (atomic save, sync); the
  imported entries are marked so the user can review them (a group or a tag), and nothing is overwritten:
  a CXF item becomes a new entry.
- Export (later) asks for the fingerprint or the master password every time and shows the destination app.

## Recommendation

1. **Wait with files.** No CXF file import or export in either app now: CXP is a stale draft, no manager
   writes CXF files, and a plain-text export is a step back from KDBX.
2. **Import on Android first**, through the system's Credential Transfer: it brings Google Password Manager,
   Bitwarden, 1Password and Dashlane data (passwords, TOTP, passkeys) into the KDBX file, which then syncs to
   Windows. The CXF ↔ entry mapping goes in the core so Windows can use it later. This changes the spec's
   *out of scope* ("importing from other password managers inside the app"), so it is the user's decision.
3. **Export with the credential provider stage** (autofill / passkeys on Android), since an exporter must be
   a credential provider.
4. **Watch:** CXP leaving draft, KeePassXC's PR #13343 (its attribute choices for CXF data), Windows.

## Issues to open

- **Core: CXF to entries (import mapping)** — parse CXF 1.0 JSON into new entries as in the table above
  (logins, URLs, TOTP, passkeys in `KPEX_PASSKEY_*`, notes, custom fields as attributes, collections as
  groups); tests from the spec's examples; secrets zeroized.
- **Android: spike Credential Transfer import** — `providerevents` beta, can a non-provider app call
  `importCredentials`, what Google Password Manager sends; a test export from Bitwarden or Google.
- **Android: import from another app** — a menu item; the system picker; the entries added under an
  *Imported* group and saved; a summary of what came in and what was skipped. Needs the spec change.
- **Later, with the credential provider stage:** export to another app (`registerExport`, the export
  activity, user confirmation, entries to CXF).

## Sources

- FIDO Alliance, [Credential Exchange specifications](https://fidoalliance.org/download-credential-exchange-specifications/)
  ([CXF 1.0 PS with errata](https://fidoalliance.org/specs/cx/cxf-v1.0-ps-errata-20260309.html),
  [CXP 1.0 WD](https://fidoalliance.org/specs/cx/cxp-v1.0-wd-20241003.html))
- Android Developers, [Credential transfer](https://developer.android.com/identity/sign-in/credential-transfer)
  and [providerevents releases](https://developer.android.com/jetpack/androidx/releases/credentials-providerevents)
- Bitwarden, [Your passkeys can now move freely](https://bitwarden.com/resources/july-2026-spotlight-your-passkeys-can-now-move-freely-heres-how/)
- KeePassXC [#11363](https://github.com/keepassxreboot/keepassxc/issues/11363),
  [#13343](https://github.com/keepassxreboot/keepassxc/pull/13343),
  [#10407](https://github.com/keepassxreboot/keepassxc/issues/10407);
  Keepass2Android [#2099](https://github.com/PhilippC/keepass2android/issues/2099)
