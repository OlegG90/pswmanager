# Design

The design handoff from Claude Design, kept as the reference for the window's look. These are HTML
prototypes, not app code: nothing here is built or shipped.

- `PswManager Prototype.dc.html` — the clickable prototype in the Classical design system: choose
  database, unlock, vault, entry, editor with generator, settings and password health. Open it in a
  browser; it loads `support.js` and the design system from `_ds/`.
- `PswManager Mockups.dc.html` — the first, static mockups (before the design system was chosen).
- `_ds/classical-…/` — the Classical design system: tokens and component classes (`styles.css`) and its
  guide (`readme.md`). The window followed it until it took the WinUI 3 look; the Android app still does.
- `github.md` — the design tool's map of screens to files in this repository.
- `winui3/` — static mockups of the window in the WinUI 3 (Fluent) style, light and dark: unlock, vault,
  editor with the generator, settings and password health. `src/style.css` takes its colours, controls
  and layout from here. Open them in a browser like the prototype (they load `../support.js`); all data in
  them is made up.
- `android/` — the Android mockups in the same design system, for the phone app (see its README).

What the app took from the prototype: the screens and what is on them — the unlock screen, vault,
entry view, editor and generator, the settings screen, the first-run "Choose your database" screen and
password health. Their look now follows `winui3/`, which keeps the standard Windows frame of
`docs/spec.md`: the search box stays in the window's toolbar rather than in the title bar. The entry
view's attachments are in neither handoff: a card under the values lists each file with its size,
**Open** and **Save**; files are added, renamed and removed in the editor's *Files* section
(`docs/spec.md`, *Editing*).
