import { button, el } from './dom'

export interface MenuItem {
  label: string
  title: string
  action: () => void
  /** Drawn in the danger colour, like Delete. */
  danger?: boolean
}

/** The open menu's close, so opening another one closes it first. */
let closeOpen: (() => void) | null = null

/**
 * The menu of `items`: arrow keys move between them, Esc closes it (`close`
 * with true gives the focus back to what opened it), Tab moves on.
 */
function menuOf(items: MenuItem[], close: (refocus: boolean) => void) {
  const entries = items.map((item) => {
    const entry = button(item.label, item.title, () => {
      close(false)
      item.action()
    }, item.danger ? 'danger' : '')
    entry.setAttribute('role', 'menuitem')
    entry.tabIndex = -1
    return entry
  })
  const menu = el('div', { className: 'menu', role: 'menu', hidden: true }, ...entries)
  menu.addEventListener('keydown', (e) => {
    const at = entries.indexOf(document.activeElement as HTMLButtonElement)
    const step = { ArrowDown: 1, ArrowUp: -1 }[e.key]
    if (step) {
      entries[(at + step + entries.length) % entries.length].focus()
    } else if (e.key === 'Home' || e.key === 'End') {
      entries[e.key === 'Home' ? 0 : entries.length - 1].focus()
    } else if (e.key === 'Escape') {
      close(true)
    } else if (e.key === 'Tab') {
      close(false)
      return // the focus moves on as usual
    } else {
      // Enter and Space press the focused action; other keys do nothing
      // here, rather than start a search. Shortcuts such as Ctrl+L still work.
      if (!e.ctrlKey) e.stopPropagation()
      return
    }
    // Handled here: the window's own keys (Esc hides it to the tray) wait.
    e.preventDefault()
    e.stopPropagation()
  })
  return { menu, first: () => entries[0]?.focus() }
}

/** Closes the menu when a click lands outside `wrap` (the focus leaving it is
 *  watched by each menu); returns what stops watching. */
function closeOutside(wrap: HTMLElement, close: () => void): () => void {
  const outside = (e: Event) => {
    if (!wrap.contains(e.target as Node)) close()
  }
  document.addEventListener('pointerdown', outside, true)
  return () => document.removeEventListener('pointerdown', outside, true)
}

/** Closes the open menu, if any: the window locks under it. */
export const closeMenus = () => closeOpen?.()

/**
 * A "⋯" button that opens a short list of actions under it (above it when
 * there is no room below). Arrow keys move between the actions, Esc closes
 * the menu and gives the focus back to the button; a click elsewhere or
 * moving the focus away closes it too.
 */
export function menuButton(title: string, items: MenuItem[]): HTMLSpanElement {
  const trigger = button('⋯', title, () => (menu.hidden ? open() : close()), 'more')
  trigger.setAttribute('aria-haspopup', 'menu')
  trigger.setAttribute('aria-expanded', 'false')
  const { menu, first } = menuOf(items, (refocus) => {
    close()
    if (refocus) trigger.focus()
  })
  const wrap = el('span', { className: 'menu-wrap' }, trigger, menu)
  let unwatch = () => {}

  function open() {
    closeOpen?.()
    closeOpen = close
    menu.hidden = false
    trigger.setAttribute('aria-expanded', 'true')
    const below = window.innerHeight - trigger.getBoundingClientRect().bottom
    menu.classList.toggle('up', below < menu.offsetHeight + 8)
    first()
    unwatch = closeOutside(wrap, close)
  }
  function close() {
    if (menu.hidden) return
    menu.hidden = true
    trigger.setAttribute('aria-expanded', 'false')
    unwatch()
    if (closeOpen === close) closeOpen = null
  }
  wrap.addEventListener('focusout', (e) => {
    if (!wrap.contains(e.relatedTarget as Node)) close()
  })
  return wrap
}

/**
 * The same list of actions at a point of the window (#255): where the list
 * was right-clicked, or under its chosen entry for Shift+F10 and the menu key.
 * It stays inside the window and closes as the "⋯" menu does; `back` gets the
 * focus when Esc closes it.
 */
export function contextMenu(items: MenuItem[], x: number, y: number, back: HTMLElement) {
  closeOpen?.()
  const { menu, first } = menuOf(items, (refocus) => {
    close()
    if (refocus) back.focus()
  })
  const wrap = el('div', { className: 'menu-wrap context' }, menu)
  document.body.append(wrap)
  menu.hidden = false
  wrap.style.left = `${Math.max(0, Math.min(x, window.innerWidth - menu.offsetWidth - 4))}px`
  wrap.style.top = `${Math.max(0, Math.min(y, window.innerHeight - menu.offsetHeight - 4))}px`
  const unwatch = closeOutside(wrap, close)
  let closed = false
  function close() {
    // Removing the menu takes the focus from it, and its focusout calls this
    // again while it is still in the page: closed first, it is removed once.
    if (closed) return
    closed = true
    unwatch()
    if (closeOpen === close) closeOpen = null
    wrap.remove()
  }
  closeOpen = close
  wrap.addEventListener('focusout', (e) => {
    if (!wrap.contains(e.relatedTarget as Node)) close()
  })
  first()
}
