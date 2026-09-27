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

What the app takes from it so far: the look of the unlock screen, vault, entry view, editor and
generator, and the settings screen. Not built: the first-run screen, password health (out of scope,
see `docs/spec.md`) and reordering additional fields. The app uses system fonts instead of the design
system's Google Fonts, and adds a dark variant of the palette.
