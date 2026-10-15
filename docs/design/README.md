# Design

The design handoff from Claude Design, kept as the reference for the window's look. These are HTML
prototypes, not app code: nothing here is built or shipped.

- `PswManager Prototype.dc.html` — the clickable prototype in the Classical design system: choose
  database, unlock, vault, entry, editor with generator, settings and password health. Open it in a
  browser; it loads `support.js` and the design system from `_ds/`.
- `PswManager Mockups.dc.html` — the first, static mockups (before the design system was chosen).
- `_ds/classical-…/` — the Classical design system: tokens and component classes (`styles.css`) and its
  guide (`readme.md`). `src/style.css` takes its colours from here.
- `github.md` — the design tool's map of screens to files in this repository.
- `winui3/` — static mockups of the window in the WinUI 3 (Fluent) style, light and dark: unlock, vault,
  editor with the generator, settings and password health. A proposal, not what the app follows yet: they
  put the search box into the title bar, while `docs/spec.md` keeps the standard Windows frame. Open them
  in a browser like the prototype (they load `../support.js`); all data in them is made up.
- `android/` — the Android mockups in the same design system, for the phone app (see its README).

What the app takes from it so far: the look of the unlock screen, vault, entry view, editor and
generator, the settings screen, the first-run "Choose your database" screen and password health —
all of the prototype. The entry view's attachments are not in the handoff: a section under the notes
lists each file with its size, **Open**, and a **⋯** menu with **Save…**; files are added, renamed and
removed in the editor's *Files* section (`docs/spec.md`, *Editing*). They follow the view's own rows and the editor's section
headings. The app uses system fonts instead of the design system's Google Fonts, and adds a dark variant
of the palette.
