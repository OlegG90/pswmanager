# PswManager — User guide

PswManager keeps your passwords in one encrypted file: a KeePass database (`.kdbx`, KDBX 4). It is an
open, standard format, so you are never locked in: the same file also opens in other KeePass apps.

| Platform | Apps that open the same file |
|---|---|
| Windows (also macOS and Linux) | [KeePassXC](https://keepassxc.org/) |
| Android | [KeePassDX](https://www.keepassdx.com/), [Keepass2Android](https://github.com/PhilippC/keepass2android) |
| iOS and iPadOS | [KeePassium](https://keepassium.com/) |

Use one of them where PswManager has no app (an iPhone, a Mac), or alongside it. Put the file in a cloud
store they all reach, and changes made in any of them are merged when PswManager syncs.

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

PswManager for Android opens **the same database** as the PC, through the same store, and keeps the same
groups, tags, history and sync rules, so an entry changed on one device shows up on the other. This part
describes the phone app on its own; you don't need to read Part 1 first.

Not on the phone yet: several databases in the list, backups, password health, renaming or removing a tag
in every entry, restoring an older version of an entry, and a network folder (a NAS) as a store. Use the PC
for those. The app works in portrait only.

## 1. Install and update

1. On the phone, open the [latest release](https://github.com/OlegG90/pswmanager/releases/latest) and
   download `pswmanager-android.apk`.
2. Open the downloaded file. Android asks to allow installing apps from your browser or file manager:
   allow it for this install.
3. **Updates** are installed the same way, over the old app; your database and settings stay.

The app needs Android 10 or newer. It is not on Google Play.

## 2. First run: choose your database

| Choice | When to use it |
|---|---|
| **Sync with Dropbox / OneDrive / Google Drive** | Your database is already in that store (e.g. the PC put it there). Sign in in the browser, then pick the file. |
| **Open a local file** | A `.kdbx` on the phone, on an SD card, or in a folder another app syncs (Syncthing, say). |
| **Create a new database** | You are starting from scratch. |

**A cloud store.** The app sees only **its own folder** there: `Apps/PswManager Sync` in Dropbox,
`Apps/PswManager` in OneDrive, `PswManager` in Google Drive. To use the PC's database on the phone, upload
it from the PC first (*Settings → Sync → Upload…* on Windows). After you pick the file, the app asks
**where to keep it on this phone**: a folder for a **visible copy** (see below) and a file name. The file
is downloaded and sync is on from the start.

**A local file.** The file you pick is kept in step with the app's own copy: changes made to it by
another app (Keepass2Android, Syncthing…) are merged in, and yours are written back to it.

**A new database.** Give it a name and a master password (typed twice; the eye shows what you typed). The
password needs at least 6 characters, upper- and lower-case letters and one character that is not a
letter or a digit; the rules are ticked as you type. Then choose where it lives: in a cloud store (with a
visible copy on the phone) or as a file in a folder on the phone. A file that is already there is never
replaced. A new database comes with **starter templates** (credit card, bank account, ID document, e-mail
account, Wi-Fi).

**Remember the master password.** Nobody can open the database without it.

### The visible copy

For a database in a cloud store, the phone keeps a plain `.kdbx` copy in a folder you chose, updated after
every change, also while you are offline. It is **for you**: back it up, or open it in another app if
you need to. PswManager never reads it back, and it doesn't overwrite it if another app changed it.

### One database at a time

To switch, lock the app and choose **Use another database…** on the unlock screen. The app forgets the
old database and signs out of its store; the old database's file stays where it is.

## 3. Unlock and lock

- **The first unlock** is with the master password (the eye shows what you typed) and/or the **key
  file**, if your database uses one: *Use a key file…* on the unlock screen picks it, *Remove* lets it go.
- **Fingerprint or face**: with *Unlock with fingerprint* on (the default), that first unlock sets it up.
  From then on, a fingerprint button sits beside the password field; tap it, or tap *Unlock* with the
  field empty. When you come back from another app, the prompt opens by itself. *Use the master password*
  in the prompt takes you back to the field.
- **The master password is asked again every 14 days**, so you don't forget it, and also when you add a
  new fingerprint or face to the phone, or when the password was changed on another device.
- **Lock by hand** with the lock button in the toolbar.

**It locks by itself:**

| When | Default | Setting (*Settings → General → Locking*) |
|---|---|---|
| after you leave the app | 30 seconds | *In the background*: at once, 30 s, 1 min, 5 min or never |
| when the screen turns off | at once | *When the screen turns off* |
| without a touch while the app is open | 5 minutes | *Without a touch* |

Unlocked from a login field or a passkey request (see *Fill in logins*), it stays unlocked for at least a
minute, so the login can go through.

Locked, the app forgets everything it decrypted; only the encrypted file is left.

## 4. The main screen

| Where | What it does |
|---|---|
| **Toolbar** | **☰** (groups and tags), the name of what is shown, **Sync now**, the **wrench** (*Tools*), **⚙** (*Settings*), **Lock** |
| **List** | the entries, each with its icon; tap one to open it |
| **Round buttons**, bottom right | **search**, and **+** under it for a new entry |
| **Bottom line** | the sync status; tap it for details |

- **Swipe to the right** on the list for the groups and tags, **to the left** for Settings. Swipe back
  to close them. (Turn swipes off in *Settings → Appearance → Swipes on the list*.)
- **Search** looks in titles, user names, web addresses, tags and notes — never in passwords — within the
  group or tag shown. Back or ← closes it.
- **Pull down** the list to sync.

### Groups and tags (☰)

| Group | Shows |
|---|---|
| All | every entry (not the deleted ones or the templates) |
| Favorites | entries you marked with the star |
| Expired | entries whose expiry date has passed or comes within 14 days |
| 2FA | entries with a one-time code (TOTP) |
| Passkey | entries with a passkey |
| Templates | patterns for new entries |
| Trash | deleted entries |

Under the groups: **Untagged** (entries with no tags) and your **tags**, each with how many entries it
has. The app's version is at the bottom.

## 5. Look at an entry and copy from it

An entry shows its title, tags, user name, password (hidden), the one-time code with its countdown, web
address, notes, extra fields, files and when it was last changed. Each line ends in icons:

- **copy** — or just tap the value. A short message says it was copied and when the clipboard will be
  cleared (after **20 seconds**, if it still holds what you copied). Copied passwords are marked as
  sensitive, so the keyboard's clipboard suggestions don't show them.
- **eye** — shows a hidden value; tap again to hide it.
- **open** — opens a web address in the browser, or a file in another app (Android asks which one).
  Programs and scripts are never opened.
- **save** (by a file) — writes a copy of the file where you choose.

The toolbar has the **star** (favorite), the **pencil** (edit) and **⋮ → Delete** (to the Trash, after
asking). **Back** returns to the list where you were.

**History (N)** lists the entry's older versions, newest first, with what changed in each (never the
values). Open one to look at it and copy from it. Restoring an old version is done on the PC.

## 6. Add and edit entries

- **+** asks for a **Blank entry** or one of your **templates** (with no templates, it goes straight to a
  blank entry). A blank entry gets the database's default user name, if one is set.
- In the editor you set: title, user name, password, web address, notes, **tags** (type one, or pick one
  you already use), favorite, **expiry date**, **one-time code** secret, **extra fields** (the padlock hides
  a field like a password; the eye shows it) and **files**.
- **Password generator**: by the password field; length and kinds of characters, 20 characters without
  look-alikes by default. A strength meter shows how good the password is.
- **One-time code**: paste the site's secret, or use the **scan** button: **the camera** (Android asks
  for camera access the first time) or a **screenshot** of the site's QR code.
- **Files**: add, rename or remove them; nothing changes until you save.
- **✓** saves and shows the entry. **←** or Back cancels; it asks before dropping changes.

Every change is saved at once and goes up with the next sync. The entry's previous version is kept in
its history (a change of tags or the star alone keeps none).

### Several entries at once

**Long-press** an entry to choose it, then tap others to add them (as in Gmail). The bar on top says how
many and offers:

- **copy** (one entry chosen): *Copy user name*, *Copy password*, *Copy TOTP code*;
- **tag**: add a tag to all of them, or take one off;
- **bin**: move all of them to the Trash, the recycle bin (after asking).

**✕** or Back lets them go.

### Templates

Open **Templates** from ☰. A template opens like an entry, with **New entry from it** under it; the pencil
edits it, **⋮ → Delete** deletes it. **+** there makes a new template. A database with no templates offers
**Add starter templates**. An entry made from a template gets its fields, icon and tags, but not its
title, expiry or one-time code.

### The Trash

**Trash** in ☰ lists deleted entries (the database's recycle bin). Open one to **Restore** it (back where it was) or **Delete
permanently…**. **Empty the recycle bin…** above the list removes all of them for good, after saying how
many. Only these last two steps delete anything for good, on every device.

## 7. Tools (the wrench)

- **Import from another app…** — Android's own transfer between password managers. Android lists the
  apps on the phone that can hand over their passwords (Google Password Manager, Bitwarden, 1Password…);
  pick one and confirm there. Everything comes over as **new entries** with the tag *Imported from* the
  app and the date; nothing you have is changed. A sheet then says what came in and what did not.
- **Import from a file…** — a CSV or JSON file another password manager exported (Google, Chrome,
  Edge, Firefox, Bitwarden, 1Password, KeePass, KeePassXC, Aegis). An encrypted Aegis file asks for its password.
  Such a file holds every password in plain text: **delete it** afterwards. The app offers *Delete the
  file* when it can, otherwise it reminds you.
- **Import from Google Authenticator…** — in Google Authenticator choose *Transfer accounts → Export
  accounts*; then scan each QR code it shows with the camera (from another phone's screen), or pick
  screenshots of them.
- **Export to a file…** — a CSV or CXF JSON file for another password manager. The app asks for your
  fingerprint or master password first and never writes over an existing file. Delete the file once it
  is imported elsewhere.
- **Find similar entries** — finds entries for the same site and user name (or the same title) and merges
  them: tick what counts as similar, choose the entry to keep, **Proceed** shows the result, **Merge**
  does it (the others go to the Trash). **Leave as is** hides a group until you open the tool again.
- **Tidy up imported folders** — only shown when an older version of the app put imported entries in
  folders; it turns those folders into tags.
- **Export to another app** is not in Tools: you start it **in the other password manager**, which asks
  Android to import from PswManager (this needs Google Play services on the phone). Android then opens
  PswManager's export screen, which names the app the entries go to and asks for your fingerprint or master
  password **every time**. Nothing in your database changes.

## 8. Sync

A database in a cloud store or a local file syncs **when you use the app**: Android stops apps in the
background, so the phone does not sync on its own while it is closed.

| When | What happens |
|---|---|
| You unlock | the app shows your entries at once and syncs; the list updates when it is done |
| You come back to the app (more than a minute since the last sync) | it syncs |
| About 10 seconds after a change | the change goes up (a change of tags or the star waits for the next sync) |
| You leave the app with changes not sent | they go up at once; if Android stops the app, later, as soon as there is a network |
| Every 5 minutes while the app is open | the app checks for changes (*Settings → Sync → Check for changes*) |
| You pull the list down or tap **Sync now** | it syncs |

- **The status line** at the bottom: *Synced 12:04*, *Syncing…*, *Changes waiting — offline*, *Merged 3
  entries from Dropbox*, *Sign in to Dropbox again*… Tap it for the **sync sheet**: the last result, the
  file in the store, the entries the last merge touched, and *Sync now*.
- **Offline** is fine: you keep working, and the changes go up later.
- **Conflicts**: if you edited the same entry on two devices, the newer change wins and the other goes
  into the entry's history, so nothing is lost.
- **Settings → Sync** shows where the database syncs and the visible copy's folder, and offers:
  - **Stop syncing…** — e.g. when the store can't be reached. The visible copy becomes the database's
    file, so nothing is lost; the file in the store is left alone.
  - **Sync with Dropbox… / OneDrive… / Google Drive…** — for a database that syncs with no store: merge
    it with a file in the store, or upload it there as a new one.

## 9. Fill in logins in other apps and websites

### Autofill

**Set it up once:**

1. *Settings → General → Autofill* says whether PswManager fills in passwords, and opens Android's choice:
   pick **PswManager**.
2. **Chrome** fills through it only with *Autofill using another service* on in Chrome's own
   settings. Other browsers do it by themselves.

**Then** tap a user name or password field in any app or website:

- **Locked**: the only suggestion is **Unlock PswManager**. Tap it, unlock (fingerprint or master
  password), and the suggestions appear.
- **Unlocked**: the entries for that app or site are offered by title and user name. Tap one to fill
  the fields.
- **Not there?** The last suggestion, **Choose an entry in PswManager…** (or *Another entry in
  PswManager…*, *More entries in PswManager…*), lets you search all entries and pick one. That entry is then **linked** to the app or site
  and offered there by itself from then on.

Which entries are offered:

- **for an app**: entries linked to that app (its `androidapp://` address, which PswManager and
  Keepass2Android keep in the entry), and only if the app is signed by the same maker as when the entry
  was linked;
- **for a website**: entries whose web address has that site's host, and only in browsers Android trusts
  to tell the real address. Plain `http://` pages, browser menus and pages without a domain get nothing.

**Saving a login**: after you type a new password into a login form, Android asks **Save to
PswManager?**. Tap *Save*: PswManager unlocks if needed and asks where it goes:

- **a new entry** (named after the app or the site);
- **update** one of the entries for that app or site (the old password goes to its history);
- **another entry**, searched among all and then linked.

You confirm with your fingerprint or the screen lock, and a short message names the entry it went to.

### Passkeys and Credential Manager (Android 14 and newer)

**Set it up once:** in Android's *Settings → Passwords, passkeys & accounts*, turn on PswManager and,
best, make it the *preferred service* (Android offers Google first).

- **Signing in with a passkey**: the site or app asks; PswManager lists the passkeys for it, by entry
  title and user name. Pick one and confirm with your fingerprint or the screen lock.
- **Creating a passkey**: choose **PswManager**. It unlocks if needed, shows who asks, and lets you keep
  the passkey in **a new entry** for the site or in one of the site's entries that has no passkey yet.
  It goes up with the next sync, so it also works on the PC (through the browser extension) and in
  KeePassXC.
- **Apps that ask Credential Manager for a password** get PswManager's logins in the same sheet, and can
  offer a new one to save, as with autofill.

## 10. When another app asks PswManager

**Say yes only to what you started yourself.** PswManager asks for your master password only in its own
screens, never in a web page or another app.

| What you see | What it means | What to do |
|---|---|---|
| **Unlock PswManager** (in a login field or Android's sheet) | The database is locked; nothing about your entries has been shown. | Tap it and unlock. |
| ***Trust this app?*** with the app's certificate | An app wants a login from an entry that doesn't know this app's certificate yet (one made in Keepass2Android, say), or you are linking an entry to the app. Any app can take any name; the certificate shows who really made it. | **Trust** only if it is the app you expect, from a source you trust. The entry then remembers it, and a fake app with the same name gets nothing. Otherwise **Cancel**. |
| **Save to PswManager?** | You typed a new password into a form. | *Save*, then choose where it goes — or dismiss it. |
| PswManager asks **where to keep a passkey** | A site or app wants to create a passkey. | A new entry, or one of the site's entries. |
| **The export screen**, naming another app | Another password manager wants your entries. | Confirm only if **you** started the import in that app. |
| **The new key** screen | The master password was changed on another device. | Type the new one. *Not now* lets you keep working (your changes wait); the key button in the toolbar, or **Enter the new key…** in the sync sheet, brings it back. |
| The store's **sign-in page** in the browser | You chose to sync with Dropbox, OneDrive or Google Drive, or *Sign in to … again*. | Sign in on the real page of the store. PswManager gets only its own folder there. |
| **Camera access** | You chose to scan a QR code. | Allow it — or pick a screenshot instead. |
| **All files access** | Android's pickers hide some folders (one named *Data*, say), and you wanted something there. | Optional. Turn it on only if you need such a folder; without it, Android's pickers work as usual. |

## 11. Settings

**⚙** in the toolbar, or a swipe to the left on the list. The tabs scroll sideways if they don't fit.

| Tab | What you set there |
|---|---|
| **General** | **Locking** (in the background, screen off, without a touch); **Unlock** (*Unlock with fingerprint*, how often the master password is asked: 1 to 90 days); **Autofill**; **Screen** (*Allow screenshots*); **Clipboard** (*Clear after copying*: 5 to 120 seconds) |
| **Appearance** | theme (as the phone, light or dark), *Download site icons*, *Swipes on the list* |
| **Database** | the master password and key file, the database's name and description, the default user name, how much history to keep, the encryption (with a **Test** of how long an unlock takes on this phone) |
| **Sync** | where it syncs, the visible copy's folder, *Check for changes*, *Sync now*, *Stop syncing…* or *Sync with …* |
| **About** | the version, the GitHub repository |

- What is on the **Database** tab is kept inside the database file, so the PC and KeePassXC see it too.
  The other tabs belong to this phone only.
- **Screenshots** are blocked unless you turn on *Allow screenshots*: screenshots and screen recordings
  show nothing, and the app's preview among the recent apps is blank. The camera screen is never
  captured.
- **Changing the master password or key file** (*Database* tab): type the current password, then the new
  one twice, and/or choose a key file (or let the app make a **New key file…**). Other devices need the new
  key from then on; the fingerprint is set up again at the next unlock with the password.
- **Site icons** come straight from each site, only by its name, never through another service.

## 12. Problems and answers

| Problem | Answer |
|---|---|
| No suggestions in a login field | Check *Settings → General → Autofill*. In Chrome, turn on *Autofill using another service*. Some apps block autofill: copy and paste then. |
| Nothing is offered on a plain `http://` page | On purpose: unencrypted pages get nothing. |
| An app's entry is not offered | It may not be linked to the app yet: use *Choose an entry in PswManager…* once. If the entry keeps another maker's certificate for that app, it is not offered at all. |
| The fingerprint button is gone | A new fingerprint or face was added, or 14 days have passed. Unlock once with the master password. |
| Changes don't reach the PC | Open the app or pull down the list. The phone syncs while you use it, not while it is closed. |
| *Sign in to Dropbox again* | Tap the status line and sign in. Your changes wait on the phone meanwhile. |
| A folder can't be chosen | Android's pickers refuse some folders. Pick another folder, or turn on *All files access* when the app offers it. |
| A screenshot of the app is black | *Allow screenshots* is off (the default). |

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
