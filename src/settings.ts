import { api, type SettingName, type Settings, type Status } from './api'
import { button, el } from './dom'

type Choice = [value: number, label: string]

const minutes = (never: string): Choice[] =>
  [...[1, 5, 15, 30, 60].map((m): Choice => [m, `${m} min`]), [0, never]]
const LOCK_AFTER = minutes('Never')
const SYNC_EVERY = minutes('Off')
const CLEAR_AFTER: Choice[] = [5, 10, 20, 30, 60, 120].map((s) => [s, `${s} s`])

/** A switch: a button that is on or off. */
function toggle(label: string, on: boolean, change: (on: boolean) => void): HTMLButtonElement {
  const toggle = el('button', { type: 'button', className: 'switch', role: 'switch', ariaLabel: label, onclick: () => change(!on) },
    el('span', { className: 'knob' }))
  toggle.setAttribute('aria-checked', String(on))
  return toggle
}

/** A drop-down of `choices`; a value set outside them (by hand in the state file) is kept as one more. */
function select(label: string, value: number, choices: Choice[], unit: string, change: (value: number) => void) {
  const all = choices.some(([v]) => v === value) ? choices : [...choices, [value, `${value} ${unit}`] as Choice]
  const box = el('select', { ariaLabel: label }, ...all.map(([v, text]) => new Option(text, String(v), false, v === value)))
  box.addEventListener('change', () => change(Number(box.value)))
  return box
}

function row(label: string, hint: string, control: HTMLElement): HTMLDivElement {
  return el('div', { className: 'setting' },
    el('div', {}, el('span', {}, label), hint ? el('span', { className: 'hint' }, hint) : ''),
    control)
}

const group = (title: string, ...rows: HTMLElement[]) => el('section', {}, el('h3', {}, title), ...rows)

/**
 * Fills `container` with the settings screen. Each change is saved at once;
 * `onError` reports one that failed, and the screen then shows what is in effect.
 */
export async function renderSettings(container: HTMLElement, onDone: () => void, onError: (message: string) => void) {
  const [settings, status] = await Promise.all([api.settings(), api.status()])
  draw(settings, status)

  function draw(s: Settings, status: Status) {
    const set = (name: SettingName) => async (value: number | boolean) => {
      try {
        draw(await api.setSetting(name, value), status)
      } catch (e) {
        onError(String(e))
        draw(await api.settings(), status)
      }
    }
    const focused = container.contains(document.activeElement) ? document.activeElement?.getAttribute('aria-label') : null
    container.replaceChildren(
      el('header', {}, el('h1', {}, 'Settings'), button('Done', 'Back (Esc)', onDone)),
      group('Security',
        row('Lock after inactivity', '1–60 minutes, or never',
          select('Lock after inactivity', s.lockAfterMinutes, LOCK_AFTER, 'min', set('lockAfterMinutes'))),
        row('Lock when Windows locks', 'Also when the session is switched',
          toggle('Lock when Windows locks', s.lockOnSessionLock, set('lockOnSessionLock'))),
        row('Lock when hidden to tray', '', toggle('Lock when hidden to tray', s.lockWhenHidden, set('lockWhenHidden')))),
      group('Clipboard',
        row('Clear clipboard after copying', 'Only if it still holds the copied value',
          select('Clear clipboard after copying', s.clearClipboard, CLEAR_AFTER, 's', set('clearClipboard')))),
      group('Window and tray',
        row('Global hotkey', 'Shows or hides the window', el('span', { className: 'value' }, s.hotkey)),
        row('Start with Windows', 'Starts hidden in the tray, locked',
          toggle('Start with Windows', s.startWithWindows, set('startWithWindows'))),
        row('Download site icons', 'Directly from each site, never through a third party',
          toggle('Download site icons', s.downloadIcons, set('downloadIcons')))),
      group('Sync',
        row('Remote store', status.syncedWith ? (status.database ?? '') : 'Choose one on the unlock screen',
          el('span', { className: 'value' }, status.syncedWith ?? 'None')),
        row('Check for remote changes', 'While unlocked',
          select('Check for remote changes', s.syncEveryMinutes, SYNC_EVERY, 'min', set('syncEveryMinutes')))),
    )
    // Redrawing replaces the controls: keep the keyboard where it was.
    if (focused) container.querySelector<HTMLElement>(`[aria-label="${focused}"]`)?.focus()
  }
}
