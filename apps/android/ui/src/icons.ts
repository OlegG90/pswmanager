/** The phone's toolbar icons: simple strokes in the text's colour. */

const PATHS = {
  menu: 'M4 6h16M4 12h16M4 18h16',
  search: 'M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14zM16.5 16.5L21 21',
  lock: 'M6 11h12v9H6zM8.5 11V8a3.5 3.5 0 0 1 7 0v3',
  close: 'M6 6l12 12M18 6L6 18',
  back: 'M15 5l-7 7l7 7',
  settings: 'M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6zM12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1',
}

export type IconName = keyof typeof PATHS

export function svgIcon(name: IconName): SVGSVGElement {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg')
  svg.setAttribute('viewBox', '0 0 24 24')
  svg.setAttribute('aria-hidden', 'true')
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path')
  path.setAttribute('d', PATHS[name])
  svg.append(path)
  return svg
}
