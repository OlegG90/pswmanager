import { api } from './api'
import { el } from './dom'
import { SHORTCUT_GROUPS, shownCombo, SHORTCUTS, type Shortcut } from './keys'
import { dialog } from './modal'

/** The dialog is on its way or up: a second F1 or click does not open another. */
let showing = false

/** The keyboard shortcuts in a dialog, by group: the show / hide hotkey as
 *  set now first, then every shortcut of the window. */
export async function showShortcuts() {
  if (showing) return
  showing = true
  try {
    const settings = await api.settings().catch(() => null)
    const hotkey = settings && shownCombo(settings.hotkey)
    const autoType = settings?.autoTypeHotkey && shownCombo(settings.autoTypeHotkey)
    const lines: Pick<Shortcut, 'group' | 'keys' | 'does'>[] = [
      ...(hotkey ? [{ group: 'Window' as const, keys: hotkey, does: 'Show / hide the window, from any app' }] : []),
      ...(autoType ? [{ group: 'Window' as const, keys: autoType, does: 'Auto-Type: type an entry into the window in front' }] : []),
      ...SHORTCUTS,
    ]
    const groups = SHORTCUT_GROUPS.map((group) =>
      el('section', {}, el('h3', {}, group),
        el('dl', {}, ...lines.filter((l) => l.group === group).flatMap((l) => [el('dt', {}, el('kbd', {}, l.keys)), el('dd', {}, l.does)]))))
    await dialog('Keyboard shortcuts', undefined, () => ({ body: groups }), 'Close', 'shortcuts')
  } finally {
    showing = false
  }
}
