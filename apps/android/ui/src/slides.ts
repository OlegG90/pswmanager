import { screens } from './page'

/** Sideways swipes across `target` (#194) bring a panel in with the finger:
 *  to the right `right`'s, to the left `left`'s. Let go past a third of the
 *  way (or flicked), it opens; otherwise it goes back. A swipe counts once it
 *  is clearly sideways, so the list's scrolling and pull-to-sync are not taken
 *  for one; one that starts at the screen's edge is left to Android's Back
 *  gesture. `allowed` says whether swipes are on now. */
export function swipes(target: HTMLElement, allowed: () => boolean, to: { right?: () => Sliding; left?: () => Sliding }) {
  const EDGE = 32
  const START = 12
  let from: { x: number; y: number; at: number } | null = null
  let panel: Sliding | null = null
  let dx = 0
  target.addEventListener('touchstart', (e) => {
    // Another finger mid-swipe: the panel goes back, and that is the end of it.
    panel?.release(false)
    panel = null
    dx = 0
    const { clientX: x, clientY: y } = e.touches[0]
    const inside = x > EDGE && x < window.innerWidth - EDGE
    from = e.touches.length === 1 && inside && allowed() ? { x, y, at: e.timeStamp } : null
  }, { passive: true })
  target.addEventListener('touchmove', (e) => {
    if (!from) return
    dx = e.touches[0].clientX - from.x
    const dy = e.touches[0].clientY - from.y
    if (!panel) {
      if (Math.abs(dy) > START && Math.abs(dy) >= Math.abs(dx)) from = null // a scroll
      else if (Math.abs(dx) > START && Math.abs(dx) > 2 * Math.abs(dy)) {
        const open = dx > 0 ? to.right : to.left
        if (open) panel = open()
        else from = null // nothing that way
      }
      if (!panel) return
    }
    e.preventDefault()
    panel.follow(dx)
  }, { passive: false })
  const end = (e: TouchEvent) => {
    if (!from || !panel) return
    const speed = Math.abs(dx) / Math.max(1, e.timeStamp - from.at)
    panel.release(e.type === 'touchend' && (Math.abs(dx) > window.innerWidth / 3 || speed > 0.6))
    from = null
    panel = null
  }
  target.addEventListener('touchend', end)
  target.addEventListener('touchcancel', end)
}

/** A panel coming in from a side over a shade: in step with a finger
 *  (`follow`, by how far it has moved), then let go (`release`), open or
 *  back out (`close` then). */
export interface Sliding {
  follow: (dx: number) => void
  release: (open: boolean, opened?: () => void) => void
}

/** `sliding`, open, taken away by a swipe the other way: let go far enough, it closes. */
export const closing = (sliding: Sliding & { back: (dx: number) => void }): Sliding => ({
  follow: sliding.back,
  release: (away) => sliding.release(!away),
})

/** Runs `done` once `node`'s own transform transition ends, or soon anyway
 *  (none may come: the app went to the background, or nothing moved). */
function afterSlide(node: HTMLElement, done: () => void) {
  let finished = false
  const finish = () => {
    if (finished) return
    finished = true
    node.removeEventListener('transitionend', ended)
    done()
  }
  const ended = (e: TransitionEvent) => e.target === node && finish()
  node.addEventListener('transitionend', ended)
  setTimeout(finish, 400)
}

/** The screen shown, taken away to the right by a swipe; let go far enough, `away` runs. */
export function leaving(node: HTMLElement, away: () => void): Sliding {
  let moved = 0
  const place = (x: string) => (node.style.transform = x)
  return {
    follow: (dx) => {
      node.style.transition = 'none'
      moved = Math.max(0, dx)
      place(`translateX(${moved}px)`)
    },
    release: (go) => {
      const before = screens.current
      // The next screen first, then the node back in place: no flash of the
      // old one. Not when Back already left this screen meanwhile.
      const done = () => {
        if (go && screens.current === before) away()
        node.style.transition = ''
        place('')
      }
      if (!go && moved === 0) return done()
      node.style.transition = 'transform 200ms ease-out'
      requestAnimationFrame(() => {
        afterSlide(node, done)
        place(go ? 'translateX(100%)' : 'translateX(0)')
      })
    },
  }
}

/** A panel coming in over `shade` from its side (`sign` -1: the left, 1: the
 *  right); `close` takes it away when it is let go short of open. */
export function slide(shade: HTMLElement, panel: HTMLElement, sign: -1 | 1, close: () => void): Sliding & { back: (dx: number) => void } {
  let shown = 0
  const show = (part: number) => {
    shown = part
    panel.style.transform = `translateX(${sign * (1 - part) * 100}%)`
    shade.style.backgroundColor = `rgb(0 0 0 / ${Math.round(40 * part)}%)`
  }
  const part = (dx: number) => Math.min(1, Math.max(0, (-sign * dx) / panel.offsetWidth))
  shade.classList.add('following')
  show(0)
  return {
    follow: (dx) => show(part(dx)),
    // Open, moved back towards its side.
    back: (dx) => {
      shade.classList.add('following')
      show(1 - part(-dx))
    },
    release: (open, opened) => {
      shade.classList.remove('following')
      const done = () => (open ? opened?.() : close())
      // Already there: no transition to wait for.
      if (shown === (open ? 1 : 0)) return done()
      // Laid out where the finger left it (or off screen, just added), so the
      // transition starts from there.
      void panel.offsetWidth
      afterSlide(panel, done)
      show(open ? 1 : 0)
    },
  }
}
