import { describe, expect, it } from 'vitest'
import type { Module } from './api'
import { moduleGroups, moduleHint, switchable, turned } from './modules-settings'

const module = (kind: Module['kind'], id: string, extra: Partial<Module> = {}): Module => ({
  id, kind, label: id, hint: `${id}'s hint.`, enabled: true, extensions: [], needsNetwork: null, optional: true, inUse: false, ...extra,
})

const all = [
  module('importers', 'csv'),
  module('stores', 'folder', { optional: false }),
  module('stores', 'dropbox', { inUse: true }),
  module('stores', 'google'),
  module('stores', 'document', { optional: false }),
  module('fillers', 'login'),
  module('fillers', 'card'),
]

describe('Settings › Modules, as both apps behave', () => {
  it('offers the cloud stores, and the autofill categories on the phone only', () => {
    const ids = (phone: boolean) => moduleGroups(all, phone).map((g) => [g.title, g.modules.map((m) => m.id)])
    expect(ids(false)).toEqual([['Stores', ['dropbox', 'google']]])
    expect(ids(true)).toEqual([['Stores', ['dropbox', 'google']], ['Autofill categories', ['login', 'card']]])
  })

  it('keeps a store in use on, saying why', () => {
    const dropbox = all[2]
    expect(switchable(dropbox)).toBe(false)
    expect(moduleHint(dropbox)).toContain('stop syncing there')
    // Turned off by an older setting, it can be turned back on.
    expect(switchable({ ...dropbox, enabled: false })).toBe(true)
    expect(moduleHint(all[3])).toBe("google's hint.")
  })

  it('saves the keys turned off, sorted', () => {
    expect(turned(['stores:google'], all[6], false)).toEqual(['fillers:card', 'stores:google'])
    expect(turned(['fillers:card', 'stores:google'], all[6], true)).toEqual(['stores:google'])
  })
})
