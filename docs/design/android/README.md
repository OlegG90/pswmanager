# Android design

Mockups of PswManager for Android from Claude Design, in the Classical design system with system fonts
only. HTML mockups, not app code: nothing here is built or shipped. Open a file in a browser; it loads
`support.js` from this folder and the design system from `../_ds/`.

- `PswManager Android v2.dc.html` — the current mockups at the Samsung Galaxy S24 Ultra viewport
  (384 × 832 dp), rebuilt from `apps/android/ui` (version 0.20.0) with its labels:
  1a–1d first run (choose your database, pick the file, where to keep it, new database);
  2a unlock with the fingerprint button; 2b the list (normal, search open, entries chosen);
  2c drawer with groups, Untagged and tags; 2d Tools; 3a entry view; 3b editor with the generator;
  3c history; 4a Trash; 4b the sync sheet (merged, sign in again, new key); 4c every status-line state;
  5a settings in tabs (General, Appearance, Database, Sync, About). Light and dark theme.
- `Android UI Audit.dc.html` — how v1 differed from the code and `docs/spec-android.md`, and what v2
  changed.
- `PswManager Android.dc.html` — v1, kept for reference.

Where a mockup and `docs/spec-android.md` disagree, the spec wins. Buttons and floating buttons are
outlined, as Classical asks; the app's `style.css` still fills them (see the audit).
