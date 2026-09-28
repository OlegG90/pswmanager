import { api, type SettingName, type Settings, type Status, type Theme } from './api'
import { button, el } from './dom'
import { setUpSync } from './sync-setup'

type Choice<T = number> = [value: T, label: string]

const minutes = (never: string): Choice[] =>
  [...[1, 5, 15, 30, 60].map((m): Choice => [m, `${m} min`]), [0, never]]
const LOCK_AFTER = minutes('Never')
const SYNC_EVERY = minutes('Off')
const CLEAR_AFTER: Choice[] = [5, 10, 20, 30, 60, 120].map((s) => [s, `${s} s`])
const THEMES: Choice<Theme>[] = [['system', 'As Windows'], ['light', 'Light'], ['dark', 'Dark']]

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

/** A drop-down of named choices. */
function pick<T extends string>(label: string, value: T, choices: Choice<T>[], change: (value: T) => void) {
  const box = el('select', { ariaLabel: label }, ...choices.map(([v, text]) => new Option(text, v, false, v === value)))
  box.addEventListener('change', () => change(box.value as T))
  return box
}

function row(label: string, hint: string, control: HTMLElement): HTMLDivElement {
  return el('div', { className: 'setting' },
    el('div', {}, el('span', {}, label), hint ? el('span', { className: 'hint' }, hint) : ''),
    control)
}

const group = (title: string, ...rows: HTMLElement[]) => el('section', {}, el('h3', {}, title), ...rows)

const MODIFIER_KEYS = ['Control', 'Alt', 'Shift', 'Meta', 'AltGraph']

/**
 * A key combination as the backend reads it, from the keys' positions
 * (`e.code`), so it is the same in every keyboard layout: `Ctrl+Alt+P`.
 */
export function comboOf(e: KeyboardEvent): string | null {
  if (MODIFIER_KEYS.includes(e.key)) return null
  const key = e.code.replace(/^Key/, '').replace(/^Digit/, '')
  const mods = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Super'].filter(Boolean)
  return [...mods, key].join('+')
}

/** The hotkey, and a button that takes the next key combination pressed. */
function hotkeyControl(hotkey: string, change: (value: string) => void): HTMLElement {
  const shown = el('span', { className: 'value' }, hotkey.replace('Super', 'Win'))
  const edit = button('Change…', 'Press the new combination; Esc cancels', () => {
    shown.textContent = 'Press the keys…'
    edit.disabled = true
    const take = (e: KeyboardEvent) => {
      e.preventDefault()
      e.stopPropagation()
      if (e.key === 'Escape') return done(null)
      const combo = comboOf(e)
      if (combo) done(combo)
    }
    const done = (combo: string | null) => {
      document.removeEventListener('keydown', take, true)
      edit.disabled = false
      shown.textContent = hotkey.replace('Super', 'Win')
      if (combo && combo !== hotkey) change(combo)
    }
    document.addEventListener('keydown', take, true)
  })
  return el('span', { className: 'hotkey' }, shown, edit)
}

/**
 * Fills `container` with the settings screen. Each change is saved at once;
 * `onError` reports one that failed, and the screen then shows what is in effect.
 */
export async function renderSettings(container: HTMLElement, onDone: () => void, onError: (message: string) => void) {
  const waiting = el('p', { className: 'muted', hidden: true })
  const [settings, status] = await Promise.all([api.settings(), api.status()])
  draw(settings, status)

  /** Sync for the open database: where it syncs and Stop, or Upload / Link. */
  function syncControl(status: Status): HTMLElement {
    const redraw = async () => draw(await api.settings(), await api.status())
    const run = (action: () => Promise<unknown>) => async () => {
      try {
        await action()
      } catch (e) {
        onError(String(e))
      }
      await redraw()
    }
    if (!status.database) return el('span', { className: 'value' }, 'No database')
    if (status.syncedWith) {
      return el('span', { className: 'sync-control' }, el('span', { className: 'value' }, status.syncedWith),
        button('Stop syncing', 'Keep the file as it is, without sync', run(api.stopSync)))
    }
    return el('span', { className: 'sync-control' }, el('span', { className: 'value' }, 'Not synced'),
      button('Upload…', 'Put this database into a store as a new file, and sync with it', run(() => setUpSync('upload', waiting))),
      button('Link…', 'Sync with a file already in a store', run(() => setUpSync('link', waiting))))
  }

  function draw(s: Settings, status: Status) {
    const set = (name: SettingName) => async (value: number | boolean | string) => {
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
        row('Theme', 'Light or dark, or as Windows is set', pick('Theme', s.theme, THEMES, set('theme'))),
        row('Global hotkey', 'Shows or hides the window from any app', hotkeyControl(s.hotkey, set('hotkey'))),
        row('Start with Windows', 'Starts hidden in the tray, locked',
          toggle('Start with Windows', s.startWithWindows, set('startWithWindows'))),
        row('Download site icons', 'Directly from each site, never through a third party',
          toggle('Download site icons', s.downloadIcons, set('downloadIcons')))),
      group('Sync',
        row('This database', status.database ?? '', syncControl(status)),
        waiting,
        row('Check for remote changes', 'While unlocked',
          select('Check for remote changes', s.syncEveryMinutes, SYNC_EVERY, 'min', set('syncEveryMinutes')))),
    )
    // Redrawing replaces the controls: keep the keyboard where it was.
    if (focused) container.querySelector<HTMLElement>(`[aria-label="${focused}"]`)?.focus()
  }
}
