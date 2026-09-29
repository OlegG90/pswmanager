import { describe, expect, it } from 'vitest'
import { actionFor, SHORTCUTS, type Action, type KeyContext } from './keys'

const press = (key: string, ctrlKey = false, code = `Key${key.toUpperCase()}`) => ({
  key,
  code,
  ctrlKey,
  altKey: false,
  metaKey: false,
})
const outside: KeyContext = { inTextField: false, inSelect: false, hasSelection: false }
const inSearch: KeyContext = { ...outside, inTextField: true }

describe('actionFor', () => {
  it('maps the entry shortcuts', () => {
    expect(actionFor(press('b', true), inSearch)).toBe('copy-username')
    expect(actionFor(press('C', true), inSearch)).toBe('copy-password')
    expect(actionFor(press('u', true), outside)).toBe('open-url')
    expect(actionFor(press('h', true), outside)).toBe('toggle-password')
    expect(actionFor(press('l', true), outside)).toBe('lock')
    expect(actionFor(press('t', true), inSearch)).toBe('copy-totp')
    expect(actionFor(press('n', true), inSearch)).toBe('new-entry')
    expect(actionFor(press('e', true), inSearch)).toBe('edit-entry')
    expect(actionFor(press(',', true, 'Comma'), inSearch)).toBe('settings')
    expect(actionFor(press('Delete', false, 'Delete'), outside)).toBe('delete-entry')
    expect(actionFor(press('Delete', false, 'Delete'), inSearch)).toBeNull()
    expect(actionFor(press('ArrowDown'), inSearch)).toBe('next')
    expect(actionFor(press('Escape'), inSearch)).toBe('escape')
  })

  it('follows the physical key, whatever the layout', () => {
    expect(actionFor(press('р', true, 'KeyH'), outside)).toBe('toggle-password')
    expect(actionFor(press('с', true, 'KeyC'), outside)).toBe('copy-password')
  })

  it('leaves Ctrl+C to the browser when text is selected', () => {
    expect(actionFor(press('c', true), { ...inSearch, hasSelection: true })).toBeNull()
  })

  it('leaves the arrow keys to a focused drop-down', () => {
    expect(actionFor(press('ArrowDown'), { ...outside, inSelect: true })).toBeNull()
  })

  it('opens the shortcuts with F1', () => {
    expect(actionFor(press('F1', false, 'F1'), inSearch)).toBe('shortcuts')
  })

  it('lists every key it reacts to in the shortcuts panel', () => {
    // Each Ctrl shortcut gives what the panel says.
    for (const s of SHORTCUTS.filter((s) => s.ctrl)) {
      expect(actionFor({ ...press('x', true, s.ctrl), key: 'x' }, outside), s.keys).toBe(s.actions[0])
    }
    // And every action a key can give is on the panel.
    const presses = [
      ...'ABCDEFGHIJKLMNOPQRSTUVWXYZ'.split('').map((l) => press(l.toLowerCase(), true)),
      press(',', true, 'Comma'), press('.', true, 'Period'), press('/', true, 'Slash'),
      ...['ArrowUp', 'ArrowDown', 'Escape', 'Delete', 'F1', 'Tab', 'Enter', 'g'].map((k) => press(k, false, k)),
    ]
    const given = new Set(presses.map((p) => actionFor(p, outside)).filter((a): a is Action => a !== null))
    const listed = new Set(SHORTCUTS.flatMap((s) => s.actions))
    for (const action of given) expect(listed.has(action), action).toBe(true)
  })

  it('starts a search when typing outside a text field', () => {
    expect(actionFor(press('g'), outside)).toBe('type-to-search')
    expect(actionFor(press('g'), inSearch)).toBeNull()
    expect(actionFor(press(' '), outside)).toBeNull()
    expect(actionFor(press('Tab'), outside)).toBeNull()
    expect(actionFor({ ...press('b', true), altKey: true }, outside)).toBeNull()
  })
})
