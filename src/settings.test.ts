import { describe, expect, it } from 'vitest'
import { comboOf } from './settings'

const press = (key: string, code: string, mods: Partial<KeyboardEvent> = {}) =>
  ({ key, code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods }) as KeyboardEvent

describe('comboOf', () => {
  it('names the keys by position, in any layout', () => {
    expect(comboOf(press('p', 'KeyP', { ctrlKey: true, altKey: true }))).toBe('Ctrl+Alt+P')
    expect(comboOf(press('з', 'KeyP', { ctrlKey: true, altKey: true }))).toBe('Ctrl+Alt+P')
    expect(comboOf(press('1', 'Digit1', { metaKey: true, shiftKey: true }))).toBe('Shift+Super+1')
    expect(comboOf(press('F8', 'F8', { ctrlKey: true }))).toBe('Ctrl+F8')
  })

  it('waits while only modifiers are down', () => {
    expect(comboOf(press('Control', 'ControlLeft', { ctrlKey: true }))).toBeNull()
    expect(comboOf(press('Alt', 'AltLeft', { altKey: true }))).toBeNull()
  })
})
