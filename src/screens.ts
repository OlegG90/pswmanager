/** The tools, each shown beside the sidebar. */
export type Tool = 'home' | 'health' | 'similar' | 'import' | 'gauth' | 'export'

/** What a locked window shows: the unlock screen, or the choose-database
 *  screen (`back`: there is a database to go back to; none on the first run). */
type Locked = { kind: 'unlock' } | { kind: 'choose'; back: boolean }

/** The list and the entry, beside the sidebar of an unlocked database. */
type Records = { kind: 'records' }

/**
 * The screen the window shows. A tool's report takes the records' place
 * beside the sidebar; the settings are over the records while unlocked, and
 * over the whole window while locked, and go back to what they are over.
 */
export type Screen = Locked | Records | { kind: 'report'; tool: Tool } | { kind: 'settings'; over: Locked | Records }

/** What a key does on the screen shown, and whether the key ends there. */
export interface ScreenKey {
  does:
    | 'hide-window'
    | 'home'
    | 'lock'
    | 'close-settings'
    | 'choose-back'
    | 'settings'
    /** On Home: the records' shortcuts that need no entry. */
    | 'home-shortcuts'
    /** Over the list and the entry: their shortcuts, or the editor's. */
    | 'records-shortcuts'
  prevent: boolean
}

/**
 * Which screen the window shows, and what goes with showing it: an unlock
 * under way, Windows Hello's prompt, a copy of the database that needs a key.
 * It holds the rules on that state and nothing of the page: the window draws
 * what it says, and asks it what a key does.
 */
export function screenState() {
  let screen: Screen = { kind: 'unlock' }
  /** The copy that needs a key, as the app last said (`key-needed`); null when none does. */
  let keyNeededFor: string | null = null
  /** A key was asked for in a dialog since the unlock: that happens once, then the button stays. */
  let keyAsked = false

  /** What is shown once the settings close. */
  const under = () => (screen.kind === 'settings' ? screen.over : screen)
  /** `shown` in place of what the settings are over, the settings staying over it. */
  const show = (shown: Locked | Records) => {
    screen = screen.kind === 'settings' ? { kind: 'settings', over: shown } : shown
  }
  const unlocked = () => under().kind === 'records' || under().kind === 'report'

  const state = {
    /** This window is unlocking: the `unlocked` it causes is not the chooser's. */
    unlocking: false,
    /** Windows Hello is offered for the current database (#235). */
    helloReady: false,
    /** Windows Hello's prompt is open: another one waits for it. */
    helloOpen: false,

    get screen() {
      return screen
    },
    get unlocked() {
      return unlocked()
    },
    /** The settings are over the records or the locked window, whichever is current. */
    get settingsOpen() {
      return screen.kind === 'settings'
    },
    /** The tool whose report is beside the sidebar, marked there; null without one. */
    get reportTool(): Tool | null {
      return screen.kind === 'report' ? screen.tool : null
    },
    /** A page is shown in the records' place or over the window: a tool's
     *  report, or the settings. */
    get pageOpen() {
      return screen.kind === 'settings' || screen.kind === 'report'
    },
    get keyNeededFor() {
      return keyNeededFor
    },

    /** The unlock screen of the current database. */
    showUnlock() {
      show({ kind: 'unlock' })
    },
    /** The choose-database screen; `back` when there is a database to go back to. */
    showChoose(back: boolean) {
      show({ kind: 'choose', back })
    },
    /** The database was unlocked: its records, under the settings when they
     *  were open; what an unlocked window shows stays. A key is asked for anew. */
    unlock() {
      if (!unlocked()) show({ kind: 'records' })
      keyAsked = false
    },
    /** The database locked, from any screen: the unlock screen, the settings
     *  and a report closed. */
    lock() {
      screen = { kind: 'unlock' }
    },
    /** A tool's report in the records' place (another report's, too). */
    openReport(tool: Tool) {
      if (unlocked() && screen.kind !== 'settings') screen = { kind: 'report', tool }
    },
    closeReport() {
      if (screen.kind === 'report') screen = { kind: 'records' }
    },
    /** The settings over what is shown; a report closes first. */
    openSettings() {
      if (screen.kind === 'settings') return
      screen = { kind: 'settings', over: screen.kind === 'report' ? { kind: 'records' } : screen }
    },
    closeSettings() {
      screen = under()
    },

    /** The `unlocked` event is another window's (Auto-Type's chooser, #307):
     *  this one follows it. */
    followsUnlock: () => !unlocked() && !state.unlocking,

    /** What the key `e` does on the screen shown; null leaves it alone. */
    keyFor(e: Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey'>): ScreenKey | null {
      const escape = e.key === 'Escape'
      const lock = e.ctrlKey && e.code === 'KeyL'
      switch (screen.kind) {
        case 'report':
          if (escape) return { does: screen.tool === 'home' ? 'hide-window' : 'home', prevent: true }
          if (lock) return { does: 'lock', prevent: true }
          // On Home the keys that need no entry work as over the list.
          return screen.tool === 'home' ? { does: 'home-shortcuts', prevent: false } : null
        case 'settings':
          if (escape) return { does: 'close-settings', prevent: true }
          return lock && unlocked() ? { does: 'lock', prevent: true } : null
        case 'unlock':
        case 'choose':
          if (escape) return { does: screen.kind === 'choose' && screen.back ? 'choose-back' : 'hide-window', prevent: false }
          return e.ctrlKey && e.code === 'Comma' ? { does: 'settings', prevent: true } : null
        case 'records':
          return { does: 'records-shortcuts', prevent: false }
      }
    },

    /** The app said which copy needs a key (`key-needed`); null when none does. */
    keyNeeded(copy: string | null) {
      keyNeededFor = copy
    },
    /** The copy to ask the key for in a dialog now, if any: once per unlock
     *  unless `again` (the button), and not while the window is `busy` with a
     *  question or the editor, locked, or showing the settings. */
    askForKey(again: boolean, busy: boolean): string | null {
      if (!keyNeededFor || !unlocked() || busy || screen.kind === 'settings' || (keyAsked && !again)) return null
      keyAsked = true
      return keyNeededFor
    },
  }
  return state
}
