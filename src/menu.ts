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
 * A "⋯" button that opens a short list of actions under it (above it when
 * there is no room below). Arrow keys move between the actions, Esc closes
 * the menu and gives the focus back to the button; a click elsewhere or
 * moving the focus away closes it too.
 */
export function menuButton(title: string, items: MenuItem[]): HTMLSpanElement {
  const trigger = button('⋯', title, () => (menu.hidden ? open() : close()), 'more')
  trigger.setAttribute('aria-haspopup', 'menu')
  trigger.setAttribute('aria-expanded', 'false')
  const entries = items.map((item) => {
    const entry = button(item.label, item.title, () => {
      close()
      item.action()
    }, item.danger ? 'danger' : '')
    entry.setAttribute('role', 'menuitem')
    entry.tabIndex = -1
    return entry
  })
  const menu = el('div', { className: 'menu', role: 'menu', hidden: true }, ...entries)
  const wrap = el('span', { className: 'menu-wrap' }, trigger, menu)

  const outside = (e: Event) => {
    if (!wrap.contains(e.target as Node)) close()
  }
  function open() {
    closeOpen?.()
    closeOpen = close
    menu.hidden = false
    trigger.setAttribute('aria-expanded', 'true')
    const below = window.innerHeight - trigger.getBoundingClientRect().bottom
    menu.classList.toggle('up', below < menu.offsetHeight + 8)
    entries[0]?.focus()
    document.addEventListener('pointerdown', outside, true)
  }
  function close() {
    if (menu.hidden) return
    menu.hidden = true
    trigger.setAttribute('aria-expanded', 'false')
    document.removeEventListener('pointerdown', outside, true)
    if (closeOpen === close) closeOpen = null
  }

  menu.addEventListener('keydown', (e) => {
    const at = entries.indexOf(document.activeElement as HTMLButtonElement)
    const step = { ArrowDown: 1, ArrowUp: -1 }[e.key]
    if (step) {
      entries[(at + step + entries.length) % entries.length].focus()
    } else if (e.key === 'Home' || e.key === 'End') {
      entries[e.key === 'Home' ? 0 : entries.length - 1].focus()
    } else if (e.key === 'Escape') {
      close()
      trigger.focus()
    } else if (e.key === 'Tab') {
      close()
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
  wrap.addEventListener('focusout', (e) => {
    if (!wrap.contains(e.relatedTarget as Node)) close()
  })
  return wrap
}
