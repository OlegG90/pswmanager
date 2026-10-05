/** Icons both apps share: the sidebar's groups, an entry's commands (copy,
 *  show / hide, open), its files' (open, save, rename, remove, attach) and the
 *  editor's two-state ones (the eye, the padlock).
 *  Lucide's (lucide.dev, ISC licence), as in the phone's mockups; strokes in
 *  the text's colour. */
import { el } from './dom'
import type { Group } from './search'

export const SHARED_PATHS = {
  // The groups.
  layers:
    'M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83zM22 17.65l-9.17 4.16a2 2 0 0 1-1.66 0L2 17.65M22 12.65l-9.17 4.16a2 2 0 0 1-1.66 0L2 12.65',
  star: 'M11.53 2.3a.53.53 0 0 1 .95 0l2.31 4.68a2.12 2.12 0 0 0 1.6 1.16l5.16.76a.53.53 0 0 1 .3.9l-3.74 3.64a2.12 2.12 0 0 0-.61 1.88l.88 5.14a.53.53 0 0 1-.77.56l-4.62-2.43a2.12 2.12 0 0 0-1.97 0L6.4 21.01a.53.53 0 0 1-.77-.56l.88-5.14a2.12 2.12 0 0 0-.61-1.88L2.16 9.8a.53.53 0 0 1 .3-.91l5.16-.75a2.12 2.12 0 0 0 1.6-1.16z',
  clock: 'M12 2a10 10 0 1 0 0 20a10 10 0 1 0 0-20zM12 6v6l4 2',
  shieldCheck:
    'M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1zM9 12l2 2l4-4',
  keyRound:
    'M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4zM16.5 7a.5.5 0 1 0 0 1a.5.5 0 1 0 0-1z',
  layoutTemplate:
    'M4 3h16a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM4 14h7a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1zM17 14h3a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-3a1 1 0 0 1-1-1v-5a1 1 0 0 1 1-1z',
  trash: 'M3 6h18M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2M10 11v6M14 11v6',
  // An entry's commands.
  copy: 'M10 8h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H10a2 2 0 0 1-2-2V10a2 2 0 0 1 2-2zM4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2',
  eye: 'M2.062 12.348a1 1 0 0 1 0-.696a10.75 10.75 0 0 1 19.876 0a1 1 0 0 1 0 .696a10.75 10.75 0 0 1-19.876 0M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6z',
  eyeOff:
    'M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575a1 1 0 0 1 0 .696a10.747 10.747 0 0 1-1.444 2.49M14.084 14.158a3 3 0 0 1-4.242-4.242M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151a1 1 0 0 1 0-.696a10.75 10.75 0 0 1 4.446-5.143M2 2l20 20',
  externalLink: 'M15 3h6v6M10 14L21 3M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6',
  // An entry's files.
  fileOutput: 'M14 2v4a2 2 0 0 0 2 2h4M4 7V4a2 2 0 0 1 2-2M4.063 20.999a2 2 0 0 0 2 1L18 22a2 2 0 0 0 2-2V7l-5-5H6M5 11l-3 3M5 17l-3-3h10',
  download: 'M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5l5-5M12 15V3',
  pencil: 'M21.17 6.81a1 1 0 0 0-3.98-3.99L3.84 16.17a2 2 0 0 0-.5.83l-1.32 4.35a.5.5 0 0 0 .62.62l4.35-1.32a2 2 0 0 0 .83-.5zM15 5l4 4',
  paperclip:
    'M13.234 20.252L21 12.3M16 6l-8.414 8.586a2 2 0 0 0 0 2.828a2 2 0 0 0 2.828 0l8.414-8.586a4 4 0 0 0 0-5.656a4 4 0 0 0-5.656 0l-8.415 8.585a6 6 0 1 0 8.486 8.486',
  // A field protected or not, in the editor.
  padlock: 'M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2zM7 11V7a5 5 0 0 1 10 0v4',
  padlockOpen: 'M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2zM7 11V7a5 5 0 0 1 9.9-1',
}

export type SharedIcon = keyof typeof SHARED_PATHS

/** Each group's icon in the sidebar (the phone's drawer shows the first five). */
export const GROUP_ICONS: Record<Group, SharedIcon> = {
  all: 'layers',
  favorites: 'star',
  expired: 'clock',
  '2fa': 'shieldCheck',
  passkey: 'keyRound',
  templates: 'layoutTemplate',
  trash: 'trash',
}

/** A 24×24 stroke icon drawn from a path. */
export function pathIcon(d: string): SVGSVGElement {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
  svg.setAttribute('viewBox', '0 0 24 24')
  svg.setAttribute('aria-hidden', 'true')
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path')
  path.setAttribute('d', d)
  svg.append(path)
  return svg
}

export const icon = (name: SharedIcon) => pathIcon(SHARED_PATHS[name])

/** A command as an icon, with its title (the tooltip and what a screen reader says). */
export function iconAction(name: SharedIcon, title: string, onClick: () => void, className = ''): HTMLButtonElement {
  const b = el('button', { type: 'button', title, className: `icon action ${className}`.trim(), onclick: onClick })
  b.setAttribute('aria-label', title)
  b.append(icon(name))
  return b
}

/** The eye: open while the value is hidden (to show it), crossed out once shown. */
export const shownIcon = (shown: boolean) => icon(shown ? 'eyeOff' : 'eye')

/** The padlock: closed when protected, open when not. */
export const protectedIcon = (on: boolean) => icon(on ? 'padlock' : 'padlockOpen')
