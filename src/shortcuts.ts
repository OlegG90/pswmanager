import { api } from './api'
import { el } from './dom'
import { SHORTCUTS, type Shortcut } from './keys'
import { dialog } from './modal'

const GROUPS: Shortcut['group'][] = ['Window', 'List', 'Entry']

/** The keyboard shortcuts in a dialog, by group: the show / hide hotkey as
 *  set now first, then every shortcut of the window. */
export async function showShortcuts() {
  const hotkey = await api.settings().then((s) => s.hotkey.replace('Super', 'Win'), () => null)
  const lines = [
    ...(hotkey ? [{ group: 'Window', keys: hotkey, does: 'Show / hide the window, from any app' }] : []),
    ...SHORTCUTS,
  ]
  const groups = GROUPS.map((group) =>
    el('section', {}, el('h3', {}, group),
      el('dl', {}, ...lines.filter((l) => l.group === group).flatMap((l) => [el('dt', {}, el('kbd', {}, l.keys)), el('dd', {}, l.does)]))))
  await dialog('Keyboard shortcuts', undefined, () => ({ body: groups }), 'Close', 'shortcuts')
}
