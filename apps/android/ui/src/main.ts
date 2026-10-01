import { listen } from '@tauri-apps/api/event'
import { el, button, busyButton, errorLine, enterPresses } from '../../../../src/dom'
import { formatDateTime, formatSize, splitCode, titleOf } from '../../../../src/entry-text'
import { ALL, GROUPS, sameFilter, search, tagCounts, type Filter } from '../../../../src/search'
import { svgIcon, type IconName } from './icons'
import { api, type CloudFile, type Entry, type Listing, type Picked, type SignedIn, type Status, type Synced } from './api'

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

/** What Back closes or returns to, innermost last (see BackPlugin.kt). */
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

async function start() {
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
  const back = button('←', 'Back', chooseScreen, 'icon')
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
    el('header', { className: 'bar' }, button('←', 'Back', chooseScreen, 'icon'), el('h1', {}, 'Pick the file')),
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
    el('header', { className: 'bar' }, button('←', 'Back', back, 'icon'), el('h1', {}, 'Where to keep it')),
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
  show([
    el('h1', {}, current.title),
    el('p', { className: 'muted' }, current.description),
    el('p', { className: 'muted' }, current.syncedWith ? `Syncs with ${current.syncedWith}` : ''),
    password,
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
  show([query ? searching : toolbar, scroll, findButton, status], undefined, 'list-screen')
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

function icon(entry: Entry, listing: Listing) {
  const custom = entry.customIcon && listing.customIcons[entry.customIcon]
  if (custom) return el('img', { className: 'icon', src: custom, alt: '' })
  return el('span', { className: 'icon letter' }, titleOf(entry).slice(0, 1).toUpperCase())
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
    button('Lock', 'Lock the database', () => void lock(), 'lock'),
  )
}

/** Shows the unlock screen after the database locked by itself (in the background). */
void listen('locked', () => {
  unlocked = false
  query = ''
  unlockScreen()
})

async function lock() {
  unlocked = false
  query = ''
  await api.lock()
  unlockScreen()
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
  const back = button('←', 'Back to the list', toList, 'icon')
  show([el('header', { className: 'bar' }, back, icon(entry, listing), el('h1', {}, titleOf(entry))), ...rows], toList)
  leave = () => stops.forEach((stop) => stop())
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

/** How long (ms) in the background, and in front without a touch, before the database locks. */
const LOCK_IN_BACKGROUND = 30 * 1000
const LOCK_WHEN_IDLE = 5 * 60 * 1000
/** When the app went to the background, and when it was last touched (ms). */
let hiddenAt = Date.now()
let lastTouch = Date.now()

/** Syncs on its own while unlocked: coming back to the app after a minute
 *  (AGAIN_AFTER), every 5 minutes while it is in front (EVERY); going away
 *  sends what is waiting. */
const AGAIN_AFTER = 60 * 1000
const EVERY = 5 * 60 * 1000
const syncIfOlder = (age: number) => {
  if (unlocked && !document.hidden && Date.now() - lastSynced >= age) void api.syncNow()
}
document.addEventListener('visibilitychange', () => {
  if (unlocked && document.hidden) {
    hiddenAt = Date.now()
    void api.syncIfPending()
    void api.lockLater(LOCK_IN_BACKGROUND / 1000)
  } else if (!document.hidden) {
    void api.stayUnlocked()
    lastTouch = Date.now()
    // The backend's timer may not have run on time (Android asleep): the clock decides.
    if (unlocked && Date.now() - hiddenAt > LOCK_IN_BACKGROUND) void lock()
    else syncIfOlder(AGAIN_AFTER)
  }
})
setInterval(() => syncIfOlder(EVERY), 30 * 1000)

// ------------------------------------------------------------ locking

for (const type of ['pointerdown', 'keydown', 'scroll']) document.addEventListener(type, () => (lastTouch = Date.now()), { passive: true, capture: true })
setInterval(() => {
  if (unlocked && !document.hidden && Date.now() - lastTouch > LOCK_WHEN_IDLE) void lock()
}, 10 * 1000)
/** The screen turned off (ScreenPlugin.kt): lock at once. */
;(window as unknown as { pswmScreenOff: () => void }).pswmScreenOff = () => {
  if (unlocked) void lock()
}

void start()
