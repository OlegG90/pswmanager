import { api, type BackupInfo, type DatabaseSetting, type DatabaseSettings, type SettingName, type Settings, type Status, type Theme } from './api'
import { getVersion } from '@tauri-apps/api/app'
import { button, el } from './dom'
import { formatDateTime, formatSize } from './entry-text'
import { shownCombo } from './keys'
import { ask } from './modal'
import { changeMasterKey } from './change-key'
import { changeEncryption, describeEncryption } from './change-encryption'
import { setUpSync } from './sync-setup'

type Choice<T = number> = [value: T, label: string]

const minutes = (never: string): Choice[] =>
  [...[1, 5, 15, 30, 60].map((m): Choice => [m, `${m} min`]), [0, never]]
const LOCK_AFTER = minutes('Never')
const SYNC_EVERY = minutes('Off')
const CLEAR_AFTER: Choice[] = [5, 10, 20, 30, 60, 120].map((s) => [s, `${s} s`])
const THEMES: Choice<Theme>[] = [['system', 'As Windows'], ['light', 'Light'], ['dark', 'Dark']]
const versions = (n: number) => (n === 0 ? 'None' : n === 1 ? '1 version' : `${n} versions`)
const HISTORY_ITEMS: Choice[] = [0, 3, 5, 10, 20, 50, 100].map((n): Choice => [n, versions(n)])
const HISTORY_SIZE: Choice[] = [1, 2, 4, 6, 10, 20, 64].map((m): Choice => [m * 2 ** 20, formatSize(m * 2 ** 20)])

/** `choices` with the file's own value among them: another client may have
 *  set no limit (-1), or a limit these do not offer. */
function withValue(choices: Choice[], value: number, label: (value: number) => string): Choice[] {
  return choices.some(([v]) => v === value) ? choices : [...choices, [value, value < 0 ? 'No limit' : label(value)]]
}

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

const TABS = [['general', 'General'], ['window', 'Window'], ['database', 'Database'], ['sync', 'Sync'], ['backup', 'Backup'], ['about', 'About']] as const
type Tab = (typeof TABS)[number][0]
/** The tab shown: kept while the app runs, through redraws after a change. */
let shownTab: Tab = 'general'

/**
 * The row of tabs: `open(tab)` shows one. ←/→ move to the next or previous
 * one (wrapping), Home and End to the first and last. `off(tab)` says why a
 * tab cannot be opened now: it is then disabled, with that as its title, and skipped.
 */
function tabBar(off: (tab: Tab) => string | null, open: (tab: Tab) => void): HTMLElement {
  const tabs = TABS.map(([tab, label]) => {
    const on = tab === shownTab
    const why = off(tab)
    const button = el('button', {
      type: 'button', id: `settings-tab-${tab}`, role: 'tab', className: 'tab', disabled: why !== null,
      tabIndex: on ? 0 : -1, onclick: () => open(tab),
    }, label)
    if (why) button.title = why
    button.setAttribute('aria-selected', String(on))
    button.setAttribute('aria-controls', 'settings-panel')
    return button
  })
  const bar = el('div', { className: 'tabs', role: 'tablist', ariaLabel: 'Settings' }, ...tabs)
  bar.addEventListener('keydown', (e) => {
    const usable = TABS.map(([tab]) => tab).filter((tab) => off(tab) === null)
    const step = ({ ArrowRight: 1, ArrowLeft: -1 } as Record<string, number>)[e.key]
    const next = step ? usable[(usable.indexOf(shownTab) + step + usable.length) % usable.length]
      : e.key === 'Home' ? usable[0] : e.key === 'End' ? usable[usable.length - 1] : undefined
    if (!next) return
    e.preventDefault()
    open(next)
  })
  return bar
}

/** A text field saved when it changes (Enter, or leaving it); a textarea for several lines. */
function textField(label: string, value: string, change: (value: string) => void, lines = 1) {
  const field = lines > 1
    ? el('textarea', { ariaLabel: label, value, rows: lines, spellcheck: false })
    : el('input', { ariaLabel: label, value, spellcheck: false })
  field.addEventListener('change', () => change(field.value))
  return field
}

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
  const shown = el('span', { className: 'value' }, shownCombo(hotkey))
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
      shown.textContent = shownCombo(hotkey)
      if (combo && combo !== hotkey) change(combo)
    }
    document.addEventListener('keydown', take, true)
  })
  return el('span', { className: 'hotkey' }, shown, edit)
}

/**
 * Fills `container` with the settings screen. Each change is saved at once;
 * `onError` reports one that failed, and the screen then shows what is in effect.
 * `onDatabase` gets the open database's settings after one of them changed.
 */
export async function renderSettings(
  container: HTMLElement,
  onDone: () => void,
  onError: (message: string) => void,
  onDatabase: (settings: DatabaseSettings) => void = () => {},
) {
  const waiting = el('p', { className: 'muted', hidden: true })
  // Locked: no database settings to show.
  const [settings, status, version, saved, savedBackup] = await Promise.all([
    api.settings(), api.status(), getVersion(), api.databaseSettings().catch(() => null), api.backup()])
  let database = saved
  let backup = savedBackup
  draw(settings, status)

  /** The current database's backup: kept in the app's list, so it works
   *  while locked too. Redrawn after each change, which may back up at once. */
  function backupGroup(b: BackupInfo, redraw: () => void): HTMLElement {
    const run = (action: () => Promise<BackupInfo | null>) => async () => {
      try {
        backup = await action()
      } catch (e) {
        onError(String(e))
        backup = await api.backup().catch(() => backup)
      }
      redraw()
    }
    const when = (secs: number | null, none: string) => (secs === null ? none : formatDateTime(new Date(secs * 1000).toISOString()))
    const intervals = b.intervals.map((days): Choice => [days, days ? `${days} days` : 'Never'])
    // The path, however long, wraps under the label; the buttons stay beside it.
    const folder = el('span', { className: 'sync-control' },
      button('Select…', 'The folder the copy goes to', run(api.pickBackupFolder)),
      ...(b.folder ? [button('Show', 'Open the folder in Explorer', () => api.showBackupFolder().catch((e) => onError(String(e))))] : []))
    return group('Backup',
      row('Back up every', 'A copy of the database file, encrypted as it is; one copy, replaced each time',
        select('Back up every', b.everyDays, intervals, 'days', (days) => run(() => api.setBackupEvery(days))())),
      row('Backup folder', b.folder ?? 'None: without one, there is no backup', folder),
      row('Last backup', b.error ?? '', el('span', { className: 'value' }, when(b.last, 'Never'))),
      row('Next backup', b.folder && b.everyDays ? '' : 'Choose a folder and how often', el('span', { className: 'value' }, when(b.next, '—'))),
      row('Back up now', '', button('Backup now', 'Copy the database file now', run(api.backupNow))))
  }

  /** Settings kept in the database file: saved and synced like an edit.
   *  Redrawn after each, which shows the value as saved (trimmed). */
  function databaseGroup(d: DatabaseSettings, keyFile: string | null, redraw: (d: DatabaseSettings, status?: Status) => void): HTMLElement {
    const set = (setting: DatabaseSetting) => async (value: string) => {
      try {
        d = await api.setDatabaseSetting(setting, value)
        onDatabase(d)
      } catch (e) {
        onError(String(e))
      }
      redraw(d)
    }
    /** New history limits; when they remove versions, only after saying how many. */
    const setLimits = async (maxItems: number, maxSize: number) => {
      try {
        const going = await api.historyLimitsPreview(maxItems, maxSize)
        const versions = going === 1 ? '1 older version' : `${going} older versions`
        if (!going || await ask(`${versions} will be removed from the entries' history, in this file and in its synced copies.`, 'Remove')) {
          d = await api.setHistoryLimits(maxItems, maxSize)
          onDatabase(d)
        }
      } catch (e) {
        onError(String(e))
      }
      redraw(d)
    }
    /** The cipher and key derivation, in a dialog. */
    const changeCrypto = async () => {
      const next = await changeEncryption(d.encryption)
      if (next) {
        onDatabase(next)
        redraw(next)
      }
    }
    /** The master password and / or key file, in a dialog; the key file shown follows. */
    const changeKey = async () => {
      const status = await changeMasterKey(keyFile)
      if (status) redraw(d, status)
    }
    return group('Database',
      row('Name', 'On the unlock screen and in the title; the file keeps its name', textField('Name', d.name, set('name'))),
      row('Description', 'Under the name on the unlock screen', textField('Description', d.description, set('description'), 2)),
      row('Default user name', 'Filled in on a new blank entry', textField('Default user name', d.defaultUsername, set('defaultUsername'))),
      row('History: versions per entry', 'Older versions each entry keeps',
        select('History: versions per entry', d.historyMaxItems, withValue(HISTORY_ITEMS, d.historyMaxItems, versions), '',
          (n) => setLimits(n, d.historyMaxSize))),
      row('History: size per entry', 'The oldest versions go first when they are larger together',
        select('History: size per entry', d.historyMaxSize, withValue(HISTORY_SIZE, d.historyMaxSize, formatSize), '',
          (bytes) => setLimits(d.historyMaxItems, bytes))),
      row('Master password and key file', keyFile ? `Key file: ${keyFile}` : 'No key file',
        button('Change…', 'Change the master password and / or key file', changeKey)),
      row('Encryption', describeEncryption(d.encryption),
        button('Change…', 'Change the cipher and key derivation', changeCrypto)))
  }

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
    const focused = container.contains(document.activeElement) ? document.activeElement : null
    const focusedLabel = focused?.getAttribute('aria-label')
    const onTab = focused?.getAttribute('role') === 'tab'
    // The database's own settings while it is unlocked; its backup while there is one.
    const off = (tab: Tab) =>
      tab === 'database' && !database ? 'Unlock the database to change its settings'
        : tab === 'backup' && !backup ? 'Choose a database first'
        : null
    if (off(shownTab)) shownTab = 'general'
    // Chosen by click or key on a tab: the redraw keeps the focus on the tabs.
    const open = (tab: Tab) => {
      shownTab = tab
      draw(s, status)
    }
    const groups: Record<Tab, () => HTMLElement[]> = {
      general: () => [
        group('Security',
          row('Lock after inactivity', '1–60 minutes, or never',
            select('Lock after inactivity', s.lockAfterMinutes, LOCK_AFTER, 'min', set('lockAfterMinutes'))),
          row('Lock when Windows locks', 'Also when the session is switched',
            toggle('Lock when Windows locks', s.lockOnSessionLock, set('lockOnSessionLock'))),
          row('Lock when hidden to tray', '', toggle('Lock when hidden to tray', s.lockWhenHidden, set('lockWhenHidden')))),
        group('Clipboard',
          row('Clear clipboard after copying', 'Only if it still holds the copied value',
            select('Clear clipboard after copying', s.clearClipboard, CLEAR_AFTER, 's', set('clearClipboard')))),
      ],
      window: () => [
        group('Window and tray',
          row('Theme', 'Light or dark, or as Windows is set', pick('Theme', s.theme, THEMES, set('theme'))),
          row('Global hotkey', 'Shows or hides the window from any app', hotkeyControl(s.hotkey, set('hotkey'))),
          row('Start with Windows', 'Starts hidden in the tray, locked',
            toggle('Start with Windows', s.startWithWindows, set('startWithWindows'))),
          row('Download site icons', 'Directly from each site, never through a third party',
            toggle('Download site icons', s.downloadIcons, set('downloadIcons')))),
      ],
      database: () => database ? [databaseGroup(database, status.keyFile, (d, next = status) => {
        database = d
        draw(s, next)
      })] : [],
      sync: () => [
        group('Sync',
          row('This database', status.database ?? '', syncControl(status)),
          waiting,
          row('Check for remote changes', 'While unlocked',
            select('Check for remote changes', s.syncEveryMinutes, SYNC_EVERY, 'min', set('syncEveryMinutes')))),
      ],
      backup: () => (backup ? [backupGroup(backup, () => draw(s, status))] : []),
      about: () => [
        group('About',
          row('Version', 'This copy of PswManager', el('span', { className: 'value' }, version))),
      ],
    }
    const panel = el('div', { id: 'settings-panel', role: 'tabpanel' }, ...groups[shownTab]())
    panel.setAttribute('aria-labelledby', `settings-tab-${shownTab}`)
    // A disabled tab's title is not read out or shown from the keyboard: said here.
    const locked = database ? [] : [el('p', { className: 'muted locked-note' }, 'Unlock the database to change its own settings.')]
    container.replaceChildren(
      el('header', {}, el('h1', {}, 'Settings'), button('Done', 'Back (Esc)', onDone)),
      tabBar(off, open),
      ...locked,
      panel,
    )
    // Redrawing replaces the controls: keep the keyboard where it was.
    if (onTab) container.querySelector<HTMLElement>(`#settings-tab-${shownTab}`)?.focus()
    else if (focusedLabel) container.querySelector<HTMLElement>(`[aria-label="${focusedLabel}"]`)?.focus()
  }
}
