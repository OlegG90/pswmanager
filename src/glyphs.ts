/**
 * The app's own entry icons, drawn for it (no icon set is bundled). Each is
 * stored as one of KeePass's standard icon numbers, so other clients show
 * their own picture for the same idea; numbers without a drawing here show
 * the key.
 */
export const GLYPHS: { id: number; name: string; path: string }[] = [
  { id: 0, name: 'Key', path: 'M12 12h7m-2 0v3M12.2 12a3.2 3.2 0 1 1-6.4 0 3.2 3.2 0 0 1 6.4 0' },
  { id: 1, name: 'Web site', path: 'M18 12a6 6 0 1 1-12 0 6 6 0 0 1 12 0M6 12h12M12 6c-3.4 3.4-3.4 8.6 0 12M12 6c3.4 3.4 3.4 8.6 0 12' },
  { id: 58, name: 'Account', path: 'M14.8 9.5a2.8 2.8 0 1 1-5.6 0 2.8 2.8 0 0 1 5.6 0M6.5 18c1-3 3-4.2 5.5-4.2s4.5 1.2 5.5 4.2' },
  { id: 19, name: 'E-mail', path: 'M7 7.5h10a1.5 1.5 0 0 1 1.5 1.5v6a1.5 1.5 0 0 1-1.5 1.5H7A1.5 1.5 0 0 1 5.5 15V9A1.5 1.5 0 0 1 7 7.5M6 8.5l6 4.5 6-4.5' },
  { id: 37, name: 'Bank', path: 'M5.5 10h13L12 6zM7.5 11v5M10.5 11v5M13.5 11v5M16.5 11v5M5.5 17.5h13' },
  { id: 66, name: 'Card', path: 'M6.5 7.5h11a1.5 1.5 0 0 1 1.5 1.5v6a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 15V9a1.5 1.5 0 0 1 1.5-1.5M5 10.5h14M7.5 14h3' },
  { id: 9, name: 'Identity', path: 'M6.5 6.5h11A1.5 1.5 0 0 1 19 8v8a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 16V8a1.5 1.5 0 0 1 1.5-1.5M11.1 11a1.6 1.6 0 1 1-3.2 0 1.6 1.6 0 0 1 3.2 0M7.3 15c.5-1.2 1.2-1.7 2.2-1.7s1.7.5 2.2 1.7M13.5 10.5H17M13.5 13.5H17' },
  { id: 68, name: 'Phone', path: 'M10 5h4a1.5 1.5 0 0 1 1.5 1.5v11A1.5 1.5 0 0 1 14 19h-4a1.5 1.5 0 0 1-1.5-1.5v-11A1.5 1.5 0 0 1 10 5M11 16.5h2' },
  { id: 18, name: 'Computer', path: 'M6 6.5h12a1 1 0 0 1 1 1v7a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1v-7a1 1 0 0 1 1-1M9.5 18.5h5M12 15.5v3' },
  { id: 3, name: 'Server', path: 'M7 5.5h10a1 1 0 0 1 1 1v3.5a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V6.5a1 1 0 0 1 1-1M7 13h10a1 1 0 0 1 1 1v3.5a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V14a1 1 0 0 1 1-1M8.5 8.25h.01M8.5 15.75h.01' },
  { id: 12, name: 'Wireless', path: 'M6.5 10.5a8 8 0 0 1 11 0M8.7 13a5 5 0 0 1 6.6 0M12 16h.01' },
  { id: 30, name: 'Terminal', path: 'M6.5 6.5h11A1.5 1.5 0 0 1 19 8v8a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 16V8a1.5 1.5 0 0 1 1.5-1.5M8 10l2.5 2L8 14M12.5 14.5H16' },
  { id: 60, name: 'Home', path: 'M6 11.5l6-5 6 5M7.5 10.5v7h9v-7M10.5 17.5v-4h3v4' },
  { id: 67, name: 'Certificate', path: 'M12 5.5l6 2V12c0 3.5-2.6 5.8-6 6.8-3.4-1-6-3.3-6-6.8V7.5zM9.5 12l1.8 1.8 3.2-3.3' },
  { id: 44, name: 'Note', path: 'M7 5.5h7l3 3v10H7zM14 5.5v3h3M9.5 12h5M9.5 15h5' },
  { id: 61, name: 'Star', path: 'M12 6l1.8 3.8 4.2.5-3.1 2.9.8 4.1-3.7-2-3.7 2 .8-4.1-3.1-2.9 4.2-.5z' },
]

const cache = new Map<number, string>()

/** The icon for KeePass standard icon `id`, as a data URL. */
export function glyphIcon(id: number): string {
  const glyph = GLYPHS.find((g) => g.id === id) ?? GLYPHS[0]
  let url = cache.get(glyph.id)
  if (!url) {
    url =
      'data:image/svg+xml,' +
      encodeURIComponent(
        `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="2" y="2" width="20" height="20" rx="5" fill="#8a94a3"/>` +
          `<path d="${glyph.path}" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>`,
      )
    cache.set(glyph.id, url)
  }
  return url
}

export const DEFAULT_ICON = glyphIcon(0)
