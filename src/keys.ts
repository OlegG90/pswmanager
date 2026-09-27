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
    switch (e.code) {
      case 'KeyB':
        return 'copy-username'
      case 'KeyC':
        return ctx.hasSelection ? null : 'copy-password'
      case 'KeyU':
        return 'open-url'
      case 'KeyH':
        return 'toggle-password'
      case 'KeyL':
        return 'lock'
      case 'KeyT':
        return 'copy-totp'
      case 'KeyN':
        return 'new-entry'
      case 'KeyE':
        return 'edit-entry'
      case 'Comma':
        return 'settings'
      default:
        return null
    }
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
  }
  // A printable key typed anywhere outside a text field starts a search.
  return !ctx.inTextField && e.key.length === 1 && e.key !== ' ' ? 'type-to-search' : null
}
