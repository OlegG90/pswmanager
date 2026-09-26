import { describe, expect, it } from 'vitest'
import { actionFor, type KeyContext } from './keys'

const press = (key: string, ctrlKey = false, code = `Key${key.toUpperCase()}`) => ({
  key,
  code,
  ctrlKey,
  altKey: false,
  metaKey: false,
})
const outside: KeyContext = { inTextField: false, hasSelection: false }
const inSearch: KeyContext = { inTextField: true, hasSelection: false }

describe('actionFor', () => {
  it('maps the entry shortcuts', () => {
    expect(actionFor(press('b', true), inSearch)).toBe('copy-username')
    expect(actionFor(press('C', true), inSearch)).toBe('copy-password')
    expect(actionFor(press('u', true), outside)).toBe('open-url')
    expect(actionFor(press('h', true), outside)).toBe('toggle-password')
    expect(actionFor(press('l', true), outside)).toBe('lock')
    expect(actionFor(press('ArrowDown'), inSearch)).toBe('next')
    expect(actionFor(press('Escape'), inSearch)).toBe('escape')
  })

  it('follows the physical key, whatever the layout', () => {
    expect(actionFor(press('р', true, 'KeyH'), outside)).toBe('toggle-password')
    expect(actionFor(press('с', true, 'KeyC'), outside)).toBe('copy-password')
  })

  it('leaves Ctrl+C to the browser when text is selected', () => {
    expect(actionFor(press('c', true), { inTextField: true, hasSelection: true })).toBeNull()
  })

  it('starts a search when typing outside a text field', () => {
    expect(actionFor(press('g'), outside)).toBe('type-to-search')
    expect(actionFor(press('g'), inSearch)).toBeNull()
    expect(actionFor(press(' '), outside)).toBeNull()
    expect(actionFor(press('Tab'), outside)).toBeNull()
    expect(actionFor({ ...press('b', true), altKey: true }, outside)).toBeNull()
  })
})
