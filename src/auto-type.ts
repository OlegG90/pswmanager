import type { AutoTypeAsk, Entry, EntryChoice, Typed } from './api'
import { button, el } from './dom'
import { titleOf } from './entry-text'
import { search } from './search'

/** What to type into the window: the entry, and what of it. */
export interface TypeChoice {
  id: string
  typed: Typed
}

/** What the choice types into: the window's title, and a browser's site. */
export function typeTarget(ask: AutoTypeAsk): string {
  const where = ask.site ?? (ask.browser ? 'a page whose address could not be read' : null)
  return `Type into “${ask.window || 'the window'}”${where ? ` · ${where}` : ''}`
}

/**
 * The choice of the entry Auto-Type types into the window its hotkey was
 * pressed in (#272), for its chooser (#307): the entries that fit the window,
 * or those a search over every entry in use finds. ↑ / ↓ choose, Enter types
 * the entry's sequence; the buttons type one value alone. `answer` gets the choice.
 */
export function typeChoice(ask: AutoTypeAsk, entries: Entry[], answer: (choice: TypeChoice) => void) {
  const query = el('input', { className: 'text', placeholder: 'Search every entry', spellcheck: false, ariaLabel: 'Search every entry' })
  const note = el('p', { className: 'muted' })
  const list = el('ul', { className: 'type-choices' })
  list.setAttribute('role', 'listbox')
  let shown: EntryChoice[] = []
  let active = 0
  const pick = (typed: Typed) => {
    const chosen = shown[active]
    if (chosen) answer({ id: chosen.id, typed })
  }
  const buttons = [
    button('Type', 'Type the entry\'s sequence (Enter)', () => pick('sequence'), 'primary'),
    button('User name', 'Type the user name alone', () => pick('username')),
    button('Password', 'Type the password alone', () => pick('password')),
    button('TOTP', 'Type the TOTP code alone', () => pick('totp')),
  ]
  const draw = () => {
    const words = query.value.trim()
    shown = words ? search(entries, words).map((e) => ({ id: e.id, title: e.title, username: e.username })) : ask.matches
    active = Math.max(0, Math.min(active, shown.length - 1))
    note.textContent = shown.length ? (words ? '' : 'These fit the window:')
      : words ? 'Nothing found'
      : ask.browser ? 'No entry is for this site: search for one'
      : 'No entry fits this window: search for one'
    list.replaceChildren(...shown.map((choice, i) => {
      const item = el('li', { className: 'type-choice' }, el('span', { className: 'title' }, titleOf(choice)), el('span', { className: 'subtitle' }, choice.username))
      item.setAttribute('role', 'option')
      item.setAttribute('aria-selected', String(i === active))
      item.addEventListener('click', () => {
        active = i
        draw()
        query.focus()
      })
      item.addEventListener('dblclick', () => pick('sequence'))
      return item
    }))
    list.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' })
    for (const b of buttons) b.disabled = shown.length === 0
  }
  query.addEventListener('input', () => {
    active = 0
    draw()
  })
  query.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      active += e.key === 'ArrowDown' ? 1 : -1
      draw()
    } else if (e.key === 'Enter') {
      e.preventDefault()
      pick('sequence')
    }
  })
  draw()
  return { body: [query, note, list], buttons, focus: query }
}
