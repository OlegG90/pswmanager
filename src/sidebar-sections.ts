/** The sidebar's four sections (groups, tags, tools, settings): each heading
 *  folds its list away and back, and which ones are folded is kept on this PC.
 *  The headings, the tools and the settings' tabs have their icons here; the
 *  groups' are shared with the phone (src/icons.ts). */
import { pathIcon } from './icons'

const KEY = 'sidebar-folded'

/** Strokes on a 24×24 grid, as the shared icons; drawn after docs/design/winui3. */
export const SIDEBAR_PATHS = {
  // The sections.
  groups: 'M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2z',
  tags: 'M12.586 2.586A2 2 0 0 0 11.172 2H4a2 2 0 0 0-2 2v7.172a2 2 0 0 0 .586 1.414l8.704 8.704a2.426 2.426 0 0 0 3.42 0l6.58-6.58a2.426 2.426 0 0 0 0-3.42zM7.5 7a.5.5 0 1 0 0 1a.5.5 0 1 0 0-1z',
  tools: 'M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z',
  settings: 'M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6zM19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z',
  // The tools.
  health: 'M19 14c1.49-1.46 3-3.21 3-5.5A5.5 5.5 0 0 0 16.5 3c-1.76 0-3 .5-4.5 2-1.5-1.5-2.74-2-4.5-2A5.5 5.5 0 0 0 2 8.5c0 2.3 1.5 4.05 3 5.5l7 7zM3.22 12H9.5l.5-1 2 4.5 2-7 1.5 3.5h5.27',
  similar: 'M18 15a3 3 0 1 0 0 6a3 3 0 1 0 0-6zM6 3a3 3 0 1 0 0 6a3 3 0 1 0 0-6zM6 21V9a9 9 0 0 0 9 9',
  import: 'M12 3v12M8 11l4 4 4-4M8 5H4a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2h-4',
  export: 'M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M17 8l-5-5-5 5M12 3v12',
  // The settings' tabs.
  general: 'M4 7h10M18 7h2M4 17h4M12 17h8M16 5v4M10 15v4',
  window: 'M3 5h18v14H3zM3 9h18',
  database: 'M5 6c0-1.4 3.1-2.5 7-2.5s7 1.1 7 2.5-3.1 2.5-7 2.5S5 7.4 5 6zM5 6v12c0 1.4 3.1 2.5 7 2.5s7-1.1 7-2.5V6M5 12c0 1.4 3.1 2.5 7 2.5s7-1.1 7-2.5',
  sync: 'M20 12a8 8 0 0 1-14 5.3M4 12a8 8 0 0 1 14-5.3M18 3v4h-4M6 21v-4h4',
  backup: 'M12 4v11M7 10l5 5 5-5M5 20h14',
  about: 'M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17zM12 11v5M12 8h.01',
  // A section's state: down while open.
  chevron: 'M6 9l6 6 6-6',
}

export type SidebarIcon = keyof typeof SIDEBAR_PATHS

export const sidebarIcon = (name: SidebarIcon) => pathIcon(SIDEBAR_PATHS[name])

/** The folded sections as kept; nothing when the storage cannot be read. */
function folded(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(KEY) ?? '[]') as string[])
  } catch {
    return new Set()
  }
}

/** Gives every `.section-head` in `sidebar` its icon and chevron, and folds
 *  the list it controls away and back on a click. */
export function foldingSections(sidebar: HTMLElement) {
  const kept = folded()
  for (const head of sidebar.querySelectorAll<HTMLButtonElement>('.section-head')) {
    const name = head.dataset.section as SidebarIcon
    const list = document.getElementById(head.getAttribute('aria-controls') ?? '')
    if (!list) continue
    head.prepend(sidebarIcon(name))
    head.append(sidebarIcon('chevron'))
    const show = (open: boolean) => {
      head.setAttribute('aria-expanded', String(open))
      list.hidden = !open
    }
    show(!kept.has(name))
    head.addEventListener('click', () => {
      const open = list.hidden === true
      show(open)
      if (open) kept.delete(name)
      else kept.add(name)
      try {
        localStorage.setItem(KEY, JSON.stringify([...kept]))
      } catch {
        // Not kept: the sections open again next time.
      }
    })
  }
}
