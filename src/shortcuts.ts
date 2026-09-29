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
    const hotkey = await api.settings().then((s) => shownCombo(s.hotkey), () => null)
    const lines: Pick<Shortcut, 'group' | 'keys' | 'does'>[] = [
      ...(hotkey ? [{ group: 'Window' as const, keys: hotkey, does: 'Show / hide the window, from any app' }] : []),
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
