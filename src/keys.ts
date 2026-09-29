export type Action =
  | 'copy-username'
  | 'copy-password'
  | 'open-url'
  | 'toggle-password'
  | 'lock'
  | 'copy-totp'
  | 'new-entry'
  | 'edit-entry'
  | 'delete-entry'
  | 'previous'
  | 'next'
  | 'escape'
  | 'type-to-search'
  | 'settings'
  | 'shortcuts'

/** One line of the shortcuts panel: the keys as shown, what they do, and the
 *  actions they give ([actionFor]); `ctrl` is the physical key pressed with Ctrl. */
export interface Shortcut {
  group: 'Window' | 'List' | 'Entry'
  keys: string
  does: string
  actions: Action[]
  ctrl?: string
}

/** Every shortcut of the unlocked window. [actionFor] reads the Ctrl ones from
 *  here, so the panel cannot drift from what the keys do. The global hotkey,
 *  which the settings change, is shown apart. */
export const SHORTCUTS: Shortcut[] = [
  { group: 'Window', keys: 'Esc', does: 'Clear the search, close what is open, then hide to the tray', actions: ['escape'] },
  { group: 'Window', keys: 'Ctrl+L', does: 'Lock', actions: ['lock'], ctrl: 'KeyL' },
  { group: 'Window', keys: 'Ctrl+,', does: 'Settings', actions: ['settings'], ctrl: 'Comma' },
  { group: 'Window', keys: 'F1', does: 'These shortcuts', actions: ['shortcuts'] },
  { group: 'List', keys: 'Type anywhere', does: 'Search', actions: ['type-to-search'] },
  { group: 'List', keys: '↑ / ↓', does: 'Move in the list', actions: ['previous', 'next'] },
  { group: 'List', keys: 'Ctrl+click / Shift+click', does: 'Choose several entries', actions: [] },
  { group: 'Entry', keys: 'Ctrl+B', does: 'Copy the user name', actions: ['copy-username'], ctrl: 'KeyB' },
  { group: 'Entry', keys: 'Ctrl+C', does: 'Copy the password (unless text is selected)', actions: ['copy-password'], ctrl: 'KeyC' },
  { group: 'Entry', keys: 'Ctrl+T', does: 'Copy the TOTP code', actions: ['copy-totp'], ctrl: 'KeyT' },
  { group: 'Entry', keys: 'Ctrl+U', does: 'Open the URL in the browser', actions: ['open-url'], ctrl: 'KeyU' },
  { group: 'Entry', keys: 'Ctrl+H', does: 'Show / hide the password', actions: ['toggle-password'], ctrl: 'KeyH' },
  { group: 'Entry', keys: 'Ctrl+N', does: 'New entry (blank or from a template)', actions: ['new-entry'], ctrl: 'KeyN' },
  { group: 'Entry', keys: 'Ctrl+E', does: 'Edit the entry', actions: ['edit-entry'], ctrl: 'KeyE' },
  { group: 'Entry', keys: 'Del', does: 'Delete (in the trash: delete for good)', actions: ['delete-entry'] },
]

/** `code` is the physical key: shortcuts follow it, so they work in any keyboard layout. */
export type KeyInfo = Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'altKey' | 'metaKey'>

export interface KeyContext {
  /** Focus is in a text field (search or password). */
  inTextField: boolean
  /** Focus is in a drop-down, which needs the arrow keys itself. */
  inSelect: boolean
  /** Text is selected on the page or in a field: Ctrl+C copies that instead. */
  hasSelection: boolean
}

/** What a key press in the unlocked window does; null leaves it to the browser. */
export function actionFor(e: KeyInfo, ctx: KeyContext): Action | null {
  if (e.altKey || e.metaKey) return null
  if (e.ctrlKey) {
    const action = SHORTCUTS.find((s) => s.ctrl === e.code)?.actions[0] ?? null
    return action === 'copy-password' && ctx.hasSelection ? null : action
  }
  switch (e.key) {
    case 'ArrowUp':
      return ctx.inSelect ? null : 'previous'
    case 'ArrowDown':
      return ctx.inSelect ? null : 'next'
    case 'Escape':
      return 'escape'
    case 'Delete':
      return ctx.inTextField ? null : 'delete-entry'
    case 'F1':
      return 'shortcuts'
  }
  // A printable key typed anywhere outside a text field starts a search.
  return !ctx.inTextField && e.key.length === 1 && e.key !== ' ' ? 'type-to-search' : null
}
