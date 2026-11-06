import { describe, expect, it } from 'vitest'
import type { Entry, Settings, Synced } from './api'
import { appState } from './app-state'
import { ALL, TEMPLATES } from '../../../../src/search'

const synced = (otherKey: boolean) => ({ text: 'Synced', problem: false, changed: false, signIn: false, copyProblem: null, otherKey }) as Synced
const entry = (id: string, kind: Entry['kind'] = 'entry') => ({ id, title: id, username: '', url: '', tags: [], group: [], kind }) as unknown as Entry
const settings = (s: Partial<Settings>) => s as Settings

/** A state with its own clock, set with `at`. */
function clocked() {
  let now = 0
  const app = appState(() => now)
  return { app, at: (ms: number) => (now = ms) }
}

describe('appState', () => {
  it('opens the new-key screen once per key change, and only while unlocked', () => {
    const app = appState()
    app.synced(synced(true))
    expect(app.askForKey()).toBe(false)
    app.unlocked = true
    expect(app.askForKey()).toBe(true)
    expect(app.askForKey()).toBe(false)
    // A sync that finds the key as it is re-arms it.
    app.synced(synced(false))
    app.synced(synced(true))
    expect(app.askForKey()).toBe(true)
  })

  it('forgets the key change only when the page locked', () => {
    const app = appState()
    Object.assign(app, { unlocked: true, otherKey: true, keyAsked: true, query: 'mail' })
    app.leaveList(10, 'a')
    app.locked(false)
    expect([app.unlocked, app.query, app.listPlace, app.otherKey, app.keyAsked]).toEqual([false, '', null, true, true])
    app.locked(true)
    expect([app.otherKey, app.keyAsked]).toEqual([false, false])
  })

  it('gives the list place back once, for the same filter and search', () => {
    const app = appState()
    app.leaveList(120, 'a')
    expect(app.takeListPlace()).toEqual({ top: 120, id: 'a', filter: ALL, query: '' })
    expect(app.takeListPlace()).toBeNull()
    app.leaveList(120, 'a')
    app.query = 'other'
    expect(app.takeListPlace()).toBeNull()
    app.leaveList(120, 'a')
    app.filter = TEMPLATES
    expect(app.takeListPlace()).toBeNull()
  })

  it('shows a saved entry the filter or search leaves out among All, a template among the templates', () => {
    const app = appState()
    app.filter = { kind: 'tag', tag: 'work' }
    app.query = 'x'
    app.showSaved('a', [entry('a')], false)
    expect([app.filter, app.query]).toEqual([ALL, ''])
    app.filter = { kind: 'tag', tag: 'work' }
    app.showSaved('t', [entry('t', 'template')], true)
    expect(app.filter).toEqual(TEMPLATES)
  })

  it('keeps the filter and search when the saved entry shows', () => {
    const app = appState()
    app.query = 'a'
    app.showSaved('a', [entry('a')], false)
    expect([app.filter, app.query]).toEqual([ALL, 'a'])
  })

  it('locks on coming back only after the background time, while unlocked', () => {
    const { app, at } = clocked()
    app.settings = settings({ lockInBackground: 30 })
    app.unlocked = true
    app.wentAway()
    at(30_000)
    expect(app.cameBack()).toBe(false)
    at(30_001)
    expect(app.cameBack()).toBe(true)
    app.settings = settings({ lockInBackground: null })
    expect(app.cameBack()).toBe(false)
    app.settings = settings({ lockInBackground: 30 })
    app.unlocked = false
    expect(app.cameBack()).toBe(false)
  })

  it('keeps when it went away only while unlocked', () => {
    const { app, at } = clocked()
    at(5_000)
    app.wentAway()
    expect(app.hiddenAt).toBe(0)
    app.unlocked = true
    app.wentAway()
    expect(app.hiddenAt).toBe(5_000)
  })

  it('counts as just returned for three seconds', () => {
    const { app, at } = clocked()
    at(10_000)
    app.cameBack()
    at(12_999)
    expect(app.justReturned()).toBe(true)
    at(13_000)
    expect(app.justReturned()).toBe(false)
  })

  it('locks when untouched for the setting (5 min unless set, never with 0)', () => {
    const { app, at } = clocked()
    app.unlocked = true
    at(5 * 60_000)
    expect(app.idle()).toBe(false)
    at(5 * 60_000 + 1)
    expect(app.idle()).toBe(true)
    app.touched()
    expect(app.idle()).toBe(false)
    app.settings = settings({ lockAfterMinutes: 0 })
    at(60 * 60_000)
    expect(app.idle()).toBe(false)
    app.unlocked = false
    app.settings = settings({ lockAfterMinutes: 1 })
    expect(app.idle()).toBe(false)
  })

  it('syncs on its own when the last sync is old enough, while unlocked', () => {
    const { app, at } = clocked()
    app.unlocked = true
    app.synced(synced(false))
    at(59_999)
    expect(app.syncDue(60_000)).toBe(false)
    at(60_000)
    expect(app.syncDue(60_000)).toBe(true)
    expect(app.returnSyncDue()).toBe(true)
    expect(app.syncDue(0)).toBe(false)
    expect(app.everySyncDue()).toBe(false)
    at(5 * 60_000)
    expect(app.everySyncDue()).toBe(true)
    app.unlocked = false
    expect(app.syncDue(60_000)).toBe(false)
  })

  it('has a sync to start by hand only with a cloud store, and swipes unless turned off', () => {
    const app = appState()
    expect(app.linked()).toBe(false)
    app.database = { title: '', description: '', syncedWith: 'Dropbox', cloud: 'dropbox', copyFolder: null, keyFile: null }
    expect(app.linked()).toBe(true)
    expect(app.swipesOn()).toBe(true)
    app.settings = settings({ swipes: false })
    expect(app.swipesOn()).toBe(false)
  })
})
