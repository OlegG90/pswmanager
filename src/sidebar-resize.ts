/** The sidebar's width, changed by dragging its edge (or with the arrow keys,
 *  Home and End on it) and kept across runs on this PC; a double click puts
 *  it back. The list and the entry always keep room beside it. */

const KEY = 'sidebar-width'
const MIN = 140
const MAX = 420
const DEFAULT = 180
const STEP = 10
/** The list's least width (`#vault`'s grid) and the entry's least room. */
const OTHERS = 220 + 160

/** Makes `handle` (inside the sidebar) resize it within `layout`, which reads `--sidebar-width`. */
export function sidebarResizer(layout: HTMLElement, handle: HTMLElement) {
  let width = DEFAULT
  let wanted = DEFAULT
  // While the vault is hidden it has no width: no room to keep yet.
  const most = () => (layout.clientWidth ? Math.max(MIN, Math.min(MAX, layout.clientWidth - OTHERS)) : MAX)
  const apply = () => {
    width = Math.round(Math.min(most(), Math.max(MIN, wanted)))
    layout.style.setProperty('--sidebar-width', `${width}px`)
    handle.setAttribute('aria-valuenow', String(width))
    handle.setAttribute('aria-valuemax', String(most()))
  }
  const choose = (w: number) => {
    wanted = w
    apply()
  }
  const keep = () => {
    try {
      localStorage.setItem(KEY, String(width))
    } catch {
      // Not kept: the width still applies until the app closes.
    }
  }
  handle.setAttribute('aria-valuemin', String(MIN))
  try {
    wanted = Number(localStorage.getItem(KEY)) || DEFAULT
  } catch {
    // Nothing kept: the usual width.
  }
  apply()
  // Shown, or a narrower window: room from the sidebar first; a wider one gives it back.
  new ResizeObserver(apply).observe(layout)

  handle.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return
    e.preventDefault()
    handle.setPointerCapture(e.pointerId)
    const left = handle.parentElement!.getBoundingClientRect().left
    const move = (m: PointerEvent) => choose(m.clientX - left)
    const end = () => {
      handle.removeEventListener('pointermove', move)
      handle.removeEventListener('lostpointercapture', end)
      keep()
    }
    handle.addEventListener('pointermove', move)
    // Released, cancelled or taken away: capture is lost in every case.
    handle.addEventListener('lostpointercapture', end)
  })
  handle.addEventListener('dblclick', () => (choose(DEFAULT), keep()))
  const keys: Record<string, () => number> = {
    ArrowLeft: () => width - STEP,
    ArrowRight: () => width + STEP,
    Home: () => MIN,
    End: () => most(),
  }
  handle.addEventListener('keydown', (e) => {
    const to = keys[e.key]
    if (!to) return
    e.preventDefault()
    choose(to())
    keep()
  })
}
