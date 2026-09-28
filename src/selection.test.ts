import { describe, expect, it } from 'vitest'
import { clicked } from './selection'

const ids = ['a', 'b', 'c', 'd', 'e']
const plain = { ctrl: false, shift: false }
const ctrl = { ctrl: true, shift: false }
const shift = { ctrl: false, shift: true }

describe('choosing entries in the list', () => {
  it('chooses one with a plain click', () => {
    expect(clicked(ids, { chosen: ['a', 'b'], anchor: 'a' }, 'd', plain)).toEqual({ chosen: ['d'], anchor: 'd' })
    expect(clicked(ids, null, 'b', ctrl)).toEqual({ chosen: ['b'], anchor: 'b' })
  })

  it('adds and takes off with Ctrl, never leaving nothing', () => {
    const one = { chosen: ['b'], anchor: 'b' }
    const two = clicked(ids, one, 'd', ctrl)
    expect(two).toEqual({ chosen: ['b', 'd'], anchor: 'd' })
    expect(clicked(ids, two, 'b', ctrl).chosen).toEqual(['d'])
    expect(clicked(ids, one, 'b', ctrl).chosen).toEqual(['b'])
  })

  it('chooses a range with Shift, from the anchor either way', () => {
    expect(clicked(ids, { chosen: ['b'], anchor: 'b' }, 'd', shift)).toEqual({ chosen: ['b', 'c', 'd'], anchor: 'b' })
    expect(clicked(ids, { chosen: ['d'], anchor: 'd' }, 'a', shift).chosen).toEqual(['a', 'b', 'c', 'd'])
    // An anchor no longer listed: a plain choice.
    expect(clicked(ids, { chosen: ['x'], anchor: 'x' }, 'c', shift)).toEqual({ chosen: ['c'], anchor: 'c' })
  })
})
