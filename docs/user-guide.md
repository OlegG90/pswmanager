# PswManager — User guide

PswManager keeps your passwords in one encrypted file: a KeePass database (`.kdbx`). The same file also
opens in KeePassXC (Windows, macOS, Linux) and Keepass2Android, so you are never locked in.

This guide says, in plain words, what you can do with the app and what to do when something outside it —
a browser, a website, another app, a cloud store — asks it for something.

- [Part 1 — Windows](#part-1--windows)
- [Part 2 — Android](#part-2--android)
- [When something asks PswManager for something](#when-something-asks-pswmanager-for-something) — a
  quick table for both apps

---

# Part 1 — Windows

## 1. Install and start

1. From the [latest release](https://github.com/OlegG90/pswmanager/releases/latest), download
   `pswm-x64.exe` (most PCs) or `pswm-arm64.exe` (Windows on ARM, e.g. Snapdragon laptops). There is no
   installer: put the file in any folder you like and, if you want, rename it `pswm.exe`.
2. The first time, Windows SmartScreen warns that the app is not signed. Choose **More info → Run
   anyway**.
3. The app lives in the **tray** (the notification area by the clock). Click its icon, or press
   **Ctrl+Alt+P** anywhere, to show or hide the window. Closing the window only hides it; **Quit** is in
   the tray icon's menu.

## 2. Your database

The first screen asks which database to use:

| Choice | When to use it |
|---|---|
| **Create a new database** | You are starting from scratch. Pick where the file goes and set a master password (typed twice). |
| **Open a local file** | You already have a `.kdbx` file on this PC or a USB drive (from KeePassXC, say). |
| **Open from a folder** | Your database is in a shared network folder (a NAS). |
| **Open from Dropbox / Google Drive / OneDrive** | Your database is in that cloud store. Sign in, then pick the file. |

A database opened from a folder or a cloud store is copied to a file on this PC, and the app keeps the two
in step from then on (see *Sync with your phone and other PCs*).

**The master password** must have at least 6 characters, upper- and lower-case letters and one character
that is not a letter or a digit (a space counts). The rules are listed under the field and ticked as you
type. A strength meter shows how hard the password is to guess.

**Remember the master password.** Nobody — not the app, not its author — can open the database without it.

You can keep **several databases** in the list; one is open at a time. The unlock screen shows which one
it will open and lets you switch. *Remove from the list* only forgets a database: the file stays where it
is.

## 3. Unlock and lock

- **Unlock** with the master password, a **key file**, or both (a key file is an extra file that acts as
  part of the key; keep it apart from the database).
- **Windows Hello** (face, fingerprint or PIN): after you unlock once with the master password, the next
  unlocks can use Windows Hello. Every 14 days the app asks for the master password again, so you don't
  forget it. Both are in *Settings → General*: *Unlock with Windows Hello* and how often the master
  password is asked (1 to 90 days).
- **Lock** with **Ctrl+L**, the lock button or the tray menu. The app also locks by itself:
  - after 5 minutes without use (change it in *Settings → General*);
  - when you lock Windows or switch user;
  - when you hide the window, if you turn on *Lock when hidden*.

Locked, the app forgets everything it decrypted; only the encrypted file is left.

## 4. The window

| Part | What it does |
|---|---|
| **Search field** (top) | Just start typing. It searches titles, user names, web addresses, tags and notes — never passwords. |
| **Sidebar** (left) | **Records** (groups and tags), **Tools** and **Settings**. Click a heading to fold or unfold it. Drag the sidebar's edge to make it wider. |
| **List** (middle) | The entries in the group or tag you chose. ↑ / ↓ move through it. |
| **Entry** (right) | The chosen entry's details. |

**Groups** are made for you; you never put an entry into one by hand:

| Group | Shows |
|---|---|
| All | every entry (not the deleted ones or the templates) |
| Favorites | entries you marked with the star |
| Expired | entries whose expiry date has passed or comes within 14 days |
| 2FA | entries with a one-time code (TOTP) |
| Passkey | entries with a passkey |
| Templates | patterns for new entries (credit card, bank account, Wi-Fi…) |
| Trash | deleted entries, until you empty it |

**Tags** are your own way of sorting: `#work`, `#bank`, anything. **Untagged** shows the entries that have
none. A tag's **⋯** menu in the sidebar renames or removes it in every entry.

## 5. Find and copy a password

1. Press **Ctrl+Alt+P** — the window comes up with the search field ready.
2. Type a few letters of the site or account.
3. Copy what you need:

| Key | Copies / does |
|---|---|
| **Ctrl+B** | user name |
| **Ctrl+C** | password |
| **Ctrl+T** | the current one-time (2FA) code |
| **Ctrl+U** | opens the web address in your browser |
| **Ctrl+H** | shows or hides the password |
| **Esc** | clears the search, then hides the window |
| **F1** | lists every shortcut |

You can also click a value, or the copy icon at the end of its line.

**The clipboard is cleared after 20 seconds** (if it still holds what you copied), and copied passwords
stay out of Windows' clipboard history and cloud clipboard.

## 6. Add and change entries

- **New entry: Ctrl+N.** Choose a blank entry or a template (a credit card, a bank account…). **Edit:
  Ctrl+E.** Nothing is saved until you press **Save**; **Cancel** drops the changes.
- **Password generator** (in the editor): length and which kinds of characters; by default 20 characters
  without look-alikes such as `l` and `1`.
- **Tags**: type a tag and press Enter or a comma. The star makes an entry a favorite.
- **Expiry date**: the entry then shows up in *Expired* when it is time to change it.
- **One-time codes (2FA)**: paste the secret the site gives you, or use the **scan** button: take a
  snip of the site's QR code with **Win+Shift+S** first (or pick a picture of it). The entry then shows
  the current code with a countdown.
- **Extra fields**: add your own (a PIN, a security question…); the padlock hides a field like a
  password.
- **Files**: attach a file (up to 20 MB) with the paperclip. In the entry, **open** shows it in its usual
  app and **save** writes a copy where you choose. Programs and scripts are never opened, only saved.
- **Icon**: by default the site's own icon; you can pick one of the app's icons or your own picture.

Every change is saved at once, and the entry's previous version is kept in its **History**.

## 7. Work with many entries, the trash and history

- **Several entries at once**: Ctrl+click or Shift+click in the list, then add or remove a tag, star them
  or delete them all together.
- **Right-click an entry** for *Auto-Type*, *Duplicate*, tags, *Delete*, and — with two chosen —
  *Merge*.
- **Delete** moves an entry to the **Trash**. There you can **Restore** it, **Delete permanently**, or
  **Empty trash**. Only this last step removes anything for good.
- **History (N)** in an entry lists its older versions, with what changed (never the values themselves).
  Open one to look at it or copy from it; **Restore this version** brings it back.

## 8. Fill in logins in browsers and other apps

Windows has no built-in autofill for password managers, so PswManager offers two ways.

### Auto-Type — any app, any browser

1. Click into the login form (the user name field).
2. Press **Ctrl+Alt+A**.
3. If one entry fits, it is typed at once: user name, Tab, password, Enter. If several fit, a small
   window lists them: choose one and press **Enter** (or **User name**, **Password**, **TOTP** to type only
   that). To be asked even when one entry fits, turn on *Always ask* in *Settings → General → Auto-Type*.

Which entries fit:

- **In a browser**: the entries whose web address matches the address in the browser's address bar. The
  app reads the real address, never the page's title, so a fake page titled like your bank gets nothing.
- **In another app**: the entries whose title is part of the window's title.

Good to know:

- From inside PswManager, choose an entry and press **Ctrl+Shift+V**: the app hides and types it into the
  window you were using before.
- If the app was locked, the chooser asks you to unlock first.
- Typing stops if you switch window or the browser goes to another page meanwhile.
- A program running *as administrator* cannot be typed into; the app tells you so.
- On keyboard layouts that type letters with **AltGr** (Polish `ą` is AltGr+A), Windows reads
  **Ctrl+Alt** as AltGr: pick another hotkey in *Settings → General → Auto-Type*.
- You can change what is typed for an entry (its *sequence*) in the editor's *Auto-Type* part.

### The KeePassXC-Browser extension — Chrome, Edge, Firefox, Brave, Vivaldi

PswManager speaks the same language as KeePassXC, so it works with KeePassXC's free browser extension.

**Set it up once:**

1. In PswManager: *Settings → General → Browser*, turn on **KeePassXC-Browser extension**.
2. Install **KeePassXC-Browser** from your browser's extension store.
3. Click the extension's icon and choose **Connect**.
4. PswManager asks: *A browser extension asks to connect to "your database"*. Give the connection a name
   (e.g. "Chrome at home") and confirm.

Only PswManager *or* KeePassXC can serve the extension at a time. If KeePassXC starts with its own browser
integration on, it takes the extension over; the setting tells you when that happened.

**Then, on a login page**, the extension fills in the matching entries by itself, or shows them in a
small list beside the field. If the database is locked, the extension says so; click it to bring up
PswManager's unlock.

## 9. Questions from the browser extension

These questions come up in PswManager's window. **Each waits two minutes**; after that it closes as
"cancelled".

| The question | What it means | What to do |
|---|---|---|
| *A browser extension asks to connect to "your database"* | An extension wants access to this database's logins. | Accept only if **you** just clicked *Connect* in your own browser. Otherwise **Cancel**. |
| A login was saved (the window shows the entry) | You saved a new login through the extension. The entry is titled after the site. | Check the entry; edit it if you want a better title or tags. |
| *Replace the password of "an entry" with the one the site offers?* | You logged in with a new password, and the extension wants to update an existing entry. | **Replace** if you really changed the password on that site. The old one stays in the entry's history. |
| *Sign in to the site with a passkey:* | The site asks for a passkey, and the database has one for it. | Choose the passkey. Choosing it is your confirmation. |
| *Save a passkey for your account on the site in:* | The site wants to create a passkey for you. | Choose **A new entry** or one of the site's entries. |

**Passkeys** work through the extension only after you turn on its own *Passkeys* option (it is off by
default). If the database has no passkey for a site, the extension hands the request to the browser's own
passkeys (Windows Hello, a phone or a security key), as long as its fallback option is on (it is by
default).

**Removing a connection**: *Settings → Database* lists every connected browser extension and when it
connected. **Remove…** cuts it off; it has to connect again.

## 10. Other requests

| Situation | What happens | What to do |
|---|---|---|
| **Signing in to Dropbox, OneDrive or Google Drive** | Your browser opens the store's own sign-in page. PswManager never sees your cloud password; it gets access only to its own folder there. | Sign in on the real page of the store and allow access. You can close the browser tab afterwards. |
| *Sign in to Dropbox again* (in the sync status) | The store's sign-in ran out. Your changes wait on this PC. | Click it and sign in again. |
| **The key changed on another device** | You changed the master password on the phone (or in another app), and the PC still knows the old one. | Type the new master password when asked. *Not now* lets you go on; your changes wait until you give the key. |
| **Site icons** | The app downloads each site's icon, like a browser does, only over https and only by the site's name — never your user name or anything else from the entry. | Nothing. To stop it, turn off *Settings → Window → Download site icons*. |
| **Check for updates** (*Settings → About*) | Only when you press it, the app asks GitHub for the latest version. It downloads nothing by itself. | If a new version is out, **Open the release**, download it and replace your old `.exe`. |
| **Opening a `.kdbx` file** (double-click, or `pswm file.kdbx`) | The running app opens it if it is locked; otherwise it says why not. | Lock the current database first. |

## 11. Sync with your phone and other PCs

To use the same database on several devices, sync it with a store: Dropbox, OneDrive, Google Drive, or a
network folder.

- **Set it up**: *Settings → Sync* → **Upload…** (put this database into a store as a new file) or
  **Link…** (sync with a file already in a store). If both files differ, the app asks: **Merge both**
  (usually the right choice), **Use the remote file** or **Keep this file**.
- **When it syncs**: when you unlock, about 10 seconds after a change, when you hide the window, when you
  lock or quit, every 5 minutes while unlocked (*Check for remote changes* in *Settings → Sync*), and when
  you press **Sync now** (in the window or the tray menu).
- **The status** next to the Sync button says what happened: *Synced 12:04*, *Syncing…*, *Changes waiting —
  offline*, *Merged 3 entries from Dropbox*…
- **Offline** is fine: you keep working, and the changes go up the next time.
- **Conflicts**: if you edited on two devices, the newer change of each entry wins and the other goes into
  the entry's history, so nothing is lost.
- **Stop syncing** keeps the database as a plain file on this PC; the file in the store is left alone.

The app also keeps a `.bak` copy of the file beside it before each save, and a `.remote.bak` before each
merge.

## 12. Tools

All in the sidebar under **Tools**.

- **Password health**: passwords used more than once, weak ones and ones not changed for over a year.
  Click one to change it.
- **Similar entries**: finds duplicates (the same site and user name, say) and merges them into one. You
  see the result before you press **Merge**; the others go to the Trash.
- **Import from a file**: a CSV or JSON exported from another password manager (Google, Chrome, Edge,
  Firefox, Bitwarden, 1Password, KeePass, KeePassXC, Aegis). New entries get a tag such as *Imported from
  export.csv 2026-11-06*. Such a file holds every password in plain text: **delete it** afterwards — the app
  offers to.
- **Import from Google Authenticator**: in Google Authenticator choose *Transfer accounts → Export
  accounts*, then snip each QR code with **Win+Shift+S** (or save it as a picture) and let the app read
  it.
- **Export to a file**: CSV (for browsers, Bitwarden, 1Password) or CXF JSON. The app asks you to confirm
  it is you (Windows Hello or the master password) first; that holds for 5 minutes. It never writes over
  an existing file. Delete
  the file once you have imported it elsewhere.

## 13. Settings

*Settings* (**Ctrl+,** or the sidebar) has these tabs:

| Tab | What you set there |
|---|---|
| **General** | when to lock, Windows Hello, how long copied values stay, Auto-Type, the browser extension |
| **Window** | light or dark theme, the show/hide hotkey, start with Windows, site icons |
| **Database** | the database's name and description, the default user name, how much history to keep, **change the master password or key file**, the encryption, connected browser extensions |
| **Sync** | *Upload…* / *Link…* or *Stop syncing*, how often to check for remote changes |
| **Backup** | a backup folder and how often (every 2, 7 or 30 days); *Backup now* |
| **About** | the version, *Check for updates*, the source code on GitHub |

What is on the **Database** tab is kept inside the database file, so KeePassXC and the phone see it too.
Everything else belongs to this PC only.

**Changing the master password**: other devices need the new password from then on. Old backups and the
store's version history still open with the old one.

## 14. Problems and answers

| Problem | Answer |
|---|---|
| Ctrl+Alt+P does nothing | Another app took that key; the unlock screen says so. Choose another one in *Settings → Window*. |
| Auto-Type typed nothing | The window in front is not the one you meant, it runs as administrator, or no entry fits its address or title. Use Ctrl+Shift+V from the entry instead. |
| The extension says "not connected" | KeePassXC may have taken it over. Turn the setting in *Settings → General → Browser* off and on, then connect again. |
| *Wrong password or key file* | Check Caps Lock and the keyboard layout. A database made with a key file needs that file too. |
| The sync status shows an error | You can keep working; the changes wait. Press **Sync now** later, or sign in to the store again. |
| The new `.exe` cannot replace the old one | The old one is still running: choose **Quit** in the tray icon's menu first. |

---

# Part 2 — Android

The phone app opens **the same database** as the PC, through the same store, and follows the same rules
for groups, tags, history and sync. This part covers what is different on the phone. Not on the phone yet:
several databases, backups, password health, renaming or removing a tag everywhere, and restoring an
older version of an entry.

## 1. Install

1. From the [latest release](https://github.com/OlegG90/pswmanager/releases/latest), download
   `pswmanager-android.apk` on the phone.
2. Open it. Android asks to allow installing apps from your browser or file manager; allow it for this
   install. There is no Google Play version, and updates are installed the same way, over the old app.

Android 10 or newer is needed.

## 2. First run

Choose your database:

| Choice | When to use it |
|---|---|
| **Sync with Dropbox / OneDrive / Google Drive** | Your database is already there (e.g. put there by PswManager on the PC). Sign in, then pick the file. |
| **Open a local file** | A `.kdbx` on the phone or on an SD card, or in a folder another app syncs. |
| **Create a new database** | Start from scratch: a name and a master password, then keep it in a store or on the phone. |

The app sees only **its own folder** in the store: `Apps/PswManager Sync` in Dropbox, `Apps/PswManager` in
OneDrive, `PswManager` in Google Drive. To share a database between the PC and the phone, let the PC put it
there (*Settings → Sync → Upload…*).

For a database in a store, the app also asks for a **folder on the phone** for a visible copy. That copy
always holds the latest state, so you can back it up; the app never reads it back.

The phone has **one database** at a time. *Use another database…* on the unlock screen switches.

## 3. Unlock and lock

- The first unlock is with the master password (the eye shows what you typed) and/or the key file.
- After that, **the fingerprint or face** unlocks it. Every 14 days the app asks for the master password
  again.
- It locks by itself **30 seconds after you leave the app**, when the screen turns off, and after 5
  minutes without use. The times are in *Settings → General*.

## 4. The list and finding an entry

- **☰** or a **swipe to the right** opens the drawer with the groups (All, Favorites, Expired, 2FA,
  Passkey, Templates, Trash) and your tags.
- A **swipe to the left** opens Settings. (You can turn swipes off in *Settings → Appearance*.)
- The round **search** button at the bottom right searches as on the PC; **+** below it adds an entry.
- **Pull down** the list to sync. The line at the bottom shows the sync status; tap it for details.
- The toolbar's **wrench** opens **Tools**.

## 5. An entry

- Tap a value, or its **copy** icon, to copy it. A message says when the clipboard will be cleared (after
  20 seconds). Copied passwords are hidden from the keyboard's clipboard suggestions.
- The **eye** shows a hidden value; **open** opens a web address in the browser or a file in another app.
- The toolbar has the **star** (favorite), the **pencil** (edit) and **⋮ → Delete**.
- **History (N)** lists older versions. On the phone you can look at them and copy from them, but restore
  one only on the PC.

## 6. Editing

As on the PC: title, user name, password with the **generator**, web address, notes, tags, favorite,
expiry date, one-time code secret, extra fields and files. **✓** saves, **←** cancels (it asks before
dropping changes).

For a one-time code, the **scan** button reads the site's QR code with the **camera** (Android asks once
for camera access) or from a screenshot.

**Several entries**: a **long press** chooses an entry; tap others to add them. Then copy, tag or delete
them together.

## 7. Tools

- **Import from another app…**: Android's own transfer between password managers (Google Password Manager,
  Bitwarden, 1Password…). Pick the app and confirm there; everything comes over as new entries.
- **Import from a file…** and **Export to a file…**: as on the PC. Delete such files afterwards: they
  hold every password in plain text.
- **Import from Google Authenticator…**: point the camera at each export QR code, or pick screenshots.
- **Find similar entries**: as on the PC.

## 8. Filling in logins in other apps and browsers

### Autofill (apps and websites)

**Set it up once:** *Settings → General → Autofill* → open Android's choice and pick **PswManager**. In
**Chrome**, also turn on *Settings → Autofill services → Autofill using another service*.

**Then** tap a login field in any app or website:

- **Locked**: the only suggestion is **Unlock PswManager**. Tap it, unlock with your fingerprint, and the
  suggestions appear.
- **Unlocked**: the matching entries are offered by title and user name. Tap one to fill both fields.
- **Nothing matches?** Tap **Choose an entry in PswManager…**, search, and pick the entry. It is then
  **linked** to that app or site and offered there by itself next time.

**Saving**: after you type a new password into a login form, Android asks **Save to PswManager?**. Tap
*Save* and choose **a new entry**, **update** an existing one (the old password goes to its history), or
**another entry**. A short message says where it went.

### Passkeys and Credential Manager (Android 14 and newer)

Turn it on in Android's **Settings → Passwords, passkeys & accounts**: add PswManager and, best, choose it
as the *preferred service* (Android offers Google first).

- **Signing in with a passkey**: the site or app asks; pick your passkey from PswManager's list and confirm
  with the fingerprint or the screen lock.
- **Creating a passkey**: choose **PswManager**, then **a new entry** or an existing entry for the site.
  The passkey then also works on the PC (through the browser extension) and in KeePassXC.
- Apps that ask Credential Manager for a password get PswManager's logins the same way.

## 9. When another app asks PswManager on the phone

| The question | What it means | What to do |
|---|---|---|
| **Unlock PswManager** (in a login field or Android's sheet) | The database is locked; nothing about your entries has been shown. | Tap it and unlock. |
| ***Trust this app?*** (with the app's certificate) | An app wants a login from an entry that does not know this app's certificate yet (one made in Keepass2Android, say), or you are linking an entry to the app. Any app can call itself anything; the certificate shows who really made it. | Trust it only if it is the app you expect — e.g. you installed it from Google Play yourself. Once trusted, the entry remembers it, and a fake app with the same name gets nothing. |
| **Save to PswManager?** | You typed a new password into a form. | *Save*, then choose where it goes. Or dismiss it. |
| **Creating a passkey** (PswManager asks where it goes) | A site or app wants to create a passkey. | A new entry, or an existing one for that site. |
| **The export screen**, naming another app | Another password manager asks Android to import your database from PswManager. | Only if **you** started that import. You confirm with your fingerprint or the master password **every time**. Nothing in your database changes. |
| **The new key** screen | The master password was changed on another device. | Type the new one. *Not now* lets you keep working; changes wait. |
| **Camera access** | You chose to scan a QR code. | Allow it, or pick a screenshot instead. |
| **All files access** | Android's pickers hide some folders (one named *Data*, say), and you tried to choose something there. | Optional. Turn it on only if you need such a folder. |

## 10. Settings on the phone

| Tab | What you set there |
|---|---|
| **General** | locking, **screenshots** (blocked by default), the clipboard, the fingerprint, **Autofill** |
| **Appearance** | theme, site icons, swipes on the list |
| **Database** | the same as on the PC: name, history, master password and key file, encryption |
| **Sync** | the store, *Sync now*, *Stop syncing*, sync with a store later |
| **About** | the version and the source code |

**Screenshots** of the app are blocked unless you turn on *Allow screenshots*: screenshots show nothing,
and the app's preview in the recent apps is blank.

## 11. Problems and answers

| Problem | Answer |
|---|---|
| No suggestions in a login field | Check that PswManager is the autofill service. In Chrome, turn on *Autofill using another service*. Some apps block autofill; copy and paste then. |
| Nothing is offered on a plain `http://` page | On purpose: unencrypted pages get nothing. |
| A fingerprint stopped working | A new fingerprint or face was added to the phone, so the stored key was cleared. Unlock once with the master password. |
| Changes don't reach the PC | Open the app or pull down to sync. Android stops apps in the background, so the phone syncs when you use it. |

---

# When something asks PswManager for something

One rule for both apps: **say yes only to a request you started yourself.** PswManager never asks for your
master password by e-mail, in a browser page or in another app — only in its own window or its own unlock
screen.

| Request | Windows | Android | Safe to accept when… |
|---|---|---|---|
| Connect a browser extension | ✓ | — | you just clicked *Connect* in your own browser |
| Fill in a login | Auto-Type, the extension | Autofill, Credential Manager | the site's address (or the app) is the one you meant |
| Save or update a login | the extension | *Save to PswManager?* | you just typed that password yourself |
| Sign in with a passkey | the extension | Credential Manager | you are signing in to that site right now |
| Create a passkey | the extension | Credential Manager | you are setting one up on that site right now |
| Trust an app | — | *Trust this app?* | it is the real app, from a source you trust |
| Export to another app | — | the export screen | you started the import in that other app |
| Sign in to a cloud store | browser | browser | the page is the store's real one |
| Give a new master password | after a change elsewhere | after a change elsewhere | you changed it yourself on another device |

When unsure, **Cancel**. Nothing breaks: the request just fails, and you can try again.
