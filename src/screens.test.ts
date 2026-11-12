import { describe, expect, it } from 'vitest'
import { screenState, type ScreenKey } from './screens'

const ESCAPE = { key: 'Escape', code: 'Escape', ctrlKey: false }
const CTRL_L = { key: 'l', code: 'KeyL', ctrlKey: true }
const CTRL_COMMA = { key: ',', code: 'Comma', ctrlKey: true }
const LETTER = { key: 'a', code: 'KeyA', ctrlKey: false }

/** A window with its database unlocked. */
function unlocked() {
  const screens = screenState()
  screens.unlock()
  return screens
}

/** What the key does, without whether it ends there. */
const does = (key: ScreenKey | null) => key?.does ?? null

describe('screenState', () => {
  it('starts locked, on the unlock screen', () => {
    const screens = screenState()
    expect(screens.screen).toEqual({ kind: 'unlock' })
    expect([screens.unlocked, screens.settingsOpen, screens.reportTool, screens.plain]).toEqual([false, false, null, true])
  })

  it('shows a report in the records\' place, one at a time', () => {
    const screens = unlocked()
    expect(screens.screen).toEqual({ kind: 'records' })
    screens.openReport('home')
    screens.openReport('health')
    expect([screens.reportTool, screens.plain, screens.unlocked]).toEqual(['health', false, true])
    screens.closeReport()
    expect(screens.screen).toEqual({ kind: 'records' })
    // Closing with none open changes nothing.
    screens.closeReport()
    expect(screens.screen).toEqual({ kind: 'records' })
  })

  it('opens no report while locked', () => {
    const screens = screenState()
    screens.openReport('home')
    expect(screens.screen).toEqual({ kind: 'unlock' })
  })

  it('puts the settings over the records, a report closed first', () => {
    const screens = unlocked()
    screens.openReport('similar')
    screens.openSettings()
    expect([screens.settingsOpen, screens.reportTool, screens.unlocked, screens.plain]).toEqual([true, null, true, false])
    // A report is not opened under the settings: they close first.
    screens.openReport('home')
    expect(screens.reportTool).toBeNull()
    screens.closeSettings()
    expect(screens.screen).toEqual({ kind: 'records' })
  })

  it('puts the settings over the locked window, and goes back to what they were over', () => {
    const screens = screenState()
    screens.openSettings()
    expect([screens.settingsOpen, screens.unlocked]).toEqual([true, false])
    screens.closeSettings()
    expect(screens.screen).toEqual({ kind: 'unlock' })

    screens.showChoose(true)
    screens.openSettings()
    // Opened again while open, they stay over the same screen.
    screens.openSettings()
    screens.closeSettings()
    expect(screens.screen).toEqual({ kind: 'choose', back: true })
  })

  it('changes the locked screen under the open settings', () => {
    const screens = screenState()
    screens.openSettings()
    // A database named by a second launch was chosen meanwhile.
    screens.showChoose(false)
    expect(screens.settingsOpen).toBe(true)
    screens.showUnlock()
    expect(screens.settingsOpen).toBe(true)
    screens.closeSettings()
    expect(screens.screen).toEqual({ kind: 'unlock' })
  })

  it('keeps the settings open through an unlock, over the records from then on', () => {
    const screens = screenState()
    screens.openSettings()
    screens.unlock()
    expect([screens.settingsOpen, screens.unlocked]).toEqual([true, true])
    screens.closeSettings()
    expect(screens.screen).toEqual({ kind: 'records' })
  })

  it('locks from any screen to the unlock screen', () => {
    const records = unlocked()
    const report = unlocked()
    report.openReport('gauth')
    const settings = unlocked()
    settings.openSettings()
    const choose = screenState()
    choose.showChoose(true)
    for (const screens of [records, report, settings, choose]) {
      screens.lock()
      expect(screens.screen).toEqual({ kind: 'unlock' })
      expect([screens.unlocked, screens.settingsOpen, screens.reportTool]).toEqual([false, false, null])
    }
  })

  it('follows an unlock made in Auto-Type\'s chooser, not its own', () => {
    const screens = screenState()
    expect(screens.followsUnlock()).toBe(true)
    // This window unlocks: the event it causes is not followed.
    screens.unlocking = true
    expect(screens.followsUnlock()).toBe(false)
    screens.unlock()
    screens.unlocking = false
    expect(screens.followsUnlock()).toBe(false)
    screens.lock()
    expect(screens.followsUnlock()).toBe(true)
    // With the settings over the unlock screen too.
    screens.openSettings()
    expect(screens.followsUnlock()).toBe(true)
  })
})

describe('the keys of each screen', () => {
  it('Esc hides the locked window, or goes back from choosing a database', () => {
    const screens = screenState()
    expect(screens.keyFor(ESCAPE)).toEqual({ does: 'hide-window', prevent: false })
    screens.showChoose(false) // the first run: no database to go back to
    expect(does(screens.keyFor(ESCAPE))).toBe('hide-window')
    screens.showChoose(true)
    expect(screens.keyFor(ESCAPE)).toEqual({ does: 'choose-back', prevent: false })
  })

  it('a locked window opens the settings, and has nothing to lock', () => {
    const screens = screenState()
    expect(screens.keyFor(CTRL_COMMA)).toEqual({ does: 'settings', prevent: true })
    expect(screens.keyFor(CTRL_L)).toBeNull()
    expect(screens.keyFor(LETTER)).toBeNull()
  })

  it('the records have their own shortcuts', () => {
    const screens = unlocked()
    for (const key of [ESCAPE, CTRL_L, CTRL_COMMA, LETTER]) {
      expect(screens.keyFor(key)).toEqual({ does: 'records-shortcuts', prevent: false })
    }
  })

  it('Esc goes from a report to Home, and from Home to the tray', () => {
    const screens = unlocked()
    screens.openReport('health')
    expect(screens.keyFor(ESCAPE)).toEqual({ does: 'home', prevent: true })
    expect(screens.keyFor(CTRL_L)).toEqual({ does: 'lock', prevent: true })
    // A report keeps its other keys to itself.
    expect(screens.keyFor(LETTER)).toBeNull()
    expect(screens.keyFor(CTRL_COMMA)).toBeNull()

    screens.openReport('home')
    expect(screens.keyFor(ESCAPE)).toEqual({ does: 'hide-window', prevent: true })
    expect(does(screens.keyFor(CTRL_L))).toBe('lock')
    expect(screens.keyFor(LETTER)).toEqual({ does: 'home-shortcuts', prevent: false })
  })

  it('Esc closes the settings; they lock only an unlocked database', () => {
    const screens = screenState()
    screens.openSettings()
    expect(screens.keyFor(ESCAPE)).toEqual({ does: 'close-settings', prevent: true })
    expect(screens.keyFor(CTRL_L)).toBeNull()
    expect(screens.keyFor(CTRL_COMMA)).toBeNull()
    screens.unlock()
    expect(does(screens.keyFor(ESCAPE))).toBe('close-settings')
    expect(screens.keyFor(CTRL_L)).toEqual({ does: 'lock', prevent: true })
    expect(screens.keyFor(LETTER)).toBeNull()
  })
})

describe('a copy that needs a key', () => {
  it('is asked for in a dialog once per unlock, and again from the button', () => {
    const screens = unlocked()
    expect(screens.askForKey(false, false)).toBeNull()
    screens.keyNeeded('the remote file')
    expect(screens.askForKey(false, false)).toBe('the remote file')
    expect(screens.askForKey(false, false)).toBeNull()
    expect(screens.askForKey(true, false)).toBe('the remote file')
    // The next unlock asks anew.
    screens.lock()
    screens.unlock()
    expect(screens.askForKey(false, false)).toBe('the remote file')
    screens.keyNeeded(null)
    expect(screens.askForKey(true, false)).toBeNull()
  })

  it('waits while locked, busy or under the settings, and is asked for after', () => {
    const screens = screenState()
    screens.keyNeeded('the file on this PC')
    expect(screens.askForKey(false, false)).toBeNull()
    screens.unlock()
    expect(screens.askForKey(false, true)).toBeNull()
    screens.openSettings()
    expect(screens.askForKey(true, false)).toBeNull()
    screens.closeSettings()
    // None of those counted as asked.
    expect(screens.askForKey(false, false)).toBe('the file on this PC')
    screens.openReport('home')
    expect(screens.askForKey(true, false)).toBe('the file on this PC')
  })
})
