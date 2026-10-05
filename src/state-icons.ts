/** The editor's two-state icons, for both apps: the eye (show a secret;
 *  crossed out once shown) and the padlock (a field protected; open when not).
 *  Lucide's (lucide.dev, ISC licence), as in the mockups; strokes in the
 *  text's colour. */

export const STATE_PATHS = {
  eye: 'M2.062 12.348a1 1 0 0 1 0-.696a10.75 10.75 0 0 1 19.876 0a1 1 0 0 1 0 .696a10.75 10.75 0 0 1-19.876 0M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6z',
  eyeOff:
    'M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575a1 1 0 0 1 0 .696a10.747 10.747 0 0 1-1.444 2.49M14.084 14.158a3 3 0 0 1-4.242-4.242M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151a1 1 0 0 1 0-.696a10.75 10.75 0 0 1 4.446-5.143M2 2l20 20',
  padlock: 'M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2zM7 11V7a5 5 0 0 1 10 0v4',
  padlockOpen: 'M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2zM7 11V7a5 5 0 0 1 9.9-1',
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

/** The eye: open while the value is hidden (to show it), crossed out once shown. */
export const shownIcon = (shown: boolean) => pathIcon(shown ? STATE_PATHS.eyeOff : STATE_PATHS.eye)

/** The padlock: closed when protected, open when not. */
export const protectedIcon = (on: boolean) => pathIcon(on ? STATE_PATHS.padlock : STATE_PATHS.padlockOpen)
