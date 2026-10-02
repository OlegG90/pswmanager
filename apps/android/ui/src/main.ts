import { listen } from '@tauri-apps/api/event'
import { el, button, busyButton, errorLine, enterPresses } from '../../../../src/dom'
import { EMPTY_ENTRY, chip, fieldRow, generatorPanel, input, readField, showHide, strengthMeter, withStar } from '../../../../src/editor-parts'
import { dateOf, formatDateTime, formatSize, formatTags, keep, parseTags, singleLine, splitCode, startOfDay, textareaLines, titleOf } from '../../../../src/entry-text'
import { DEFAULT_ICON, glyphIcon } from '../../../../src/glyphs'
import { siteIconCache } from '../../../../src/site-icons'
import { ALL, FAVORITE, GROUPS, sameFilter, search, tagCounts, type Filter } from '../../../../src/search'
import { tagInput } from '../../../../src/tag-input'
import { svgIcon, type IconName } from './icons'
import { api, type Settings, type CloudFile, type Entry, type EntryData, type Listing, type Picked, type SignedIn, type Status, type Synced } from './api'

const screen = document.querySelector<HTMLElement>('#screen')!
const snackbar = document.querySelector<HTMLElement>('#snackbar')!

/** The groups the phone shows; Templates and Trash come later. */
const PHONE_GROUPS = GROUPS.filter((g) => g.group !== 'templates' && g.group !== 'trash')

/** The database on this phone: what the screens say about it. */
let database: Status['database'] = null
let filter: Filter = ALL
let query = ''
/** What the last sync did, as the status line says it. */
let syncLine = ''
/** The last sync asked to sign in to the store again. */
let signInAgain = false
/** Why the visible copy was not written at the last sync, if it was not. */
let copyProblem: string | null = null
/** When the last sync ended (ms), for syncing again on coming back. */
let lastSynced = 0
/** The database is unlocked: syncs run on their own. */
let unlocked = false
/** Stops what the screen shown runs on a timer (the TOTP countdown). */
let leave = () => {}
/** The screen shown's answer to a sync (the list's status line). */
let onSynced = (_synced: Synced) => {}

/** What Back closes or returns to, innermost last (see SystemPlugin.kt). */
let backs: (() => void)[] = []

/** Called by Android's Back: true when the page did something with it. */
;(window as unknown as { pswmBack: () => boolean }).pswmBack = () => {
  const back = backs.pop()
  back?.()
  return !!back
}

/** Shows a screen; `back` is where Back returns from it (none: Back leaves the app). */
function show(children: Node[], back?: () => void, className = '') {
  leave()
  leave = () => {}
  onSynced = () => {}
  document.querySelectorAll('.shade, .sheet').forEach((n) => n.remove())
  backs = back ? [back] : []
  screen.className = className
  screen.replaceChildren(...children)
  window.scrollTo(0, 0)
}

/** Puts an overlay (a panel, the drawer) on top: Back closes it. Returns its close. */
function overlay(node: HTMLElement): () => void {
  const close = () => {
    node.remove()
    backs = backs.filter((b) => b !== close)
  }
  backs.push(close)
  node.addEventListener('click', (e) => e.target === node && close())
  document.body.append(node)
  return close
}

let snackTimer = 0
function snack(text: string) {
  snackbar.textContent = text
  snackbar.hidden = false
  clearTimeout(snackTimer)
  snackTimer = window.setTimeout(() => (snackbar.hidden = true), 3000)
}

/** A panel from the bottom; `fill` gets what closes it. */
function sheet(fill: (close: () => void) => Node[]) {
  const shade = el('div', { className: 'sheet' })
  const close = overlay(shade)
  shade.append(el('div', { className: 'panel' }, ...fill(close)))
}

/** Asks before `action`, which runs on `yes`. */
function confirmSheet(question: string, yes: string, action: () => void) {
  sheet((close) => [el('p', {}, question), button(yes, yes, () => (close(), action()), 'primary'), button('Cancel', 'Cancel', close, 'link')])
}

async function copied(copy: Promise<number>) {
  try {
    snack(`Copied · clears in ${await copy} s`)
  } catch (e) {
    snack(String(e))
  }
}

/** The settings in effect (see applySettings). */
let settings: Settings | null = null

/** Takes the settings: the theme now, the timings where they are used. */
function applySettings(fresh: Settings) {
  settings = fresh
  if (fresh.theme === 'system') delete document.documentElement.dataset.theme
  else document.documentElement.dataset.theme = fresh.theme
}

async function start() {
  applySettings(await api.settings())
  const status = await api.status()
  database = status.database
  unlocked = status.unlocked
  if (!database) chooseScreen()
  else if (status.unlocked) listScreen(await api.listing())
  else unlockScreen()
}

// ------------------------------------------------------------ first run

function chooseScreen() {
  const error = errorLine()
  const local = busyButton('Open a local file', 'A .kdbx on this phone or an SD card', async () => {
    const status = await api.openLocalFile()
    if (!status) return
    database = status.database
    unlockScreen()
  }, error.show, 'card')
  show([
    el('p', { className: 'muted' }, 'First run'),
    el('h1', {}, 'Choose your database'),
    el('p', {}, 'PswManager opens one KeePass (KDBX 4) file. The same file opens in PswManager on Windows and in KeePassXC.'),
    el('button', { type: 'button', className: 'card', onclick: () => void dropboxScreen() }, 'Sync with Dropbox', el('small', {}, 'Sign in once. The file lives in Apps/PswManager Sync; a working copy stays on this phone.')),
    local,
    el('p', { className: 'muted' }, 'A local file is picked with Android’s file picker; it can be anywhere the picker reaches, also a folder another app syncs.'),
    error.line,
  ])
}

/** What to do when a sign-in ends (event `signed-in`). */
let onSignedIn = (_signedIn: SignedIn) => {}
void listen<SignedIn>('signed-in', (e) => onSignedIn(e.payload))

/** Signs in to Dropbox in the browser, then waits for it to come back. A
 *  sign-in started again gives up on the one before. */
function signIn(): Promise<void> {
  onSignedIn({ error: 'A new sign-in was started' })
  return new Promise((done, fail) => {
    onSignedIn = (signedIn) => {
      onSignedIn = () => {}
      if (signedIn.error) fail(signedIn.error)
      else done()
    }
    api.signInToDropbox().catch(fail)
  })
}

/** First run with Dropbox: sign in, then pick the file in the app's folder. */
async function dropboxScreen() {
  const error = errorLine()
  const back = iconButton('back', 'Back', chooseScreen)
  const waiting = el('p', {}, 'Waiting for Dropbox…')
  show([el('header', { className: 'bar' }, back, el('h1', {}, 'Dropbox')), el('p', { className: 'muted' }, 'Sign-in opens in the browser. Nothing is uploaded.'), waiting, error.line], chooseScreen)
  try {
    await signIn()
    filesScreen(await api.dropboxFiles())
  } catch (e) {
    waiting.remove()
    error.show(String(e))
    screen.append(busyButton('Sign in to Dropbox', 'Sign in again', dropboxScreen, error.show, 'primary'))
  }
}

function filesScreen(files: CloudFile[]) {
  const error = errorLine()
  const pick = (file: CloudFile) => button(file.name, `Use ${file.name}`, () => folderScreen(files, file), 'card')
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Back', chooseScreen), el('h1', {}, 'Pick the file')),
    el('p', { className: 'muted' }, 'Apps / PswManager Sync'),
    ...(files.length ? files.map(pick) : [el('p', {}, 'There is no .kdbx in the app’s folder yet. Put one there from PswManager on Windows (Settings → Sync → Upload) and come back.')]),
    el('p', { className: 'muted' }, 'PswManager sees only its app folder in Dropbox. Keepass2Android and PswManager for Windows open the same file there.'),
    error.line,
  ], chooseScreen)
}

/** First run with Dropbox, step 3: where the visible copy goes on this phone. */
function folderScreen(files: CloudFile[], file: CloudFile) {
  const error = errorLine()
  let folder: Picked | null = null
  const chosen = el('p', {}, 'No folder chosen yet.')
  const taken = el('p', { className: 'muted' })
  const go = busyButton('Download and continue', 'Download the database', async () => {
    const opened = await api.openDropboxFile(file, folder!)
    database = opened.status.database
    unlockScreen()
    if (opened.copyProblem) snack(`${opened.copyProblem}. Choose the folder again in the sync sheet.`)
  }, error.show, 'primary')
  go.disabled = true
  const choose = busyButton('Choose a folder…', 'Android’s folder picker', async () => {
    const picked = await api.pickFolder()
    if (!picked) return
    folder = picked
    chosen.textContent = `Folder: ${picked.name}`
    taken.textContent = (await api.copyNameTaken(picked.uri, file.name)) ? `A ${file.name} is already in this folder. It will be kept as ${file.name}.bak.` : ''
    go.disabled = false
  }, error.show)
  const back = () => filesScreen(files)
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Back', back), el('h1', {}, 'Where to keep it')),
    el('p', {}, `${file.name} syncs with Dropbox. A copy of it is kept in a folder on this phone, where you can see it and back it up; it is updated after every change.`),
    choose,
    chosen,
    taken,
    go,
    error.line,
  ], back)
}

// ------------------------------------------------------------ unlock

function unlockScreen() {
  const current = database!
  const error = errorLine()
  const password = el('input', { type: 'password', autocomplete: 'off', placeholder: 'Master password', className: 'field' })
  const unlock = busyButton('Unlock', 'Unlock the database', async () => {
    syncLine = 'Syncing…'
    const opened = await api.unlock(password.value)
    unlocked = true
    password.value = ''
    listScreen(opened)
    // The sync may have finished before the list was there to hear it.
    const last = await api.lastSync()
    if (last) applySync(last)
  }, error.show, 'primary')
  enterPresses(unlock, password)
  error.hideOnInput(password)
  const forget = () =>
    confirmSheet(`Forget “${current.title}”? Its file stays where it is.`, 'Forget it', async () => {
      try {
        database = (await api.forgetDatabase()).database
        chooseScreen()
      } catch (e) {
        error.show(String(e))
      }
    })
  const showAgain = (status: Status) => {
    database = status.database
    unlockScreen()
  }
  const keyLine = current.keyFile
    ? el('p', { className: 'muted' }, `Key file: ${current.keyFile} `, button('Remove', 'Unlock without a key file', () => void api.clearKeyFile().then(showAgain, error.show), 'link'))
    : button('Use a key file…', 'For a database set up with one: a .keyx or .key file kept apart from it', () => void api.pickKeyFile().then(showAgain, error.show), 'link')
  show([
    el('h1', {}, current.title),
    el('p', { className: 'muted' }, current.description),
    el('p', { className: 'muted' }, current.syncedWith ? `Syncs with ${current.syncedWith}` : ''),
    password,
    keyLine,
    unlock,
    error.line,
    button('Use another database…', 'Forget this one (its file stays where it is)', forget, 'link'),
  ])
  password.focus()
}

// ------------------------------------------------------------ the list

/** A toolbar button showing an icon. */
function iconButton(name: IconName, title: string, onClick: () => void, className = 'icon') {
  const b = button('', title, onClick, className)
  b.setAttribute('aria-label', title)
  b.append(svgIcon(name))
  return b
}

/** The list, as Keepass2Android lays it out: a toolbar on top, the entries,
 *  the search button floating at the bottom right, the sync status under it. */
function listScreen(opened: Listing) {
  let listing = opened
  const title = el('h1', {})
  const searchField = el('input', { type: 'search', className: 'field search', value: query })
  const status = el('footer', { className: 'status' }, syncLine || 'Not synced yet')
  const list = el('ul', { className: 'entries' })
  const fill = () => {
    title.textContent = filterLabel(filter)
    searchField.placeholder = `Search ${filterLabel(filter)}`
    const shown = search(listing.entries, query, filter)
    list.replaceChildren(...shown.map((entry) => row(entry, listing)))
    if (shown.length === 0) list.append(el('li', { className: 'muted empty' }, 'Nothing here.'))
  }
  searchField.addEventListener('input', () => {
    query = searchField.value
    fill()
  })
  status.addEventListener('click', syncSheet)
  const scroll = el('div', { className: 'scroll' }, list)
  pullToSync(scroll, () => (status.textContent = 'Syncing…'))
  const toolbar = el('header', { className: 'bar' },
    iconButton('menu', 'Groups and tags', () => drawer(listing, fill)),
    title,
    iconButton('sync', 'Sync now', () => {
      status.textContent = 'Syncing…'
      void api.syncNow()
    }),
    iconButton('settings', 'Settings', () => void settingsScreen(() => listScreen(listing))),
    iconButton('lock', 'Lock', () => void lock()))
  const closeSearch = () => {
    searching.replaceWith(toolbar)
    findButton.hidden = false
    backs = backs.filter((b) => b !== closeSearch)
    query = ''
    searchField.value = ''
    fill()
  }
  const searching = el('header', { className: 'bar' }, iconButton('back', 'Close the search', closeSearch), searchField)
  const findButton = iconButton('search', 'Search', () => {
    toolbar.replaceWith(searching)
    findButton.hidden = true
    backs.push(closeSearch)
    searchField.focus()
  }, 'fab')
  const add = iconButton('plus', 'New entry', () => void editorScreen(null, listing, () => listScreen(listing)), 'fab')
  show([query ? searching : toolbar, scroll, el('div', { className: 'fabs' }, findButton, add), status], undefined, 'list-screen')
  if (query) {
    findButton.hidden = true
    backs.push(closeSearch)
  }
  fill()
  onSynced = (synced) => {
    status.textContent = synced.text
    status.classList.toggle('problem', synced.problem)
    if (synced.changed) void api.listing().then((fresh) => ((listing = fresh), fill()))
  }
}

/** Picks the folder for the visible copy and writes it there. */
async function chooseCopyFolder() {
  try {
    const folder = await api.pickFolder()
    if (!folder) return
    database = (await api.setCopyFolder(folder)).database
    snack(`The copy is kept in ${folder.name}`)
  } catch (e) {
    snack(String(e))
  }
}

/** Pulling the list down from its top syncs. */
function pullToSync(scroll: HTMLElement, started: () => void) {
  let from: number | null = null
  scroll.addEventListener('touchstart', (e) => (from = scroll.scrollTop === 0 ? e.touches[0].clientY : null), { passive: true })
  scroll.addEventListener('touchend', (e) => {
    if (from !== null && e.changedTouches[0].clientY - from > 80) {
      started()
      void api.syncNow()
    }
    from = null
  })
}

function applySync(synced: Synced) {
  syncLine = synced.text
  signInAgain = synced.signIn
  copyProblem = synced.copyProblem
  lastSynced = Date.now()
  onSynced(synced)
}

function filterLabel(f: Filter) {
  return f.kind === 'tag' ? f.tag : PHONE_GROUPS.find((g) => g.group === f.group)!.label
}

function row(entry: Entry, listing: Listing): HTMLLIElement {
  const item = el('li', { tabIndex: 0 }, icon(entry, listing), el('span', {}, el('b', {}, titleOf(entry)), el('small', {}, entry.username)))
  item.addEventListener('click', () => void entryScreen(entry.id, listing))
  item.addEventListener('contextmenu', (e) => {
    e.preventDefault()
    quickCopy(entry)
  })
  return item
}

/** Site icons by host; one that arrives replaces the key in the images waiting for it. */
const siteIcons = siteIconCache(api.icon, (host, src) => {
  document.querySelectorAll<HTMLImageElement>('img.icon[data-host]').forEach((img) => img.dataset.host === host && (img.src = src))
})
void listen<string>('icon-ready', (e) => siteIcons.refresh(e.payload))

/** An entry's own image, else the drawing chosen for it, else its site's icon
 *  (when it arrives), else the key: as on Windows. */
function icon(entry: Entry, listing: Listing) {
  const custom = entry.customIcon && listing.customIcons[entry.customIcon]
  if (custom) return el('img', { className: 'icon', src: custom, alt: '' })
  if (entry.icon !== null) return el('img', { className: 'icon', src: glyphIcon(entry.icon), alt: '' })
  const img = el('img', { className: 'icon', src: (entry.host && siteIcons.get(entry.host)) || DEFAULT_ICON, alt: '' })
  if (entry.host) img.dataset.host = entry.host
  return img
}

/** A long press: copy without opening the entry. */
function quickCopy(entry: Entry) {
  const choices: [string, () => Promise<number>][] = [
    ['Copy user name', () => api.copyField(entry.id, 'UserName')],
    ['Copy password', () => api.copyField(entry.id, 'Password')],
  ]
  if (entry.otp) choices.push(['Copy TOTP code', () => api.copyTotp(entry.id)])
  sheet((close) => [el('b', {}, titleOf(entry)), ...choices.map(([label, copy]) => button(label, label, () => void copied(copy()).then(close), 'item'))])
}

/** The status line's sheet: what the last sync did, and Sync now. */
function syncSheet() {
  sheet((close) => [
    el('b', {}, 'Sync'),
    el('p', {}, syncLine || 'Not synced yet'),
    el('p', { className: 'muted' }, database?.syncedWith ? `With ${database.syncedWith}` : ''),
    ...(copyProblem ? [el('p', { className: 'error' }, copyProblem)] : []),
    ...(database?.cloud
      ? [
          el('p', { className: 'muted' }, database.copyFolder ? `Copy on this phone: ${database.copyFolder}` : 'No copy on this phone yet'),
          button(database.copyFolder ? 'Move the copy…' : 'Keep a copy on this phone…', 'Choose the folder', () => {
            close()
            void chooseCopyFolder()
          }, 'link'),
        ]
      : []),
    ...(signInAgain ? [button('Sign in', 'Sign in again', () => (close(), void signIn().then(() => api.syncNow(), (e) => snack(String(e)))), 'primary')] : []),
    button('Sync now', 'Sync now', () => {
      close()
      void api.syncNow()
    }, 'primary'),
  ])
}

function drawer(listing: Listing, changed: () => void) {
  const panel = el('nav', { className: 'drawer' })
  const close = overlay(el('div', { className: 'shade' }, panel))
  const item = (f: Filter, label: string, count: number) => {
    const choose = () => {
      filter = f
      close()
      changed()
    }
    return el('button', { type: 'button', className: sameFilter(f, filter) ? 'chosen' : '', onclick: choose }, el('span', {}, label), el('small', {}, String(count)))
  }
  const entries = listing.entries
  panel.append(
    el('b', { className: 'title' }, database?.title ?? ''),
    el('small', {}, [database?.syncedWith, syncLine].filter(Boolean).join(' · ')),
    el('h2', {}, 'Groups'),
    ...PHONE_GROUPS.map(({ group, label }) => {
      const f: Filter = { kind: 'group', group }
      return item(f, label, search(entries, '', f).length)
    }),
    el('h2', {}, 'Tags'),
    ...tagCounts(entries).map(([tag, count]) => item({ kind: 'tag', tag }, tag, count)),
    button('Settings', 'Settings', () => (close(), void settingsScreen(() => listScreen(listing))), 'action'),
    button('Lock', 'Lock the database', () => void lock(), 'action'),
  )
}

/** Shows the unlock screen after the database locked by itself (in the background). */
void listen('locked', () => {
  unlocked = false
  query = ''
  unlockScreen()
})

async function lock() {
  await api.lock()
  afterLock()
}

/** The database is locked: nothing of it stays on the page. */
function afterLock() {
  unlocked = false
  query = ''
  unlockScreen()
}

// ------------------------------------------------------------ settings

type Tab = 'general' | 'appearance' | 'sync' | 'about'
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
  } catch (e) {
    snack(String(e))
  }
}

/** The minutes offered for a setting in minutes (1–60), with 0 called `never`. */
const minutes = (never: string): [number, string][] => [1, 2, 5, 10, 15, 30, 60, 0].map((m) => [m, m === 0 ? never : `${m} min`])

async function settingsScreen(back: () => void) {
  applySettings(await api.settings())
  const tabs: [Tab, string][] = [['general', 'General'], ['appearance', 'Appearance'], ['sync', 'Sync'], ['about', 'About']]
  const body = el('div', { className: 'settings' })
  const fill = () => {
    bar.querySelectorAll('button').forEach((b, i) => b.classList.toggle('chosen', tabs[i][0] === settingsTab))
    body.replaceChildren(...tab(settingsTab, settings!))
  }
  const bar = el('nav', { className: 'tabs' }, ...tabs.map(([key, label]) => button(label, label, () => ((settingsTab = key), fill()), 'tab')))
  show([el('header', { className: 'bar' }, iconButton('back', 'Back', back), el('h1', {}, 'Settings')), bar, body], back)
  fill()
}

function tab(which: Tab, s: Settings): Node[] {
  switch (which) {
    case 'general':
      return [
        el('h2', {}, 'Locking'),
        choice('In the background', 'Locks this long after the app goes away', 'lockInBackground', s.lockInBackground, [[0, 'At once'], [30, '30 s'], [60, '1 min'], [300, '5 min'], [null, 'Never']]),
        toggle('When the screen turns off', 'Locks at once', 'lockOnScreenOff', s.lockOnScreenOff),
        choice('Without a touch', 'While the app is in front', 'lockAfterMinutes', s.lockAfterMinutes, minutes('Never')),
        el('h2', {}, 'Clipboard'),
        choice('Clear after copying', 'Only if it still holds the copied value', 'clearClipboard', s.clearClipboard, [5, 10, 20, 30, 60, 120].map((n) => [n, `${n} s`] as [number, string])),
      ]
    case 'appearance':
      return [
        choice('Theme', 'Light or dark, or as the phone is set', 'theme', s.theme, [['system', 'As the phone'], ['light', 'Light'], ['dark', 'Dark']]),
        toggle('Download site icons', 'From each site itself, never through a third party', 'downloadIcons', s.downloadIcons),
      ]
    case 'sync':
      return [
        el('p', {}, database?.syncedWith ? `Syncs with ${database.syncedWith}` : 'Not synced'),
        ...(database?.cloud ? [el('p', { className: 'muted' }, database.copyFolder ? `Copy on this phone: ${database.copyFolder}` : 'No copy on this phone yet')] : []),
        choice('Check for changes', 'While the app is in front and unlocked', 'syncEveryMinutes', s.syncEveryMinutes, minutes('Off')),
        button('Sync now', 'Sync now', () => void api.syncNow().then(() => snack('Syncing…')), 'primary'),
        el('p', { className: 'muted' }, 'To use another database or stop syncing this one, lock it and choose “Use another database…”: its file stays where it is.'),
      ]
    case 'about':
      return [
        el('p', {}, `PswManager for Android, version ${s.version}`),
        el('p', { className: 'muted' }, 'The database is a KeePass file (KDBX 4.1): it opens in PswManager on Windows, KeePassXC and Keepass2Android.'),
      ]
  }
}

// ------------------------------------------------------------ an entry

/** A command in a line's menu. */
type Action = [label: string, run: () => void]

/** A labelled value with ⋮ at the end for its commands; tapping the value
 *  runs the first of them (copies it, or opens a file). */
function line(label: string, value: Node | string, copy: (() => Promise<number>) | null, ...more: Action[]) {
  const shown = el('span', {}, el('small', {}, label), typeof value === 'string' ? el('span', {}, value) : value)
  const actions: Action[] = [...(copy ? [['Copy', () => void copied(copy())] as Action] : []), ...more]
  if (actions.length) shown.addEventListener('click', actions[0][1])
  const menu = button('⋮', `${label}: more`, () =>
    sheet((close) => [el('b', {}, label), ...actions.map(([name, run]) => button(name, name, () => (close(), run()), 'item'))]), 'icon more')
  return el('div', { className: 'line' }, shown, ...(actions.length ? [menu] : []))
}

/** A secret's line: masked, with Show / Hide in its menu. */
function secretLine(id: string, label: string, field: string) {
  const mask = '••••••••••••'
  const value = el('span', { className: 'masked' }, mask)
  let shown = false
  const toggle = async () => {
    shown = !shown
    value.textContent = shown ? await api.reveal(id, field) : mask
    value.classList.toggle('masked', !shown)
  }
  return line(label, value, () => api.copyField(id, field), ['Show / hide', () => void toggle()])
}

async function entryScreen(id: string, listing: Listing) {
  const entry = await api.entry(id)
  const rows: Node[] = []
  const stops: (() => void)[] = []
  if (entry.username) rows.push(line('User name', entry.username, () => api.copyField(id, 'UserName')))
  if (entry.hasPassword) rows.push(secretLine(id, 'Password', 'Password'))
  if (entry.otp) {
    const [node, stop] = totpLine(id)
    rows.push(node)
    stops.push(stop)
  }
  if (entry.url) {
    const open: Action = ['Open in the browser', () => void api.openUrl(id).catch((e) => snack(String(e)))]
    rows.push(line('URL', entry.url, () => api.copyField(id, 'URL'), open))
  }
  for (const field of entry.fields) {
    rows.push(field.protected ? secretLine(id, field.name, field.name) : line(field.name, field.value ?? '', () => api.copyField(id, field.name)))
  }
  if (entry.notes) rows.push(el('div', { className: 'line notes' }, el('span', {}, el('small', {}, 'Notes'), el('span', {}, entry.notes))))
  if (entry.attachments.length) {
    const open = (name: string) => void api.openAttachment(id, name).catch((e) => snack(String(e)))
    rows.push(el('h2', {}, 'Attachments'), ...entry.attachments.map((a) => line(a.name, formatSize(a.size), null, ['Open', () => open(a.name)])))
  }
  if (entry.modified) rows.push(el('p', { className: 'muted' }, `Changed ${formatDateTime(entry.modified)}`))
  const toList = () => listScreen(listing)
  const back = iconButton('back', 'Back to the list', toList)
  const starred = entry.tags.includes(FAVORITE)
  const star = iconButton('star', starred ? 'Not favorite' : 'Favorite', () =>
    void api.setFavorite(id, !starred).then((fresh) => entryScreen(id, fresh), (e) => snack(String(e))), starred ? 'icon starred' : 'icon')
  const edit = iconButton('pencil', 'Edit', () => void editorScreen(id, listing, () => void entryScreen(id, listing)))
  const remove = () =>
    confirmSheet(`Move “${titleOf(entry)}” to the recycle bin?`, 'Delete', () =>
      void api.deleteEntry(id).then((fresh) => (listScreen(fresh), snack('Moved to the recycle bin')), (e) => snack(String(e))))
  const more = iconButton('more', 'More', () => sheet((close) => [el('b', {}, titleOf(entry)), button('Delete', 'Move to the recycle bin', () => (close(), remove()), 'item')]))
  show([el('header', { className: 'bar' }, back, icon(entry, listing), el('h1', {}, titleOf(entry)), star, edit, more), ...rows], toList)
  leave = () => stops.forEach((stop) => stop())
}

// ------------------------------------------------------------ editing

/** The editor, as `spec.md` *Editing* has it; a new blank entry when `id` is
 *  null. Cancel and Back return with `back`; saving shows the entry. */
async function editorScreen(id: string | null, listing: Listing, back: () => void) {
  let data: EntryData
  try {
    // A new entry goes to the top group, with the database's default user name.
    data = id ? await api.editEntry(id) : { ...EMPTY_ENTRY, username: listing.database.defaultUsername }
  } catch (e) {
    snack(String(e))
    return
  }
  const error = el('p', { className: 'error', hidden: true })
  const showError = (message: string) => {
    error.textContent = message
    error.hidden = false
    error.scrollIntoView({ block: 'nearest' })
  }
  const title = input(data.title, { className: 'field' })
  const username = input(data.username, { className: 'field', autocapitalize: 'off' })
  const password = input(data.password, { type: 'password', className: 'field secret' })
  const url = input(data.url, { type: 'url', className: 'field', placeholder: 'https://' })
  const otp = input(data.otp, { type: 'password', className: 'field secret', placeholder: 'Secret or otpauth:// URI' })
  const tags = tagInput(data.tags.filter((t) => t !== FAVORITE), tagCounts(listing.entries).map(([tag]) => tag))
  let starred = data.tags.includes(FAVORITE)
  const star = chip('★ Favorite', 'Listed under Favorites', starred, (on) => (starred = on))
  // A date input holds a day: an expiry time on that day stays as it was.
  const expiryDay = data.expires ? dateOf(data.expires) : ''
  const expires = el('input', { type: 'date', value: expiryDay, className: 'field' })
  const notes = el('textarea', { value: data.notes, rows: 4, spellcheck: false, className: 'field' })
  const fieldList = el('div', { className: 'fields' }, ...data.fields.map((f) => fieldRow(f)))
  const strength = strengthMeter(password, api.passwordStrength)
  const generator = generatorPanel(api.generatePassword, (chosen) => {
    password.value = chosen
    strength.refresh()
  }, showError)

  const trimmedLine = (text: string) => singleLine(text).trim()
  // Values the form only reformats are kept as they were, as on Windows.
  const collect = (): EntryData => ({
    ...data,
    title: keep(data.title, title.value, singleLine),
    username: keep(data.username, username.value, singleLine),
    password: keep(data.password, password.value, singleLine),
    url: keep(data.url, url.value.trim(), trimmedLine),
    notes: keep(data.notes, notes.value, textareaLines),
    otp: keep(data.otp, otp.value.trim(), trimmedLine),
    // Typing the tag Favorite stars the entry.
    tags: keep(data.tags, withStar(tags.value(), starred || tags.value().includes(FAVORITE), data.tags), (t) => parseTags(formatTags(t))),
    fields: [...fieldList.querySelectorAll<HTMLDivElement>('.field-row')].map(readField),
    expires: expires.value === expiryDay ? data.expires : expires.value ? startOfDay(expires.value) : null,
  })

  let saving = false
  const save = async () => {
    if (saving) return
    saving = true
    try {
      const saved = await api.saveEntry(id, id ? data : null, collect())
      await entryScreen(saved.id, saved.listing)
      if (saved.conflicts.length) snack(`Also changed on another device: ${saved.conflicts.join(', ')}. That version is in the history.`)
    } catch (e) {
      showError(String(e))
    } finally {
      saving = false
    }
  }
  const close = () => {
    if (JSON.stringify(collect()) === untouched) back()
    else confirmSheet('Discard the changes?', 'Discard', back)
  }
  // Android's Back cancels too, and stays on the editor when the changes are kept.
  const onBack = () => {
    backs.push(onBack)
    close()
  }
  const label = (text: string, ...controls: Node[]) => el('label', { className: 'edit-row' }, el('small', {}, text), ...controls)
  const together = (...controls: Node[]) => el('div', { className: 'together' }, ...controls)
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Cancel', close), el('h1', {}, id ? 'Edit entry' : 'New entry'), iconButton('check', 'Save', () => void save())),
    el('form', { className: 'editor', onsubmit: (e: SubmitEvent) => (e.preventDefault(), void save()) },
      error,
      label('Title', title),
      label('User name', username),
      label('Password', together(password, showHide(password), generator.open)),
      strength.element,
      generator.panel,
      label('URL', url),
      label('TOTP secret', together(otp, showHide(otp))),
      label('Tags', tags.element),
      star,
      label('Expires', together(expires, button('Never', 'Does not expire', () => (expires.value = ''), 'link'))),
      label('Notes', notes),
      el('h2', {}, 'Additional fields'),
      fieldList,
      button('+ Add field', 'Add a field', () => fieldList.append(fieldRow()), 'link'),
      ...(id ? [el('p', { className: 'muted' }, 'Saving keeps the previous version in the history.')] : [])),
  ], onBack)
  const untouched = JSON.stringify(collect())
  if (!id) title.focus()
}

/** The TOTP code with its countdown, and what stops the countdown. A new
 *  code is asked for only when the period runs out. */
function totpLine(id: string): [HTMLElement, () => void] {
  const code = el('span', { className: 'code' })
  const left = el('small', {})
  let remaining = 0
  let fetching = false
  const tick = async () => {
    if (fetching) return
    if (remaining <= 0) {
      fetching = true
      try {
        const now = await api.totp(id)
        if (!now) return
        code.textContent = splitCode(now.code)
        remaining = now.remaining
      } finally {
        fetching = false
      }
    } else remaining--
    left.textContent = ` ${remaining} s`
  }
  void tick()
  const timer = window.setInterval(() => void tick(), 1000)
  return [line('TOTP', el('span', {}, code, left), () => api.copyTotp(id)), () => clearInterval(timer)]
}

// ------------------------------------------------------------ sync

void listen<Synced>('synced', (e) => applySync(e.payload))

/** When the app went to the background, and when it was last touched (ms). */
let hiddenAt = Date.now()
let lastTouch = Date.now()

/** Syncs on its own while unlocked: coming back to the app after a minute
 *  (AGAIN_AFTER), and as often as the setting says while it is in front;
 *  going away sends what is waiting. */
const AGAIN_AFTER = 60 * 1000
const syncIfOlder = (age: number) => {
  if (unlocked && !document.hidden && age > 0 && Date.now() - lastSynced >= age) void api.syncNow()
}
document.addEventListener('visibilitychange', () => {
  if (unlocked && document.hidden) {
    hiddenAt = Date.now()
    void api.syncIfPending()
    void api.lockLater()
  } else if (!document.hidden) {
    void api.stayUnlocked()
    lastTouch = Date.now()
    // The backend's timer may not have run on time (Android asleep): the clock decides.
    const after = settings?.lockInBackground
    if (unlocked && after != null && Date.now() - hiddenAt > after * 1000) void lock()
    else syncIfOlder(AGAIN_AFTER)
  }
})
setInterval(() => syncIfOlder((settings?.syncEveryMinutes ?? 5) * 60 * 1000), 30 * 1000)

// ------------------------------------------------------------ locking

for (const type of ['pointerdown', 'keydown', 'scroll']) document.addEventListener(type, () => (lastTouch = Date.now()), { passive: true, capture: true })
setInterval(() => {
  const idle = (settings?.lockAfterMinutes ?? 5) * 60 * 1000
  if (unlocked && !document.hidden && idle > 0 && Date.now() - lastTouch > idle) void lock()
}, 10 * 1000)
/** The screen turned off (SystemPlugin.kt): the backend locks when the setting says so. */
;(window as unknown as { pswmScreenOff: () => void }).pswmScreenOff = () => {
  if (unlocked) void api.screenOff().then((locked) => locked && afterLock())
}

void start()
