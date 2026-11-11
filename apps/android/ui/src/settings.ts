import type { DatabaseSetting, DatabaseSettings, Encryption } from '../../../../src/api'
import { type Choice, describeEncryption, encryptionForm, HEAVY_QUESTION, HISTORY_ITEMS, HISTORY_SIZE, versions, versionsGoing, withValue } from '../../../../src/database-settings'
import { busyButton, button, el, errorLine } from '../../../../src/dom'
import { input, showHide, strengthMeter } from '../../../../src/editor-parts'
import { formatSize } from '../../../../src/entry-text'
import { masterPasswordProblem, masterRulesTip } from '../../../../src/master-password'
import { api, type Cloud, type CloudFile, type Settings, type Status } from './api'
import { svgIcon } from './icons'
import { app, applySettings, confirmSheet, backBar, iconButton, overlay, screen, screens, sheet, show, snack } from './page'
import { keyFileChooser } from './phone-files'
import { slide, type Sliding, swipes } from './slides'
import { CLOUDS, filesIn, STORES } from './sync'

// The settings screen and its tabs, with the screens they open (the key, the
// encryption).

/** The settings' toolbar, coming in from the right with a swipe; once in, the
 *  settings screen takes its place. */
export function settingsPeek(back: () => void): Sliding {
  // The screen itself, drawn now; once in, the same parts become the screen.
  // Before the settings were ever read, only its toolbar's look.
  const parts = app.settings ? settingsParts(app.settings, back) : null
  const panel = el('main', { className: 'peek settings-screen' }, ...(parts?.nodes ?? [el('header', { className: 'bar' }, el('span', { className: 'icon' }, svgIcon('back')), el('h1', {}, 'Settings'))]))
  const shade = el('div', { className: 'shade' }, panel)
  const close = overlay(shade)
  const sliding = slide(shade, panel, 1, close)
  return { ...sliding, release: (open) => sliding.release(open, () => (close(), void settingsScreen(back, parts ?? undefined))) }
}

/** The settings screen taken away to the right by a swipe, as `settingsPeek`
 *  brought it in: the screen it came from is drawn under it at once and the
 *  settings go over it as a panel, so the two move together. Let go short,
 *  the settings are the screen again, scrolled where they were. */
function settingsLeaving(parts: SettingsParts, back: () => void): Sliding {
  const scrolled = window.scrollY
  // Taken off the page and back, the tabs would lose how far they scrolled.
  const tabs = parts.tabs.scrollLeft
  // Back there first (a field being typed in left, so it is saved); the
  // parts are then off the screen, free for the panel.
  parts.goBack()
  const panel = el('main', { className: 'peek settings-screen' }, ...parts.nodes)
  const shade = el('div', { className: 'shade' }, panel)
  const close = overlay(shade)
  panel.scrollTop = scrolled
  parts.tabs.scrollLeft = tabs
  const sliding = slide(shade, panel, 1, close)
  // Not when Back took the panel away meanwhile.
  const stay = () => {
    if (!shade.isConnected) return
    close()
    void settingsScreen(back, parts)
    window.scrollTo(0, scrolled)
    parts.tabs.scrollLeft = tabs
  }
  return { follow: sliding.back, release: (away) => sliding.release(!away, stay) }
}

type Tab = 'general' | 'appearance' | 'database' | 'sync' | 'about'
let settingsTab: Tab = 'general'

/** A choice among values, saved as soon as it changes. */
function choice<T>(label: string, hint: string, name: string, value: T, options: [T, string][]) {
  const select = el('select', { className: 'field' }, ...options.map(([v, text], i) => el('option', { value: String(i), selected: v === value }, text)))
  select.addEventListener('change', () => void save(name, options[Number(select.value)][0]))
  return el('label', { className: 'setting' }, el('span', {}, label, el('small', {}, hint)), select)
}

/** A switch, saved as soon as it changes. */
function toggle(label: string, hint: string, name: string, on: boolean) {
  const box = el('input', { type: 'checkbox', checked: on })
  box.addEventListener('change', () => void save(name, box.checked))
  return el('label', { className: 'setting' }, el('span', {}, label, el('small', {}, hint)), box)
}

async function save(name: string, value: unknown) {
  try {
    applySettings(await api.setSetting(name, value))
    // Turned off: the sealed key goes.
    if (name === 'biometricUnlock' && value === false) await api.forgetBiometric()
  } catch (e) {
    snack(String(e))
  }
}

/** The minutes offered for a setting in minutes (1–60), with 0 called `never`. */
const minutes = (never: string): [number, string][] => [1, 2, 5, 10, 15, 30, 60, 0].map((m) => [m, m === 0 ? never : `${m} min`])

/** Where the settings screen goes back to. */
let settingsBack = () => {}

/** For a database synced with no cloud store: sync it with a file in a store
 *  (the two are merged) or upload it there. */
function syncWithStore(): Node[] {
  const link = async (cloud: Cloud, file: CloudFile | null) => {
    try {
      app.database = (await api.syncWithCloud(cloud, file)).database
      app.syncLine = 'Syncing…'
      snack(file ? `Merging with ${file.name} in ${STORES[cloud].name}…` : `Uploaded to ${STORES[cloud].name}`)
      void settingsScreen(settingsBack)
    } catch (e) {
      snack(String(e))
    }
  }
  const choose = async (cloud: Cloud) => {
    const { name, folder } = STORES[cloud]
    let files: CloudFile[]
    try {
      files = await filesIn(cloud)
    } catch (e) {
      snack(String(e))
      return
    }
    sheet((close) => [
      el('b', {}, `Sync with ${name}`),
      el('p', { className: 'muted' }, `A file in ${folder} is merged with this database: changes on both sides are kept.`),
      ...files.map((file) => button(file.name, `Merge with ${file.name}`, () => (close(), void link(cloud, file)), 'item')),
      button('Upload as a new file', `Put this database into ${name}`, () => (close(), void link(cloud, null)), 'item'),
    ])
  }
  return [el('h2', {}, 'Store'), ...CLOUDS.map((cloud) => button(`Sync with ${STORES[cloud].name}…`, `Sync this database with ${STORES[cloud].name}`, () => void choose(cloud), 'wide'))]
}

/** Stop syncing, for a database synced with a cloud store: the one way out, as
 *  on Windows (#225). */
function stopSyncing(current: NonNullable<Status['database']>): Node[] {
  const store = current.syncedWith ?? 'the store'
  const after = current.copyFolder
    ? `From then on it syncs with its copy in ${current.copyFolder}, like a local file.`
    : 'There is no copy on this phone: the database stays inside the app only.'
  const stop = async () => {
    try {
      app.database = (await api.stopSyncing()).database
      app.syncLine = ''
      snack('Syncing stopped')
      void settingsScreen(settingsBack)
    } catch (e) {
      snack(String(e))
    }
  }
  return [
    el('h2', {}, 'Store'),
    button('Stop syncing…', 'Stop syncing with the store', () =>
      confirmSheet(`Stop syncing with ${store}? The file there stays as it is. ${after}`, 'Stop syncing', () => void stop()), 'wide'),
  ]
}

/** The settings kept in the database file (#193), as on Windows: each change
 *  is saved and synced like an edit. `saved` keeps what was saved; `redraw`
 *  shows the tab again (after a failure, what is in effect). */
function databaseTab(now: DatabaseSettings, saved: (d: DatabaseSettings) => void, redraw: () => void): Node[] {
  const failed = (e: unknown) => (snack(String(e)), redraw())
  // A text is saved as the field is left; the tab stays as it is, so the
  // field tapped next keeps the keyboard.
  const text = (label: string, hint: string, setting: DatabaseSetting, value: string, lines = 1) => {
    const field = lines > 1 ? el('textarea', { className: 'field', rows: lines, value }) : input(value, { className: 'field' })
    field.addEventListener('change', () => void api.setDatabaseSetting(setting, field.value).then(saved, failed))
    return el('label', { className: 'edit-row' }, el('small', {}, `${label} · ${hint}`), field)
  }
  /** New history limits, the other one as the file has it now (another device
   *  may have changed it); when they remove versions, only after saying how many. */
  const limits = async (wanted: (d: DatabaseSettings) => [number, number]) => {
    try {
      const [maxItems, maxSize] = wanted(await api.databaseSettings())
      const going = await api.historyLimitsPreview(maxItems, maxSize)
      const set = () => void api.setHistoryLimits(maxItems, maxSize).then((d) => (saved(d), redraw()), failed)
      if (!going) return set()
      // Shown as saved until the removal is confirmed, however the sheet closes.
      redraw()
      confirmSheet(versionsGoing(going), 'Remove', set)
    } catch (e) {
      failed(e)
    }
  }
  const limit = (label: string, hint: string, value: number, choices: Choice[], shown: (v: number) => string, pick: (v: number) => void) => {
    const options = withValue(choices, value, shown)
    const select = el('select', { className: 'field' }, ...options.map(([v, t], i) => el('option', { value: String(i), selected: v === value }, t)))
    select.addEventListener('change', () => pick(options[Number(select.value)][0]))
    return el('label', { className: 'setting' }, el('span', {}, label, el('small', {}, hint)), select)
  }
  return [
    // First: at the screen's foot its buttons were under Android's navigation bar.
    el('h2', {}, 'Master password and key file'),
    el('p', {}, app.database?.keyFile ? `Key file: ${app.database.keyFile}` : 'No key file'),
    button('Change…', 'Change the master password and / or key file', () => void changeKeyScreen(), 'wide'),
    el('h2', {}, 'Database'),
    text('Name', 'on the unlock screen; the file keeps its name', 'name', now.name),
    text('Description', 'under the name on the unlock screen', 'description', now.description, 2),
    text('Default user name', 'on a new blank entry', 'defaultUsername', now.defaultUsername),
    el('h2', {}, 'History'),
    limit('Versions per entry', 'Older versions each entry keeps', now.historyMaxItems, HISTORY_ITEMS, versions,
      (n) => void limits((d) => [n, d.historyMaxSize])),
    limit('Size per entry', 'The oldest versions go first when larger', now.historyMaxSize, HISTORY_SIZE, formatSize,
      (n) => void limits((d) => [d.historyMaxItems, n])),
    el('h2', {}, 'Encryption'),
    el('p', {}, describeEncryption(now.encryption)),
    button('Change…', 'Change the cipher and key derivation', () => encryptionScreen(now.encryption), 'wide'),
  ]
}

/** A new master password and / or key file, once the current master password
 *  proves right (#193); as on Windows (`spec.md` *Database settings*). */
async function changeKeyScreen() {
  // Which key file the phone uses, as it is now: the chooser's No key file
  // shows only when there is one to give up.
  app.database = (await api.status()).database
  const error = errorLine()
  const back = () => void settingsScreen(settingsBack)
  const current = el('input', { type: 'password', autocomplete: 'current-password', placeholder: 'Current master password', className: 'field' })
  const password = el('input', { type: 'password', autocomplete: 'new-password', placeholder: 'New master password', className: 'field' })
  const again = el('input', { type: 'password', autocomplete: 'new-password', placeholder: 'The new master password again', className: 'field' })
  const strength = strengthMeter(password, api.passwordStrength)
  for (const field of [current, password, again]) error.hideOnInput(field)
  const keyFile = keyFileChooser((e) => error.show(e), true)
  const clear = () => (current.value = password.value = again.value = '')
  const change = busyButton('Change', 'Save the database with the new key', async () => {
    if (!password.value && !keyFile.name()) throw new Error('Give a master password, a key file, or both')
    const weak = masterPasswordProblem(password.value)
    if (weak) throw new Error(weak)
    if (password.value !== again.value) throw new Error('The two passwords differ')
    const keyFileOnly = password.value ? '' : ' The database will then have no master password: only the key file opens it.'
    confirmSheet('Other devices, Keepass2Android too, will need the new key. The old one still opens the store\'s version history. ' +
      `Unlock with fingerprint is set up again at the next unlock with the password.${keyFileOnly}`, 'Change key', () =>
      void api.changeMasterKey(current.value, password.value, keyFile.value()).then((status) => {
        clear()
        app.database = status.database
        back()
        snack('The key is changed')
      }, (e) => error.show(String(e))))
  }, (e) => error.show(e.replace(/^Error: /, '')), 'primary')
  show([
    backBar('Master password', () => (clear(), back())),
    current,
    el('div', { className: 'together' }, password, showHide(password)),
    again,
    strength.element,
    masterRulesTip(password),
    el('p', { className: 'muted' }, 'Leave the new password empty for a key file alone.'),
    el('h2', {}, 'Key file'),
    ...keyFile.nodes,
    error.line,
    change,
  ], () => (clear(), back()))
  current.focus()
}

/** Another cipher and / or key derivation, with a Test that times an unlock on this phone. */
function encryptionScreen(now: Encryption) {
  const error = errorLine()
  const form = encryptionForm(now, api.encryptionUnlockTime, error.show, 'this phone', error.hide)
  // The settings again, read afresh.
  const back = () => void settingsScreen(settingsBack)
  const save = async () => {
    const ms = form.unchanged() ? 0 : await form.measure()
    if (ms === null) return
    const change = () => void api.setEncryption(form.wanted()).then(() => (back(), snack('Encryption changed')), (e) => error.show(String(e)))
    if (form.heavy(ms)) confirmSheet(HEAVY_QUESTION, 'Change', change)
    else change()
  }
  show([
    backBar('Encryption', back),
    el('div', { className: 'encryption' }, ...form.fields),
    error.line,
    busyButton('Change', 'Save the database with this encryption', save, error.show, 'primary'),
  ], back)
}

/** The settings screen's parts, drawn from the settings as last read: the
 *  screen shows them, and so does a swipe bringing it in (whole, not empty). */
interface SettingsParts {
  nodes: Node[]
  body: HTMLElement
  /** The row of tabs, scrolled sideways. */
  tabs: HTMLElement
  fill: () => void
  /** Back where the settings came from, a field being typed in left first (so it is saved). */
  goBack: () => void
  /** A sync brought another device's changes: the database's settings are read again. */
  synced: () => void
}

function settingsParts(s: Settings, back: () => void): SettingsParts {
  const tabs: [Tab, string][] = [['general', 'General'], ['appearance', 'Appearance'], ['database', 'Database'], ['sync', 'Sync'], ['about', 'About']]
  /** The database's settings, read when its tab is shown (null: not yet, or not readable). */
  let kept: DatabaseSettings | null = null
  const body = el('div', { className: 'settings' })
  const databaseNodes = () => {
    if (kept) return databaseTab(kept, (d) => (kept = d), fill)
    void api.databaseSettings().then((d) => ((kept = d), settingsTab === 'database' && fill()), (e) => snack(String(e)))
    return [el('p', { className: 'muted' }, 'Reading the database settings…')]
  }
  const fill = () => {
    bar.querySelectorAll('button').forEach((b, i) => b.classList.toggle('chosen', tabs[i][0] === settingsTab))
    body.replaceChildren(...(settingsTab === 'database' ? databaseNodes() : tab(settingsTab, app.settings ?? s)))
  }
  const bar = el('nav', { className: 'tabs' }, ...tabs.map(([key, label]) => button(label, label, () => ((settingsTab = key), fill()), 'tab')))
  const goBack = () => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur()
    back()
  }
  fill()
  const parts: SettingsParts = {
    nodes: [backBar('Settings', goBack), scrolledTabs(bar), body],
    body,
    tabs: bar,
    fill,
    goBack,
    synced: () => {
      kept = null
      // Shown again unless a field is being typed in.
      if (settingsTab === 'database' && !body.contains(document.activeElement)) fill()
    },
  }
  // A swipe to the right takes the settings away, back where they came from;
  // only once they are the screen, not while a swipe has them on a panel.
  swipes(body, () => app.swipesOn() && screen.contains(body), { right: () => settingsLeaving(parts, back) })
  return parts
}

/** The settings screen; `drawn` when a swipe brought its parts in already. */
export async function settingsScreen(back: () => void, drawn?: SettingsParts) {
  settingsBack = back
  if (!app.settings) applySettings(await api.settings())
  const parts = drawn ?? settingsParts(app.settings!, back)
  show(parts.nodes, parts.goBack, 'settings-screen')
  screens.onSynced((synced) => synced.changed && parts.synced())
  parts.body.parentElement?.querySelector('.tabs .chosen')?.scrollIntoView({ inline: 'nearest', block: 'nearest' })
  // As they are now (another device or the tray may have changed one).
  // Redrawn only when one did, so a choice being made is not disturbed.
  const shown = JSON.stringify(app.settings)
  void api.settings().then((fresh) => {
    applySettings(fresh)
    if (JSON.stringify(fresh) !== shown) parts.fill()
  }, () => {})
}

/** The tabs, wider than the screen, with an arrow on each side where more of
 *  them are hidden; tapping it scrolls that way. */
function scrolledTabs(bar: HTMLElement): HTMLElement {
  const by = (side: number) => () => bar.scrollBy({ left: side * bar.clientWidth * 0.6, behavior: 'smooth' })
  const left = iconButton('back', 'More tabs to the left', by(-1), 'icon tabs-more left')
  const right = iconButton('forward', 'More tabs to the right', by(1), 'icon tabs-more right')
  const arrows = () => {
    left.hidden = bar.scrollLeft <= 1
    right.hidden = bar.scrollLeft + bar.clientWidth >= bar.scrollWidth - 1
  }
  bar.addEventListener('scroll', arrows, { passive: true })
  new ResizeObserver(arrows).observe(bar)
  return el('div', { className: 'tabs-box' }, left, bar, right)
}

/**
 * Whether PswManager fills in passwords in other apps and in browsers (#236),
 * and the button to Android's choice of autofill service: the phone keeps
 * one, chosen there, and PswManager can only offer itself.
 */
function autofillRow(): HTMLElement {
  const said = el('p', { className: 'muted' }, 'Checking…')
  const choose = button('Choose the autofill service', 'Android’s list of autofill services', () =>
    void api.chooseAutofill().catch((e) => snack(String(e))), 'wide')
  void api.autofillStatus().then(({ supported, enabled }) => {
    choose.hidden = !supported
    said.textContent = !supported
      ? 'This phone does not autofill.'
      : enabled
        ? 'PswManager fills in passwords in apps and browsers. Chrome does so only with “Autofill using another service” on in its settings.'
        : 'PswManager does not fill in passwords yet: choose it as the phone’s autofill service.'
  }, (e) => (said.textContent = String(e)))
  return el('div', {}, said, choose)
}

/** A tab of the phone's own settings (the Database tab is [databaseTab]). */
function tab(which: Exclude<Tab, 'database'>, s: Settings): Node[] {
  switch (which) {
    case 'general':
      return [
        el('h2', {}, 'Locking'),
        choice('In the background', 'Locks this long after the app goes away', 'lockInBackground', s.lockInBackground, [[0, 'At once'], [30, '30 s'], [60, '1 min'], [300, '5 min'], [null, 'Never']]),
        toggle('When the screen turns off', 'Locks at once', 'lockOnScreenOff', s.lockOnScreenOff),
        choice('Without a touch', 'While the app is in front', 'lockAfterMinutes', s.lockAfterMinutes, minutes('Never')),
        el('h2', {}, 'Unlock'),
        toggle('Unlock with fingerprint', 'The fingerprint button on the unlock screen; set up at the next unlock with the password. Off deletes the stored key', 'biometricUnlock', s.biometricUnlock),
        choice('Master password', 'Asked for again after', 'passwordEveryDays', s.passwordEveryDays, [1, 3, 7, 14, 30, 60, 90].map((d) => [d, d === 1 ? '1 day' : `${d} days`] as [number, string])),
        el('h2', {}, 'Autofill'),
        autofillRow(),
        el('h2', {}, 'Screen'),
        toggle('Allow screenshots', 'Screenshots and screen recording show PswManager, and so does the recent-apps preview', 'allowScreenshots', s.allowScreenshots),
        el('h2', {}, 'Clipboard'),
        choice('Clear after copying', 'Only if it still holds the copied value', 'clearClipboard', s.clearClipboard, [5, 10, 20, 30, 60, 120].map((n) => [n, `${n} s`] as [number, string])),
      ]
    case 'appearance':
      return [
        choice('Theme', 'Light or dark, or as the phone is set', 'theme', s.theme, [['system', 'As the phone'], ['light', 'Light'], ['dark', 'Dark']]),
        toggle('Download site icons', 'From each site itself, never through a third party', 'downloadIcons', s.downloadIcons),
        toggle('Swipes on the list', 'To the right: groups and tags; to the left: settings', 'swipes', s.swipes),
      ]
    case 'sync':
      return [
        el('p', {}, app.database?.syncedWith ? `Syncs with ${app.database.syncedWith}` : 'Not synced'),
        ...(app.database?.cloud ? [el('p', { className: 'muted' }, app.database.copyFolder ? `Copy on this phone: ${app.database.copyFolder}` : 'No copy on this phone yet')] : []),
        choice('Check for changes', 'While the app is in front and unlocked', 'syncEveryMinutes', s.syncEveryMinutes, minutes('Off')),
        ...(app.linked() ? [button('Sync now', 'Sync now', () => void api.syncNow().then(() => snack('Syncing…')), 'primary')] : []),
        ...(app.database ? (app.database.cloud ? stopSyncing(app.database) : syncWithStore()) : []),
        el('p', { className: 'muted' }, 'To use another database, lock this one and choose “Use another database…”: its file stays where it is.'),
      ]
    case 'about':
      return [
        el('p', {}, `PswManager for Android, version ${s.version}`),
        el('p', { className: 'muted' }, 'The database is a KeePass file (KDBX 4.1).'),
        button('GitHub repository', 'Open the GitHub repository in the browser', () => void api.openRepository().catch((e) => snack(String(e))), 'link'),
      ]
  }
}
