import { describe, expect, it } from 'vitest'
import type { Entry, Synced } from './api'
import { appState } from './app-state'
import { ALL, TEMPLATES } from '../../../../src/search'

const synced = (otherKey: boolean) => ({ text: 'Synced', problem: false, changed: false, signIn: false, copyProblem: null, otherKey }) as Synced

describe('appState', () => {
  it('opens the new-key screen once per key change found', () => {
    const app = appState()
    app.unlocked = true
    app.synced(synced(true))
    expect(app.askForKey()).toBe(true)
    expect(app.askForKey()).toBe(false)
    app.synced(synced(false))
    app.synced(synced(true))
    expect(app.askForKey()).toBe(true)
  })

  it('forgets the key change only when the page locked', () => {
    const app = appState()
    app.otherKey = true
    app.query = 'mail'
    app.locked(false)
    expect([app.unlocked, app.query, app.otherKey]).toEqual([false, '', true])
    app.locked(true)
    expect(app.otherKey).toBe(false)
  })

  it('gives the list place back only for the same filter and search', () => {
    const app = appState()
    app.leaveList(120, 'a')
    expect(app.takeListPlace()?.top).toBe(120)
    expect(app.takeListPlace()).toBeNull()
    app.leaveList(120, 'a')
    app.query = 'other'
    expect(app.takeListPlace()).toBeNull()
  })

  it('shows a saved entry the filter leaves out among All or the templates', () => {
    const app = appState()
    app.filter = { kind: 'tag', tag: 'work' }
    app.query = 'x'
    app.showSaved('a', [{ id: 'a', title: 'A', username: '', url: '', tags: [], kind: 'template' } as unknown as Entry], true)
    expect([app.filter, app.query]).toEqual([TEMPLATES, ''])
    expect(ALL).toBeDefined()
  })

  it('locks on coming back after the background time, and when idle', () => {
    let now = 0
    const app = appState(() => now)
    app.unlocked = true
    app.settings = { lockInBackground: 30, lockAfterMinutes: 5 } as never
    app.wentAway()
    now = 31_000
    expect(app.cameBack()).toBe(true)
    expect(app.justReturned()).toBe(true)
    expect(app.idle()).toBe(false)
    now += 5 * 60_000 + 1
    expect(app.idle()).toBe(true)
  })

  it('syncs when the last sync is old enough', () => {
    let now = 0
    const app = appState(() => now)
    app.unlocked = true
    app.synced(synced(false))
    now = 59_000
    expect(app.syncDue(60_000)).toBe(false)
    now = 60_000
    expect(app.syncDue(60_000)).toBe(true)
    expect(app.syncDue(0)).toBe(false)
  })
})
