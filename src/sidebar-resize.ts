/** The sidebar's width, changed by dragging its edge (or with the arrow keys
 *  on it) and kept across runs on this PC; a double click puts it back. */

const KEY = 'sidebar-width'
const MIN = 140
const MAX = 420
const DEFAULT = 180
const STEP = 10

const clamp = (width: number) => Math.round(Math.min(MAX, Math.max(MIN, width)))

/** Makes `handle` resize `sidebar` within `layout` (which reads `--sidebar-width`). */
export function sidebarResizer(layout: HTMLElement, sidebar: HTMLElement, handle: HTMLElement) {
  const set = (width: number, keep: boolean) => {
    const w = clamp(width)
    layout.style.setProperty('--sidebar-width', `${w}px`)
    handle.setAttribute('aria-valuenow', String(w))
    if (keep) {
      try {
        localStorage.setItem(KEY, String(w))
      } catch {
        // Not kept: the width still applies until the app closes.
      }
    }
  }
  handle.setAttribute('aria-valuemin', String(MIN))
  handle.setAttribute('aria-valuemax', String(MAX))
  let saved = DEFAULT
  try {
    saved = Number(localStorage.getItem(KEY)) || DEFAULT
  } catch {
    // Nothing kept: the default width.
  }
  set(saved, false)

  handle.addEventListener('pointerdown', (e) => {
    e.preventDefault()
    handle.setPointerCapture(e.pointerId)
    const left = sidebar.getBoundingClientRect().left
    const move = (m: PointerEvent) => set(m.clientX - left, false)
    const up = (u: PointerEvent) => {
      handle.removeEventListener('pointermove', move)
      handle.removeEventListener('pointerup', up)
      set(u.clientX - left, true)
    }
    handle.addEventListener('pointermove', move)
    handle.addEventListener('pointerup', up)
  })
  handle.addEventListener('dblclick', () => set(DEFAULT, true))
  handle.addEventListener('keydown', (e) => {
    const step = e.key === 'ArrowLeft' ? -STEP : e.key === 'ArrowRight' ? STEP : 0
    if (!step) return
    e.preventDefault()
    set(sidebar.getBoundingClientRect().width + step, true)
  })
}
