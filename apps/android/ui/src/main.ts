import { listen } from '@tauri-apps/api/event'
import { el, button, busyButton, errorLine, enterPresses } from '../../../../src/dom'
import { EMPTY_ENTRY, chip, collectEntry, fieldRow, filesEditor, generatorPanel, input, showHide, strengthMeter } from '../../../../src/editor-parts'
import { beforeExtension, dateOf, formatDateTime, formatSize, labelOf, splitCode, titleOf } from '../../../../src/entry-text'
import { OTP, PASSWORD, URL_FIELD, USERNAME, type DatabaseSetting, type DatabaseSettings, type Encryption } from '../../../../src/api'
import { describeEncryption, encryptionForm, HEAVY_QUESTION, HISTORY_ITEMS, HISTORY_SIZE, versions, versionsGoing, withValue, type Choice } from '../../../../src/database-settings'
import { DEFAULT_ICON, glyphIcon } from '../../../../src/glyphs'
import { siteIconCache } from '../../../../src/site-icons'
import { ALL, FAVORITE, GROUPS, sameFilter, search, tagCounts, TRASH, UNTAGGED, type Filter } from '../../../../src/search'
import { GROUP_ICONS, shownIcon } from '../../../../src/icons'
import { tagInput } from '../../../../src/tag-input'
import { svgIcon, type IconName } from './icons'
import { api, type Settings, type Cloud, type CloudFile, type Entry, type EntryData, type EntryDetail, type Imported, type Version, type Listing, type Picked, type SignedIn, type Status, type Synced } from './api'

const screen = document.querySelector<HTMLElement>('#screen')!
const snackbar = document.querySelector<HTMLElement>('#snackbar')!

/** The groups the phone shows; Templates comes later. */
const PHONE_GROUPS = GROUPS.filter((g) => g.group !== 'templates')

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
/** The screen shown's answer to coming back from another app (the unlock screen's prompt). */
let onReturn = () => {}
/** When the app last came back from another app (ms). */
let returnedAt = 0

/** What Back closes or returns to, innermost last (see SystemPlugin.kt). */
let backs: (() => void)[] = []

/** Called by Android's Back: true when the page did something with it. */
;(window as unknown as { pswmBack: () => boolean }).pswmBack = () => {
  const back = backs.pop()
  back?.()
  return !!back
}

/** Shows a screen; `back` is where Back returns from it (none: Back leaves the app). */
/** Counts the screens shown, to tell whether another came meanwhile. */
let screenShown = 0

function show(children: Node[], back?: () => void, className = '') {
  screenShown++
  leave()
  leave = () => {}
  onSynced = () => {}
  onReturn = () => {}
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
  // In front now: a lock the credential provider set waits no longer (provider.rs).
  void api.stayUnlocked()
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
    ...CLOUDS.map((cloud) => el('button', { type: 'button', className: 'card', onclick: () => void cloudScreen(cloud) }, `Sync with ${STORES[cloud].name}`,
      el('small', {}, `Sign in once. The file lives in ${STORES[cloud].folder}; a working copy stays on this phone.`))),
    local,
    el('button', { type: 'button', className: 'card', onclick: () => newDatabaseScreen() }, 'Create a new database',
      el('small', {}, 'With a master password; it goes to a cloud store or a folder on this phone.')),
    el('p', { className: 'muted' }, 'A local file is picked with Android’s file picker; it can be anywhere the picker reaches, also a folder another app syncs.'),
    error.line,
  ])
}

/** A new database: its name and master password, then where it lives (a
 *  cloud store, with a copy in a folder on this phone, or a folder). */
function newDatabaseScreen() {
  const error = errorLine()
  const name = input('Passwords', { className: 'field', placeholder: 'Name' })
  const password = el('input', { type: 'password', autocomplete: 'new-password', placeholder: 'Master password', className: 'field' })
  const again = el('input', { type: 'password', autocomplete: 'new-password', placeholder: 'The master password again', className: 'field' })
  const strength = strengthMeter(password, api.passwordStrength)
  error.hideOnInput(name)
  error.hideOnInput(password)
  error.hideOnInput(again)
  const ready = () => {
    if (!name.value.trim()) throw new Error('Give the database a name')
    if (!password.value) throw new Error('Choose a master password')
    if (password.value !== again.value) throw new Error('The two passwords differ')
  }
  const create = async (place: Parameters<typeof api.createDatabase>[2]) => {
    const made = await api.createDatabase(name.value, password.value, place)
    password.value = again.value = ''
    database = made.status.database
    unlockScreen()
    if (made.copyProblem) snack(`${made.copyProblem}. Choose the folder again in the sync sheet.`)
  }
  const fail = (e: string) => error.show(e.replace(/^Error: /, ''))
  const inCloud = (cloud: Cloud) =>
    busyButton(STORES[cloud].name, `Put it into ${STORES[cloud].folder} in ${STORES[cloud].name}`, async () => {
      ready()
      // Signed in (the store is asked for its files), then the copy's folder.
      await filesIn(cloud)
      snack('Choose the folder for the copy on this phone')
      const picked = await api.pickFolder()
      if (picked) await create({ kind: 'cloud', cloud, folder: picked })
    }, fail, 'card')
  const onPhone = busyButton('On this phone', 'A local file in a folder you choose', async () => {
    ready()
    const picked = await api.pickFolder()
    if (picked) await create({ kind: 'folder', folder: picked })
  }, fail, 'card')
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Back', chooseScreen), el('h1', {}, 'New database')),
    name,
    password,
    again,
    strength.element,
    el('h2', {}, 'Where it lives'),
    ...CLOUDS.map(inCloud),
    onPhone,
    el('p', { className: 'muted' }, 'A file of the same name already there is never replaced. A key file can be added later in PswManager for Windows.'),
    error.line,
  ], chooseScreen)
  name.focus()
}

/** What to do when a sign-in ends (event `signed-in`). */
let onSignedIn = (_signedIn: SignedIn) => {}
void listen<SignedIn>('signed-in', (e) => onSignedIn(e.payload))

/** The stores the phone syncs with, and where the app's files are in each. */
const CLOUDS: Cloud[] = ['dropbox', 'onedrive', 'google']
const STORES: Record<Cloud, { name: string; folder: string }> = {
  dropbox: { name: 'Dropbox', folder: 'Apps / PswManager Sync' },
  onedrive: { name: 'OneDrive', folder: 'Apps / PswManager' },
  google: { name: 'Google Drive', folder: 'PswManager, at the top of the Drive' },
}

/** Signs in to `cloud` in the browser, then waits for it to come back. A
 *  sign-in started again gives up on the one before. */
function signIn(cloud: Cloud): Promise<void> {
  onSignedIn({ error: 'A new sign-in was started' })
  return new Promise((done, fail) => {
    onSignedIn = (signedIn) => {
      onSignedIn = () => {}
      if (signedIn.error) fail(signedIn.error)
      else done()
    }
    api.signIn(cloud).catch(fail)
  })
}

/** The databases in `cloud`'s app folder, signing in first when the store asks. */
async function filesIn(cloud: Cloud): Promise<CloudFile[]> {
  try {
    return await api.cloudFiles(cloud)
  } catch {
    await signIn(cloud)
    return api.cloudFiles(cloud)
  }
}

/** First run with a cloud store: sign in, then pick the file in the app's folder. */
async function cloudScreen(cloud: Cloud) {
  const { name } = STORES[cloud]
  const error = errorLine()
  const back = iconButton('back', 'Back', chooseScreen)
  const waiting = el('p', {}, `Waiting for ${name}…`)
  show([el('header', { className: 'bar' }, back, el('h1', {}, name)), el('p', { className: 'muted' }, 'Sign-in opens in the browser. Nothing is uploaded.'), waiting, error.line], chooseScreen)
  try {
    await signIn(cloud)
    filesScreen(cloud, await api.cloudFiles(cloud))
  } catch (e) {
    waiting.remove()
    error.show(String(e))
    screen.append(busyButton(`Sign in to ${name}`, 'Sign in again', () => cloudScreen(cloud), error.show, 'primary'))
  }
}

function filesScreen(cloud: Cloud, files: CloudFile[]) {
  const { name, folder } = STORES[cloud]
  const error = errorLine()
  const pick = (file: CloudFile) => button(file.name, `Use ${file.name}`, () => folderScreen(cloud, files, file), 'card')
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Back', chooseScreen), el('h1', {}, 'Pick the file')),
    el('p', { className: 'muted' }, folder),
    ...(files.length ? files.map(pick) : [el('p', {}, 'There is no .kdbx in the app’s folder yet. Put one there from PswManager on Windows (Settings → Sync → Upload) and come back.')]),
    el('p', { className: 'muted' }, `PswManager sees only its app folder in ${name}. PswManager for Windows opens the same file there.`),
    error.line,
  ], chooseScreen)
}

/** First run with a cloud store, step 3: where the visible copy goes on this phone. */
function folderScreen(cloud: Cloud, files: CloudFile[], file: CloudFile) {
  const error = errorLine()
  let folder: Picked | null = null
  const chosen = el('p', {}, 'No folder chosen yet.')
  const taken = el('p', { className: 'muted' })
  const go = busyButton('Download and continue', 'Download the database', async () => {
    const opened = await api.openCloudFile(cloud, file, folder!)
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
  const back = () => filesScreen(cloud, files)
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Back', back), el('h1', {}, 'Where to keep it')),
    el('p', {}, `${file.name} syncs with ${STORES[cloud].name}. A copy of it is kept in a folder on this phone, where you can see it and back it up; it is updated after every change.`),
    choose,
    chosen,
    taken,
    go,
    error.line,
  ], back)
}

// ------------------------------------------------------------ unlock

/** The list after an unlock; the sync may have finished before it was there to hear it. */
async function unlockedWith(opened: Listing) {
  unlocked = true
  listScreen(opened)
  const last = await api.lastSync()
  if (last) applySync(last)
}

/** How soon after coming back the unlock screen still counts as come back to (ms). */
const RETURN_PROMPT = 3000

function unlockScreen() {
  const current = database!
  const error = errorLine()
  const password = el('input', { type: 'password', autocomplete: 'off', placeholder: 'Master password', className: 'field' })
  // Biometric unlock, when its key is sealed: the fingerprint button, or Unlock
  // with no password typed, once (again, it unlocks with the key file alone).
  let prompted = false
  const unlock = busyButton('Unlock', 'Unlock the database', async () => {
    if (!password.value && !fingerprint.hidden && !prompted) return withBiometric()
    syncLine = 'Syncing…'
    const opened = await api.unlock(password.value)
    password.value = ''
    await unlockedWith(opened)
  }, error.show, 'primary')
  const withBiometric = async () => {
    prompted = true
    try {
      syncLine = 'Syncing…'
      await unlockedWith(await api.unlockWithBiometric())
    } catch (e) {
      if (String(e) !== 'cancelled') error.show(String(e))
      password.focus()
    }
  }
  const fingerprint = iconButton('fingerprint', 'Unlock with your fingerprint or face', () => void withBiometric(), 'icon fingerprint')
  fingerprint.hidden = true
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
    el('div', { className: 'together' }, password, fingerprint),
    keyLine,
    unlock,
    error.line,
    button('Use another database…', 'Forget this one (its file stays where it is)', forget, 'link'),
  ])
  password.focus()
  // Coming back from another app to this screen opens the prompt by itself
  // (the lock may have happened while away, just before the screen was shown).
  const prompt = (ready: boolean) => {
    if (ready && fingerprint.isConnected && Date.now() - returnedAt < RETURN_PROMPT) {
      // Shown before the app is fully back, Android would cancel it.
      window.setTimeout(() => fingerprint.isConnected && void withBiometric(), 400)
    }
  }
  void api.biometricReady().then((ready) => {
    fingerprint.hidden = !ready
    prompt(ready)
    onReturn = () => prompt(!fingerprint.hidden)
  }, () => {})
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
  const list = el('ul', { className: 'entries', role: 'listbox', ariaMultiSelectable: 'true', ariaLabel: 'Entries' })
  const inBin = () => search(listing.entries, '', TRASH).length
  const emptyBin = button('Empty the recycle bin…', 'Delete everything in it for good', () =>
    confirmSheet(`Delete ${entriesCount(inBin())} permanently? This cannot be undone: other devices delete them too when they sync.`, 'Delete permanently', () =>
      void api.emptyTrash().then(toListSaying('Deleted permanently'), (e) => snack(String(e)))), 'link danger empty-bin')
  // Several entries chosen with a long press (not in the trash): a bar of
  // what can be done to them all replaces the toolbar until it is closed.
  const chosen = new Set<string>()
  const chosenTitle = el('h1', {})
  const copyChosen = iconButton('copy', 'Copy from it', () => quickCopy(listing.entries.find((e) => chosen.has(e.id))!))
  const selecting = el('header', { className: 'bar', hidden: true },
    iconButton('close', 'Clear the selection', () => clearSelection()),
    chosenTitle,
    copyChosen,
    iconButton('tag', 'Add or remove a tag', () => tagSheet([...chosen], listing, changedSeveral)),
    iconButton('trash', 'Move to the recycle bin', () =>
      confirmSheet(`Move ${entriesCount(chosen.size)} to the recycle bin?`, 'Delete', () =>
        void api.deleteEntries([...chosen]).then(toListSaying('Moved to the recycle bin'), (e) => snack(String(e))))))
  const toggle = (id: string) => {
    if (!chosen.delete(id)) chosen.add(id)
    marked()
  }
  const clearSelection = () => {
    chosen.clear()
    marked()
  }
  /** The bar and the rows as the selection is. */
  const marked = () => {
    const on = chosen.size > 0
    chosenTitle.textContent = `${chosen.size} selected`
    copyChosen.hidden = chosen.size !== 1
    selecting.hidden = !on
    toolbar.hidden = searching.hidden = on
    findButton.hidden = on || searching.isConnected
    add.hidden = on
    // Back clears it, after whatever was open over it (a sheet) is closed.
    if (!on) backs = backs.filter((b) => b !== clearSelection)
    else if (!backs.includes(clearSelection)) backs.push(clearSelection)
    for (const item of list.querySelectorAll<HTMLElement>('li[data-id]')) {
      const picked = chosen.has(item.dataset.id!)
      item.classList.toggle('selected', picked)
      item.setAttribute('aria-selected', String(picked))
    }
  }
  const changedSeveral = (fresh: Listing, message: string) => {
    listing = fresh
    chosen.clear()
    fill()
    snack(message)
  }
  const pick = (entry: Entry) => {
    if (chosen.size) toggle(entry.id)
    else void entryScreen(entry.id, listing)
  }
  const press = (entry: Entry) => (sameFilter(filter, TRASH) ? quickCopy(entry) : toggle(entry.id))
  const fill = () => {
    title.textContent = filterLabel(filter)
    searchField.placeholder = `Search ${filterLabel(filter)}`
    emptyBin.hidden = !sameFilter(filter, TRASH) || !inBin()
    const shown = search(listing.entries, query, filter)
    // What is no longer shown is no longer chosen.
    for (const id of [...chosen]) if (!shown.some((e) => e.id === id)) chosen.delete(id)
    list.replaceChildren(...shown.map((entry) => row(entry, listing, pick, press)))
    if (shown.length === 0) list.append(el('li', { className: 'muted empty' }, 'Nothing here.'))
    marked()
  }
  searchField.addEventListener('input', () => {
    query = searchField.value
    fill()
  })
  status.addEventListener('click', syncSheet)
  const scroll = el('div', { className: 'scroll' }, emptyBin, list)
  pullToSync(scroll, () => (status.textContent = 'Syncing…'))
  // Not while entries are chosen (the bar has the list then), nor when turned off.
  swipes(scroll, () => !chosen.size && swipesOn(), {
    right: () => drawer(listing, fill, true),
    left: () => settingsPeek(() => listScreen(listing)),
  })
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
  show([selecting, query ? searching : toolbar, scroll, el('div', { className: 'fabs' }, findButton, add), status], undefined, 'list-screen')
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

/** Sideways swipes across `target` (#194) bring a panel in with the finger:
 *  to the right `right`'s, to the left `left`'s. Let go past a third of the
 *  way (or flicked), it opens; otherwise it goes back. A swipe counts once it
 *  is clearly sideways, so the list's scrolling and pull-to-sync are not taken
 *  for one; one that starts at the screen's edge is left to Android's Back
 *  gesture. `allowed` says whether swipes are on now. */
function swipes(target: HTMLElement, allowed: () => boolean, to: { right?: () => Sliding; left?: () => Sliding }) {
  const EDGE = 32
  const START = 12
  let from: { x: number; y: number; at: number } | null = null
  let panel: Sliding | null = null
  let dx = 0
  target.addEventListener('touchstart', (e) => {
    // Another finger mid-swipe: the panel goes back, and that is the end of it.
    panel?.release(false)
    panel = null
    dx = 0
    const { clientX: x, clientY: y } = e.touches[0]
    const inside = x > EDGE && x < window.innerWidth - EDGE
    from = e.touches.length === 1 && inside && allowed() ? { x, y, at: e.timeStamp } : null
  }, { passive: true })
  target.addEventListener('touchmove', (e) => {
    if (!from) return
    dx = e.touches[0].clientX - from.x
    const dy = e.touches[0].clientY - from.y
    if (!panel) {
      if (Math.abs(dy) > START && Math.abs(dy) >= Math.abs(dx)) from = null // a scroll
      else if (Math.abs(dx) > START && Math.abs(dx) > 2 * Math.abs(dy)) {
        const open = dx > 0 ? to.right : to.left
        if (open) panel = open()
        else from = null // nothing that way
      }
      if (!panel) return
    }
    e.preventDefault()
    panel.follow(dx)
  }, { passive: false })
  const end = (e: TouchEvent) => {
    if (!from || !panel) return
    const speed = Math.abs(dx) / Math.max(1, e.timeStamp - from.at)
    panel.release(e.type === 'touchend' && (Math.abs(dx) > window.innerWidth / 3 || speed > 0.6))
    from = null
    panel = null
  }
  target.addEventListener('touchend', end)
  target.addEventListener('touchcancel', end)
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

const entriesCount = (n: number) => `${n} ${n === 1 ? 'entry' : 'entries'}`

/** After a change made from an entry: the list again, and what happened. */
const toListSaying = (message: string) => (fresh: Listing) => (listScreen(fresh), snack(message))

function filterLabel(f: Filter) {
  if (f.kind === 'tag') return f.tag
  if (f.kind === 'untagged') return 'Untagged'
  return PHONE_GROUPS.find((g) => g.group === f.group)!.label
}

/** An entry in the list: a tap does `tap`, a long press `press`. */
function row(entry: Entry, listing: Listing, tap: (entry: Entry) => void, press: (entry: Entry) => void): HTMLLIElement {
  const item = el('li', { tabIndex: 0, role: 'option' }, icon(entry, listing), el('span', {}, el('b', {}, titleOf(entry)), el('small', {}, entry.username)))
  item.dataset.id = entry.id
  item.addEventListener('click', () => tap(entry))
  item.addEventListener('contextmenu', (e) => {
    e.preventDefault()
    press(entry)
  })
  return item
}

/** Adds a tag to entries or takes one off: the tag typed or picked among the
 *  database's (to add) or theirs (to take off). */
function tagSheet(ids: string[], listing: Listing, done: (fresh: Listing, message: string) => void) {
  const theirs = [...new Set(listing.entries.filter((e) => ids.includes(e.id)).flatMap((e) => e.tags))].filter((t) => t !== FAVORITE).sort((a, b) => a.localeCompare(b))
  const known = tagCounts(listing.entries).map(([tag]) => tag)
  const change = (tag: string, on: boolean, close: () => void) => {
    close()
    void api.setTag(ids, tag, on).then((fresh) => done(fresh, `${on ? 'Tagged' : 'Untagged'} ${entriesCount(ids.length)}: #${tag.trim()}`), (e) => snack(String(e)))
  }
  sheet((close) => {
    const name = input('', { className: 'field', ariaLabel: 'Tag', placeholder: 'Tag to add' })
    const offered = el('div', { className: 'tag-offers' })
    // The first dozen that match what is typed: enough to pick from without scrolling the sheet.
    const offer = () =>
      offered.replaceChildren(...known.filter((t) => t.toLowerCase().includes(name.value.trim().toLowerCase())).slice(0, 12)
        .map((t) => button(`#${t}`, `Add the tag ${t}`, () => change(t, true, close), 'tag-offer')))
    name.addEventListener('input', offer)
    offer()
    const addTyped = button('Add', 'Add the tag typed', () => name.value.trim() && change(name.value, true, close), 'primary')
    enterPresses(addTyped, name)
    return [
      el('b', {}, `Tag ${entriesCount(ids.length)}`),
      name, offered, addTyped,
      ...(theirs.length ? [el('b', {}, 'Take off'), ...theirs.map((t) => button(`#${t}`, `Take the tag ${t} off`, () => change(t, false, close), 'item danger'))] : []),
    ]
  })
}

/** Site icons by host; one that arrives replaces the key in the images waiting for it. */
const siteIcons = siteIconCache(api.icon, (host, src) => {
  document.querySelectorAll<HTMLImageElement>('img.icon[data-host]').forEach((img) => {
    if (img.dataset.host !== host) return
    img.src = src
    img.classList.add('site')
  })
})
void listen<string>('icon-ready', (e) => siteIcons.refresh(e.payload))

/** An entry's own image, else the drawing chosen for it, else its site's icon
 *  (when it arrives), else the key: as on Windows. Images drawn for light
 *  pages (the entry's own, a site's) are marked `site` for their light tile. */
function icon(entry: Entry, listing: Listing) {
  const custom = entry.customIcon && listing.customIcons[entry.customIcon]
  if (custom) return el('img', { className: 'icon site', src: custom, alt: '' })
  if (entry.icon !== null) return el('img', { className: 'icon', src: glyphIcon(entry.icon), alt: '' })
  const site = entry.host && siteIcons.get(entry.host)
  const img = el('img', { className: site ? 'icon site' : 'icon', src: site || DEFAULT_ICON, alt: '' })
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

/** Signs in to `cloud` again (the store asked), then syncs. */
const signInButton = (cloud: Cloud, close: () => void) =>
  button('Sign in', 'Sign in again', () => (close(), void signIn(cloud).then(() => api.syncNow(), (e) => snack(String(e)))), 'primary')

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
    ...(signInAgain && database?.cloud ? [signInButton(database.cloud, close)] : []),
    button('Sync now', 'Sync now', () => {
      close()
      void api.syncNow()
    }, 'primary'),
  ])
}

/** The drawer; it slides in, unless a swipe brings it in step with the finger (`dragged`). */
function drawer(listing: Listing, changed: () => void, dragged = false): Sliding {
  const panel = el('nav', { className: 'drawer' })
  const shade = el('div', { className: 'shade' }, panel)
  const close = overlay(shade)
  const item = (f: Filter, label: string, count: number, icon?: IconName) => {
    const choose = () => {
      filter = f
      close()
      changed()
    }
    const name = el('span', {}, ...(icon ? [svgIcon(icon)] : []), label)
    const b = el('button', { type: 'button', onclick: choose }, name, el('small', {}, String(count)))
    b.classList.toggle('tag', f.kind === 'tag')
    b.classList.toggle('chosen', sameFilter(f, filter))
    return b
  }
  const entries = listing.entries
  panel.append(
    el('b', { className: 'title' }, database?.title ?? ''),
    el('small', {}, [database?.syncedWith, syncLine].filter(Boolean).join(' · ')),
    el('h2', {}, 'Groups'),
    ...PHONE_GROUPS.map(({ group, label }) => {
      const f: Filter = { kind: 'group', group }
      return item(f, label, search(entries, '', f).length, GROUP_ICONS[group])
    }),
    el('h2', {}, 'Tags'),
    item(UNTAGGED, 'Untagged', search(entries, '', UNTAGGED).length),
    ...tagCounts(entries).map(([tag, count]) => item({ kind: 'tag', tag }, tag, count)),
    button('Import from another app…', 'Passwords and passkeys from another password manager on this phone', () => (close(), importSheet()), 'action'),
    button('Settings', 'Settings', () => (close(), void settingsScreen(() => listScreen(listing))), 'action'),
    button('Lock', 'Lock the database', () => void lock(), 'action'),
  )
  const sliding = slide(shade, panel, -1, close)
  if (!dragged) sliding.release(true)
  // A swipe back to the left takes it away with the finger.
  swipes(shade, swipesOn, { left: () => closing(sliding) })
  return sliding
}

/** A panel coming in from a side over a shade: in step with a finger
 *  (`follow`, by how far it has moved), then let go (`release`), open or
 *  back out (`close` then). */
interface Sliding {
  follow: (dx: number) => void
  release: (open: boolean, opened?: () => void) => void
}

/** Swipes are on unless the setting turned them off. */
const swipesOn = () => settings?.swipes !== false

/** `sliding`, open, taken away by a swipe the other way: let go far enough, it closes. */
const closing = (sliding: Sliding & { back: (dx: number) => void }): Sliding => ({
  follow: sliding.back,
  release: (away) => sliding.release(!away),
})

/** Runs `done` once `node`'s own transform transition ends, or soon anyway
 *  (none may come: the app went to the background, or nothing moved). */
function afterSlide(node: HTMLElement, done: () => void) {
  let finished = false
  const finish = () => {
    if (finished) return
    finished = true
    node.removeEventListener('transitionend', ended)
    done()
  }
  const ended = (e: TransitionEvent) => e.target === node && finish()
  node.addEventListener('transitionend', ended)
  setTimeout(finish, 400)
}

/** The screen shown, taken away to the right by a swipe; let go far enough, `away` runs. */
function leaving(node: HTMLElement, away: () => void): Sliding {
  let moved = 0
  const place = (x: string) => (node.style.transform = x)
  return {
    follow: (dx) => {
      node.style.transition = 'none'
      moved = Math.max(0, dx)
      place(`translateX(${moved}px)`)
    },
    release: (go) => {
      const before = screenShown
      // The next screen first, then the node back in place: no flash of the
      // old one. Not when Back already left this screen meanwhile.
      const done = () => {
        if (go && screenShown === before) away()
        node.style.transition = ''
        place('')
      }
      if (!go && moved === 0) return done()
      node.style.transition = 'transform 200ms ease-out'
      requestAnimationFrame(() => {
        afterSlide(node, done)
        place(go ? 'translateX(100%)' : 'translateX(0)')
      })
    },
  }
}

/** A panel coming in over `shade` from its side (`sign` -1: the left, 1: the
 *  right); `close` takes it away when it is let go short of open. */
function slide(shade: HTMLElement, panel: HTMLElement, sign: -1 | 1, close: () => void): Sliding & { back: (dx: number) => void } {
  let shown = 0
  const show = (part: number) => {
    shown = part
    panel.style.transform = `translateX(${sign * (1 - part) * 100}%)`
    shade.style.backgroundColor = `rgb(0 0 0 / ${Math.round(40 * part)}%)`
  }
  const part = (dx: number) => Math.min(1, Math.max(0, (-sign * dx) / panel.offsetWidth))
  shade.classList.add('following')
  show(0)
  return {
    follow: (dx) => show(part(dx)),
    // Open, moved back towards its side.
    back: (dx) => {
      shade.classList.add('following')
      show(1 - part(-dx))
    },
    release: (open, opened) => {
      shade.classList.remove('following')
      const done = () => (open ? opened?.() : close())
      // Already there: no transition to wait for.
      if (shown === (open ? 1 : 0)) return done()
      // Laid out where the finger left it (or off screen, just added), so the
      // transition starts from there.
      void panel.offsetWidth
      afterSlide(panel, done)
      show(open ? 1 : 0)
    },
  }
}

/** The settings' toolbar, coming in from the right with a swipe; once in, the
 *  settings screen takes its place. */
function settingsPeek(back: () => void): Sliding {
  // The screen itself, drawn now; once in, the same parts become the screen.
  // Before the settings were ever read, only its toolbar's look.
  const parts = settings ? settingsParts(settings, back) : null
  const panel = el('main', { className: 'peek' }, ...(parts?.nodes ?? [el('header', { className: 'bar' }, el('span', { className: 'icon' }, svgIcon('back')), el('h1', {}, 'Settings'))]))
  const shade = el('div', { className: 'shade' }, panel)
  const close = overlay(shade)
  const sliding = slide(shade, panel, 1, close)
  return { ...sliding, release: (open) => sliding.release(open, () => (close(), void settingsScreen(back, parts ?? undefined))) }
}

/** Importing from another password manager on this phone (#152): what it does, then Android's list of apps. */
function importSheet() {
  sheet((close) => [
    el('b', {}, 'Import from another app'),
    el('p', {}, 'Android lists the password managers on this phone that can hand over their passwords. Everything the one you choose hands over is added as new entries: nothing here is changed or replaced.'),
    el('p', { className: 'muted' }, 'Passwords, passkeys, TOTP secrets and notes come over; attached files do not. The entries go to a group of their own, shown under All.'),
    button('Choose the app', 'Choose the app to import from', () => (close(), void runImport()), 'primary'),
    button('Cancel', 'Cancel', close, 'link'),
  ])
}

async function runImport() {
  try {
    const imported = await api.importFromApp()
    if (!imported) return
    if (imported.added) listScreen(imported.listing)
    importedSheet(imported)
  } catch (e) {
    snack(String(e))
  }
}

/** What an import brought, and what it could not. */
function importedSheet({ added, exporter, group, skipped }: Imported) {
  sheet((close) => [
    el('b', {}, `${added} ${added === 1 ? 'entry' : 'entries'} from ${exporter}`),
    el('p', {}, added ? `In the group “${group}”, shown under All.` : 'Nothing was added.'),
    ...(skipped.length
      ? [el('h2', {}, `Not brought over (${skipped.length})`), el('ul', { className: 'skipped' }, ...skipped.map((s) => el('li', {}, el('b', {}, s.title || 'Untitled'), `: ${s.why}`)))]
      : []),
    button('OK', 'OK', close, 'primary'),
  ])
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
      database = (await api.syncWithCloud(cloud, file)).database
      syncLine = 'Syncing…'
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

/** Stop syncing and Disconnect, for a database synced with a cloud store. */
function stopSyncing(current: NonNullable<Status['database']>): Node[] {
  const store = current.syncedWith ?? 'the store'
  const after = current.copyFolder
    ? `From then on it syncs with its copy in ${current.copyFolder}, like a local file.`
    : 'There is no copy on this phone: the database stays inside the app only.'
  const stop = (signOut: boolean) => async () => {
    try {
      database = (await api.stopSyncing(signOut)).database
      syncLine = ''
      snack(signOut ? 'Disconnected' : 'Syncing stopped')
      void settingsScreen(settingsBack)
    } catch (e) {
      snack(String(e))
    }
  }
  return [
    el('h2', {}, 'Store'),
    button('Stop syncing', 'Stop syncing with the store', () =>
      confirmSheet(`Stop syncing with ${store}? The file there stays as it is. ${after}`, 'Stop syncing', () => void stop(false)()), 'wide'),
    button('Disconnect…', 'Stop syncing and sign out of the store', () =>
      confirmSheet(`Stop syncing and sign out of ${store}? The file there stays as it is. ${after}`, 'Disconnect', () => void stop(true)()), 'wide'),
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
    el('p', { className: 'muted' }, 'The master password and key file are changed on Windows or in Keepass2Android for now.'),
  ]
}

/** Another cipher and / or key derivation, with a Test that times an unlock on this phone. */
function encryptionScreen(now: Encryption) {
  const error = el('p', { className: 'error', hidden: true })
  const showError = (message: string) => ((error.textContent = message), (error.hidden = false))
  const form = encryptionForm(now, api.encryptionUnlockTime, showError, 'this phone', () => (error.hidden = true))
  // The settings again, read afresh.
  const back = () => void settingsScreen(settingsBack)
  const save = async () => {
    const ms = form.unchanged() ? 0 : await form.measure()
    if (ms === null) return
    const change = () => void api.setEncryption(form.wanted()).then(() => (back(), snack('Encryption changed')), (e) => showError(String(e)))
    if (form.heavy(ms)) confirmSheet(HEAVY_QUESTION, 'Change', change)
    else change()
  }
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Back', back), el('h1', {}, 'Encryption')),
    el('div', { className: 'encryption' }, ...form.fields),
    error,
    busyButton('Change', 'Save the database with this encryption', save, showError, 'primary'),
  ], back)
}

/** The settings screen's parts, drawn from the settings as last read: the
 *  screen shows them, and so does a swipe bringing it in (whole, not empty). */
interface SettingsParts {
  nodes: Node[]
  body: HTMLElement
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
  const database = () => {
    if (kept) return databaseTab(kept, (d) => (kept = d), fill)
    void api.databaseSettings().then((d) => ((kept = d), settingsTab === 'database' && fill()), (e) => snack(String(e)))
    return [el('p', { className: 'muted' }, 'Reading the database settings…')]
  }
  const fill = () => {
    bar.querySelectorAll('button').forEach((b, i) => b.classList.toggle('chosen', tabs[i][0] === settingsTab))
    body.replaceChildren(...(settingsTab === 'database' ? database() : tab(settingsTab, settings ?? s)))
  }
  const bar = el('nav', { className: 'tabs' }, ...tabs.map(([key, label]) => button(label, label, () => ((settingsTab = key), fill()), 'tab')))
  const goBack = () => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur()
    back()
  }
  fill()
  return {
    nodes: [el('header', { className: 'bar' }, iconButton('back', 'Back', goBack), el('h1', {}, 'Settings')), scrolledTabs(bar), body],
    body,
    fill,
    goBack,
    synced: () => {
      kept = null
      // Shown again unless a field is being typed in.
      if (settingsTab === 'database' && !body.contains(document.activeElement)) fill()
    },
  }
}

/** The settings screen; `drawn` when a swipe brought its parts in already. */
async function settingsScreen(back: () => void, drawn?: SettingsParts) {
  settingsBack = back
  if (!settings) applySettings(await api.settings())
  const parts = drawn ?? settingsParts(settings!, back)
  show(parts.nodes, parts.goBack)
  // A swipe to the right takes the settings away, back where they came from.
  swipes(parts.body, swipesOn, { right: () => leaving(screen, parts.goBack) })
  onSynced = (synced) => synced.changed && parts.synced()
  parts.body.parentElement?.querySelector('.tabs .chosen')?.scrollIntoView({ inline: 'nearest', block: 'nearest' })
  // As they are now (another device or the tray may have changed one).
  // Redrawn only when one did, so a choice being made is not disturbed.
  const shown = JSON.stringify(settings)
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
        el('p', {}, database?.syncedWith ? `Syncs with ${database.syncedWith}` : 'Not synced'),
        ...(database?.cloud ? [el('p', { className: 'muted' }, database.copyFolder ? `Copy on this phone: ${database.copyFolder}` : 'No copy on this phone yet')] : []),
        choice('Check for changes', 'While the app is in front and unlocked', 'syncEveryMinutes', s.syncEveryMinutes, minutes('Off')),
        button('Sync now', 'Sync now', () => void api.syncNow().then(() => snack('Syncing…')), 'primary'),
        ...(database ? (database.cloud ? stopSyncing(database) : syncWithStore()) : []),
        el('p', { className: 'muted' }, 'To use another database, lock this one and choose “Use another database…”: its file stays where it is.'),
      ]
    case 'about':
      return [
        el('p', {}, `PswManager for Android, version ${s.version}`),
        el('p', { className: 'muted' }, 'The database is a KeePass file (KDBX 4.1): it opens in PswManager on Windows, KeePassXC and Keepass2Android.'),
      ]
  }
}

// ------------------------------------------------------------ an entry

/** A command at the end of a line: what it does (for screen readers) and its icon. */
type Action = [label: string, icon: IconName, run: () => void]

/** A labelled value with its commands as icons at the end (Copy first);
 *  tapping the value runs the first of them (copies it, or opens a file).
 *  `extra` are buttons made by the caller (Show / hide). */
function line(label: string, value: Node | string, copy: (() => Promise<number>) | null, more: Action[] = [], extra: HTMLElement[] = []) {
  const shown = el('span', {}, el('small', {}, label), typeof value === 'string' ? el('span', {}, value) : value)
  const actions: Action[] = [...(copy ? [['Copy', 'copy', () => void copied(copy())] as Action] : []), ...more]
  if (actions.length) shown.addEventListener('click', actions[0][2])
  // Copy, then Show / hide, then the others (Open), as on Windows.
  const buttons: HTMLElement[] = actions.map(([name, icon, run]) => iconButton(icon, `${label}: ${name}`, run, 'icon line-action'))
  buttons.splice(copy ? 1 : 0, 0, ...extra)
  return el('div', { className: 'line' }, shown, ...buttons)
}

/** A secret's line (of an older version with `version`): masked, with an eye to show or hide it. */
function secretLine(id: string, label: string, field: string, version: number | null = null) {
  const mask = '••••••••••••'
  const value = el('span', { className: 'masked' }, mask)
  let shown = false
  const eye = iconButton('eye', `${label}: show`, () => void toggle(), 'icon line-action')
  const toggle = async () => {
    shown = !shown
    value.textContent = shown ? await api.reveal(id, field, version) : mask
    value.classList.toggle('masked', !shown)
    eye.replaceChildren(shownIcon(shown))
    eye.title = `${label}: ${shown ? 'hide' : 'show'}`
    eye.setAttribute('aria-label', eye.title)
  }
  return line(label, value, () => api.copyField(id, field, version), [], [eye])
}

/** The lines of an entry, or of its older version `version`: values are
 *  copied and revealed from that version, its files opened from it. A
 *  version's URL is not opened, and its TOTP secret is a secret, not codes.
 *  Returns the lines and what stops the TOTP countdown. */
function entryLines(entry: EntryDetail, version: number | null): [Node[], () => void] {
  const id = entry.id
  const copy = (field: string) => () => api.copyField(id, field, version)
  const rows: Node[] = []
  const tags = entry.tags.filter((tag) => tag !== FAVORITE)
  if (tags.length) rows.push(line('Tags', el('span', { className: 'entry-tags' }, ...tags.map((tag) => el('span', { className: 'tag-chip' }, tag))), null))
  let stop = () => {}
  if (entry.username) rows.push(line('User name', entry.username, copy(USERNAME)))
  if (entry.hasPassword) rows.push(secretLine(id, 'Password', PASSWORD, version))
  if (version === null && entry.otp) {
    const [node, stopTotp] = totpLine(id)
    rows.push(node)
    stop = stopTotp
  } else if (version !== null && entry.fields.some((f) => f.name === OTP)) {
    rows.push(secretLine(id, 'TOTP', OTP, version))
  }
  if (entry.url) {
    const open: Action[] = version === null ? [['Open in the browser', 'externalLink', () => void api.openUrl(id).catch((e) => snack(String(e)))]] : []
    rows.push(line('URL', entry.url, copy(URL_FIELD), open))
  }
  for (const field of entry.fields.filter((f) => f.name !== OTP)) {
    const label = labelOf(field.name)
    if (field.protected) rows.push(secretLine(id, label, field.name, version))
    else if (field.value) rows.push(line(label, field.value, copy(field.name)))
  }
  if (entry.notes) rows.push(el('div', { className: 'line notes' }, el('span', {}, el('small', {}, 'Notes'), el('span', {}, entry.notes))))
  if (entry.attachments.length) {
    const open = (name: string) => void api.openAttachment(id, name, version).catch((e) => snack(String(e)))
    const save = (name: string) => void api.saveAttachment(id, name, version).then((saved) => saved && snack(`${name} saved`)).catch((e) => snack(String(e)))
    rows.push(el('h2', {}, 'Attachments'), ...entry.attachments.map((a) => line(a.name, formatSize(a.size), null, [
      ['Open', 'fileOutput', () => open(a.name)],
      ['Save to a file', 'download', () => save(a.name)],
    ])))
  }
  return [rows, stop]
}

/** A sync that changed entries shows entry `id` again, or the list when it is gone. */
function showAgainOnSync(id: string) {
  onSynced = (synced) => {
    if (synced.changed) void api.listing().then((fresh) => (fresh.entries.some((e) => e.id === id) ? entryScreen(id, fresh) : listScreen(fresh)))
  }
}

async function entryScreen(id: string, listing: Listing) {
  const entry = await api.entry(id)
  const [rows, stop] = entryLines(entry, null)
  if (entry.modified) rows.push(el('p', { className: 'muted' }, `Changed ${formatDateTime(entry.modified)}`))
  if (entry.versions) rows.push(button(`History (${entry.versions})`, 'Older versions of this entry', () => void historyScreen(entry, listing), 'link'))
  const toList = () => listScreen(listing)
  const back = iconButton('back', 'Back to the list', toList)
  const inBin = entry.kind === 'trash'
  if (inBin) rows.push(trashButtons(entry))
  const commands = inBin ? [] : entryCommands(entry, listing)
  show([el('header', { className: 'bar' }, back, icon(entry, listing), el('h1', {}, titleOf(entry)), ...commands), ...rows], toList)
  leave = stop
  showAgainOnSync(id)
}

/** The toolbar's star, pencil and ⋮ (with Delete) of an entry in use. */
function entryCommands(entry: EntryDetail, listing: Listing): HTMLElement[] {
  const id = entry.id
  const starred = entry.tags.includes(FAVORITE)
  const star = iconButton('star', starred ? 'Not favorite' : 'Favorite', () =>
    void api.setTag([id], FAVORITE, !starred).then((fresh) => entryScreen(id, fresh), (e) => snack(String(e))), starred ? 'icon starred' : 'icon')
  const edit = iconButton('pencil', 'Edit', () => void editorScreen(id, listing, () => void entryScreen(id, listing)))
  const remove = () =>
    confirmSheet(`Move “${titleOf(entry)}” to the recycle bin?`, 'Delete', () =>
      void api.deleteEntries([id]).then(toListSaying('Moved to the recycle bin'), (e) => snack(String(e))))
  const more = iconButton('more', 'More', () => sheet((close) => [el('b', {}, titleOf(entry)), button('Delete', 'Move to the recycle bin', () => (close(), remove()), 'item')]))
  return [star, edit, more]
}

/** An entry in the recycle bin is not edited: it is restored or deleted for good. */
function trashButtons(entry: EntryDetail): HTMLElement {
  const restore = button('Restore', 'Put it back where it was', () =>
    void api.restoreEntry(entry.id).then(toListSaying('Restored'), (e) => snack(String(e))), 'primary')
  const remove = button('Delete permanently…', 'Delete it for good, on every device', () =>
    confirmSheet(`Delete “${titleOf(entry)}” permanently? This cannot be undone: other devices delete it too when they sync.`, 'Delete permanently', () =>
      void api.deleteForGood(entry.id).then(toListSaying('Deleted permanently'), (e) => snack(String(e)))), 'link danger')
  return el('div', { className: 'trash-actions' }, restore, remove)
}

// ------------------------------------------------------------ history

const savedAt = (v: { modified: string | null }) => (v.modified ? formatDateTime(v.modified) : '(no date)')

/** The entry's older versions, newest first: when each was saved and what
 *  changed after it (names, never values). Read only. */
async function historyScreen(entry: EntryDetail, listing: Listing) {
  let versions: Version[]
  try {
    versions = await api.entryHistory(entry.id)
  } catch (e) {
    snack(String(e))
    return
  }
  const toEntry = () => void entryScreen(entry.id, listing)
  const items = versions.map((v, i) => {
    const item = el('li', { tabIndex: 0 }, el('span', {},
      el('b', {}, savedAt(v)),
      el('small', {}, v.changed.length ? `then changed: ${v.changed.join(', ')}` : 'nothing shown changed after it')))
    item.addEventListener('click', () => void versionScreen(entry, listing, i))
    return item
  })
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Back to the entry', toEntry), el('h1', {}, 'History')),
    el('p', { className: 'muted' }, `${titleOf(entry)} · ${versions.length} older ${versions.length === 1 ? 'version' : 'versions'}`),
    el('ul', { className: 'entries versions' }, ...items),
  ], toEntry)
  showAgainOnSync(entry.id)
}

/** An older version, shown like the entry, read only, with what differs in
 *  the entry now (a secret only says that it differs). No restoring here. */
async function versionScreen(entry: EntryDetail, listing: Listing, index: number) {
  let at
  try {
    at = await api.entryVersion(entry.id, index)
  } catch (e) {
    snack(String(e))
    return
  }
  const [rows] = entryLines(at, index)
  if (at.differs.length) {
    rows.push(el('h2', {}, 'The entry now'), ...at.differs.map((d) =>
      el('div', { className: 'line' }, el('span', {}, el('small', {}, labelOf(d.name)),
        el('span', {}, d.protected ? 'differs (not shown)' : d.current ?? 'not in the entry now')))))
  }
  const toHistory = () => void historyScreen(entry, listing)
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Back to the history', toHistory), el('h1', {}, titleOf(at))),
    el('p', { className: 'muted' }, `Version saved ${savedAt(at)} · read only`),
    ...rows,
  ], toHistory)
  showAgainOnSync(entry.id)
}

// ------------------------------------------------------------ editing

/** Asks for a file's new name in a sheet; null when cancelled. */
function askName(current: string): Promise<string | null> {
  return new Promise((done) => {
    const name = input(current, { className: 'field', ariaLabel: 'New name' })
    sheet((close) => {
      const answer = (value: string | null) => (close(), done(value))
      const rename = button('Rename', 'Rename', () => answer(name.value), 'primary')
      enterPresses(rename, name)
      return [el('b', {}, 'Rename the file'), name, rename, button('Cancel', 'Cancel', () => answer(null), 'link')]
    })
    // The name without its extension is chosen, ready to type over.
    name.focus()
    name.setSelectionRange(0, beforeExtension(current))
  })
}

/** The editor, as `spec.md` *Editing* has it; a new blank entry when `id` is
 *  null. Cancel and Back return with `back`; saving shows the entry. */
async function editorScreen(id: string | null, listing: Listing, back: () => void) {
  let data: EntryData
  let attachments: EntryDetail['attachments'] = []
  try {
    // A new entry goes to the top group, with the database's default user name.
    data = id ? await api.editEntry(id) : { ...EMPTY_ENTRY, username: listing.database.defaultUsername }
    if (id) attachments = (await api.entry(id)).attachments
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
  const expires = el('input', { type: 'date', value: data.expires ? dateOf(data.expires) : '', className: 'field' })
  const notes = el('textarea', { value: data.notes, rows: 4, spellcheck: false, className: 'field' })
  const fieldList = el('div', { className: 'fields' }, ...data.fields.map((f) => fieldRow(f)))
  const files = filesEditor(attachments, {
    pick: api.pickFileToAttach,
    release: api.releaseFiles,
    askName,
  }, (message) => showError(message))
  const strength = strengthMeter(password, api.passwordStrength)
  const generator = generatorPanel(api.generatePassword, (chosen) => {
    password.value = chosen
    strength.refresh()
  }, showError)

  const inputs = { title, username, password, url, otp, notes, tags, starred: () => starred, fieldList, expires }
  const collect = () => collectEntry(data, inputs)
  const untouched = JSON.stringify(collect())

  let saving = false
  const save = async () => {
    if (saving) return
    saving = true
    try {
      const saved = await api.saveEntry(id, id ? data : null, collect(), files.changes())
      await entryScreen(saved.id, saved.listing)
      if (saved.conflicts.length) snack(`Also changed on another device: ${saved.conflicts.join(', ')}. That version is in the history.`)
    } catch (e) {
      showError(String(e))
    } finally {
      saving = false
    }
  }
  const leaveEditor = () => {
    files.release()
    back()
  }
  const close = () => {
    if (JSON.stringify(collect()) === untouched && !files.changes().length) leaveEditor()
    else confirmSheet('Discard the changes?', 'Discard', leaveEditor)
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
      el('h2', {}, 'Files'),
      files.element,
      ...(id ? [el('p', { className: 'muted' }, 'Saving keeps the previous version in the history.')] : [])),
  ], onBack)
  if (!id) title.focus()
  // Saving still works: only what was edited here is applied, and where both
  // changed a field the other version goes to the entry's history.
  onSynced = (synced) => {
    if (!id || !synced.changed) return
    void api.editEntry(id).then((now) => {
      if (JSON.stringify(now) !== JSON.stringify(data)) showError('This entry was just changed on another device. Saving keeps your version; the other one goes to the entry’s history.')
    }, () => showError('This entry was just deleted on another device. Saving brings it back.'))
  }
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
    returnedAt = Date.now()
    // Unlocked meanwhile for a passkey (the credential provider shares the session).
    if (!unlocked && database) void api.status().then(async (s) => (s.unlocked ? unlockedWith(await api.listing()) : onReturn()), onReturn)
    else onReturn()
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
