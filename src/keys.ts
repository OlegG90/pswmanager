export type Action =
  | 'copy-username'
  | 'copy-password'
  | 'open-url'
  | 'toggle-password'
  | 'lock'
  | 'previous'
  | 'next'
  | 'escape'
  | 'type-to-search'

export interface KeyInfo {
  key: string
  /** The physical key: shortcuts follow it, so they work in any keyboard layout. */
  code: string
  ctrlKey: boolean
  altKey: boolean
  metaKey: boolean
}

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
  }
  // A printable key typed anywhere outside a text field starts a search.
  return !ctx.inTextField && e.key.length === 1 && e.key !== ' ' ? 'type-to-search' : null
}
