import { listen } from '@tauri-apps/api/event'
import { el, button, busyButton, errorLine, enterPresses } from '../../../../src/dom'
import { formatDateTime, formatSize, splitCode, titleOf } from '../../../../src/entry-text'
import { ALL, GROUPS, sameFilter, search, tagCounts, type Filter } from '../../../../src/search'
import { api, type Entry, type Listing, type Status, type Synced } from './api'

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
/** Stops what the screen shown runs on a timer (the TOTP countdown). */
let leave = () => {}
/** The screen shown's answer to a sync (the list's status line). */
let onSynced = (_synced: Synced) => {}

function show(...children: Node[]) {
  leave()
  leave = () => {}
  onSynced = () => {}
  screen.replaceChildren(...children)
  window.scrollTo(0, 0)
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
  const close = () => shade.remove()
  shade.append(el('div', { className: 'panel' }, ...fill(close)))
  shade.addEventListener('click', (e) => e.target === shade && close())
  document.body.append(shade)
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
  show(
    el('p', { className: 'muted' }, 'First run'),
    el('h1', {}, 'Choose your database'),
    el('p', {}, 'PswManager opens one KeePass (KDBX 4) file. The same file opens in PswManager on Windows and in KeePassXC.'),
    el('button', { type: 'button', className: 'card', disabled: true }, 'Sync with Dropbox', el('small', {}, 'Comes in the next version.')),
    local,
    el('p', { className: 'muted' }, 'A local file is picked with Android’s file picker; it can be anywhere the picker reaches, also a folder another app syncs.'),
    error.line,
  )
}

// ------------------------------------------------------------ unlock

function unlockScreen() {
  const current = database!
  const error = errorLine()
  const password = el('input', { type: 'password', autocomplete: 'off', placeholder: 'Master password', className: 'field' })
  const unlock = busyButton('Unlock', 'Unlock the database', async () => {
    syncLine = 'Syncing…'
    const opened = await api.unlock(password.value)
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
  show(
    el('h1', {}, current.title),
    el('p', { className: 'muted' }, current.description),
    el('p', { className: 'muted' }, current.syncedWith ? `Syncs with ${current.syncedWith}` : ''),
    password,
    unlock,
    error.line,
    button('Use another database…', 'Forget this one (its file stays where it is)', forget, 'link'),
  )
  password.focus()
}

// ------------------------------------------------------------ the list

function listScreen(opened: Listing) {
  let listing = opened
  const searchField = el('input', { type: 'search', className: 'field search', value: query })
  const status = el('p', { className: 'status' }, syncLine)
  const list = el('ul', { className: 'entries' })
  const fill = () => {
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
  const menu = button('☰', 'Groups and tags', () => drawer(listing, fill), 'icon')
  show(el('header', { className: 'bar' }, menu, searchField), status, list)
  fill()
  onSynced = (synced) => {
    status.textContent = synced.text
    status.classList.toggle('problem', synced.problem)
    if (synced.changed) void api.listing().then((fresh) => ((listing = fresh), fill()))
  }
}

function applySync(synced: Synced) {
  syncLine = synced.text
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
  sheet((close) => [el('b', {}, titleOf(entry)), ...choices.map(([label, copy]) => button(label, label, () => void copied(copy()).then(close)))])
}

/** The status line's sheet: what the last sync did, and Sync now. */
function syncSheet() {
  sheet((close) => [
    el('b', {}, 'Sync'),
    el('p', {}, syncLine || 'Not synced yet'),
    el('p', { className: 'muted' }, database?.syncedWith ? `With ${database.syncedWith}` : ''),
    button('Sync now', 'Sync now', () => {
      close()
      void api.syncNow()
    }, 'primary'),
  ])
}

function drawer(listing: Listing, changed: () => void) {
  const panel = el('nav', { className: 'drawer' })
  const shade = el('div', { className: 'shade' }, panel)
  const close = () => shade.remove()
  shade.addEventListener('click', (e) => e.target === shade && close())
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
  document.body.append(shade)
}

async function lock() {
  document.querySelectorAll('.shade, .sheet').forEach((n) => n.remove())
  query = ''
  await api.lock()
  unlockScreen()
}

// ------------------------------------------------------------ an entry

/** A labelled value; with `copy`, a Copy button, and tapping the value copies too. */
type Line = (label: string, value: Node | string, copy?: () => Promise<number>, ...more: Node[]) => HTMLElement

const line: Line = (label, value, copy, ...more) => {
  const shown = el('span', {}, el('small', {}, label), typeof value === 'string' ? el('span', {}, value) : value)
  const copyButton = copy ? [button('Copy', `Copy ${label.toLowerCase()}`, () => void copied(copy()))] : []
  if (copy) shown.addEventListener('click', (e) => (e.target as HTMLElement).closest('button') || void copied(copy()))
  return el('div', { className: 'line' }, shown, ...more, ...copyButton)
}

async function entryScreen(id: string, listing: Listing) {
  const entry = await api.entry(id)
  const rows: Node[] = []
  const stops: (() => void)[] = []
  if (entry.username) rows.push(line('User name', entry.username, () => api.copyField(id, 'UserName')))
  if (entry.hasPassword) rows.push(line('Password', secret(id, 'Password'), () => api.copyField(id, 'Password')))
  if (entry.otp) {
    const [node, stop] = totpLine(id)
    rows.push(node)
    stops.push(stop)
  }
  if (entry.url) {
    const open = button('Open', 'Open in the browser', () => void api.openUrl(id).catch((e) => snack(String(e))))
    rows.push(line('URL', entry.url, () => api.copyField(id, 'URL'), open))
  }
  for (const field of entry.fields) {
    rows.push(line(field.name, field.protected ? secret(id, field.name) : (field.value ?? ''), () => api.copyField(id, field.name)))
  }
  if (entry.notes) rows.push(el('div', { className: 'line notes' }, el('span', {}, el('small', {}, 'Notes'), el('span', {}, entry.notes))))
  if (entry.attachments.length) {
    rows.push(el('h2', {}, 'Attachments'), ...entry.attachments.map((a) => el('div', { className: 'line' }, el('span', {}, a.name, el('small', {}, formatSize(a.size))))))
  }
  if (entry.modified) rows.push(el('p', { className: 'muted' }, `Changed ${formatDateTime(entry.modified)}`))
  const back = button('←', 'Back to the list', () => listScreen(listing), 'icon')
  show(el('header', { className: 'bar' }, back, icon(entry, listing), el('h1', {}, titleOf(entry))), ...rows)
  leave = () => stops.forEach((stop) => stop())
}

/** A masked value with a button to show it. */
function secret(id: string, field: string) {
  const mask = '••••••••••••'
  const value = el('span', { className: 'masked' }, mask)
  let shown = false
  const toggle = button('Show', 'Show or hide', async () => {
    shown = !shown
    value.textContent = shown ? await api.reveal(id, field) : mask
    value.classList.toggle('masked', !shown)
    toggle.textContent = shown ? 'Hide' : 'Show'
  }, 'link')
  return el('span', {}, value, ' ', toggle)
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

void start()
