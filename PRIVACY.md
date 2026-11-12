# PswManager privacy policy

Effective 11 November 2026. This policy covers PswManager for Windows and PswManager for Android, from the
GitHub Releases of this repository and from Google Play.

PswManager is an open-source (MIT) password manager on top of a KeePass (KDBX 4) file. It has no account,
no server of its own and no analytics. **The developer receives no data from the app**: nothing about you,
your device or your passwords is sent to the developer or to anyone the developer works with.

## What stays on your device

- **The database** is a KeePass file, encrypted with your master password and / or key file. It is kept where
  you choose: on the device, in a folder on your network, or in a cloud store of yours (below).
- **The master password** is never stored as it is. If you turn on unlocking with your fingerprint (Android)
  or Windows Hello (Windows), the app keeps it sealed with a key that only that unlock releases (the Android
  Keystore; Windows DPAPI with a Windows Hello signature), on that device only. Turning the option off
  deletes it.
- **Settings and the site icon cache** are kept in the app's own storage. Icon files are named with a hash of
  the site keyed by a secret of the app, so their names do not reveal which sites you use.
- **Sign-ins to cloud stores** (refresh tokens) are kept in the Windows Credential Manager, or in the app's
  private storage on Android, encrypted with a key in the Android Keystore.

## What the app sends over the network, and to whom

The app connects only to the following, and only for what you use:

- **The cloud store you link**: Dropbox, Google Drive or Microsoft OneDrive, under your own account. The app
  uploads and downloads your encrypted database file there to keep it in sync. You sign in in the browser
  (OAuth 2 with PKCE), and the app asks for the narrowest access that works: Dropbox's app folder, OneDrive's
  app folder, and on Google Drive only the files the app created itself (`drive.file`). The app never reads
  anything else in your store. What the provider does with your account is covered by its own policy:
  [Dropbox](https://www.dropbox.com/privacy), [Google](https://policies.google.com/privacy),
  [Microsoft](https://privacy.microsoft.com/privacystatement).
- **Websites, for their icons**: to show an entry's icon, the app downloads it directly from the site in the
  entry's address, never through a third-party icon service. That site sees a request from your IP address.
  Turn off *Download site icons* in the settings to stop it.
- **GitHub, for updates** (Windows): *Check for updates* in the settings asks GitHub's API for the latest
  release, only when you press it. Nothing is downloaded or installed.

Nothing else: no advertising, no analytics, no crash reporting, no tracking.

## Google user data

When you link Google Drive, the app receives an access token for `drive.file` and uses it only to list, read
and write the database files it created in your Drive, on your device. PswManager's use and transfer of
information received from Google APIs adheres to the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements. Google user data is not sent to the developer or to anyone else, not
used for advertising, and not read by people.

## Android permissions and services

- **Camera**: only to read a TOTP QR code when you scan one. The frames are decoded in the app and not kept.
- **Autofill and credential provider**: when you turn PswManager on as Android's autofill service or
  credential provider, it reads the sign-in fields of the app or web page being filled, to offer and save
  your logins and passkeys. What it reads stays on the device.
- **Seeing other apps**: to tell an app being filled from another app using its name, PswManager reads that
  app's signing certificate. It looks only at apps with a launcher icon and reads only the certificate of the
  app being filled.
- **All files access** (only in the GitHub Release's APK, never in the Google Play build): asked for only if
  you choose to browse the phone's folders, and used only for the files you choose.
- **Importing from another password manager** goes through Android's Credential Manager: the other app hands
  its data to PswManager on the device, after you confirm it there.

## Browser integration (Windows)

The KeePassXC-Browser extension, if you connect it, talks to PswManager through a local channel that only
your Windows user can open. Nothing goes over the network.

## Your control over your data

Everything is on your devices and in your own cloud account, so you keep full control:

- *Stop syncing* in the settings unlinks a database from its store; the app deletes its sign-in once no
  database uses that store. You can also revoke the app's access in your Dropbox, Google or Microsoft account
  settings at any time.
- Uninstall the app, or delete its data folder on Windows, to remove its settings and cache. Your database
  file stays where you keep it until you delete it.

## Children

PswManager is not directed at children and collects no data from anyone.

## Changes

Changes to this policy are made in this file; its history in the repository shows every version, and the
date above changes with each.

## Contact

Questions or concerns: open an issue at https://github.com/OlegG90/pswmanager/issues.
