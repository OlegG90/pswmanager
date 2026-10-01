import { listen } from '@tauri-apps/api/event'
import { el, button, busyButton, errorLine, enterPresses } from '../../../../src/dom'
import { formatDateTime, formatSize } from '../../../../src/entry-text'
import { ALL, GROUPS, sameFilter, search, tagCounts, type Filter } from '../../../../src/search'
import { api, type Entry, type Listing, type Status, type Synced } from './api'

const screen = document.querySelector<HTMLElement>('#screen')!
const snackbar = document.querySelector<HTMLElement>('#snackbar')!

/** The groups the phone shows; Templates and Trash come later. */
const PHONE_GROUPS = GROUPS.filter((g) => g.group !== 'templates' && g.group !== 'trash')

let listing: Listing | null = null
let filter: Filter = ALL
let query = ''
let syncLine = ''
/** Stops what the screen shown runs on a timer (the TOTP countdown). */
let leave = () => {}

function show(...children: Node[]) {
  leave()
  leave = () => {}
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

async function start() {
  const status = await api.status()
  if (!status.database) chooseScreen()
  else if (status.unlocked) await listScreen(await api.listing())
  else unlockScreen(status)
}

// ------------------------------------------------------------ first run

function chooseScreen() {
  const error = errorLine()
  const local = busyButton('Open a local file', 'A .kdbx on this phone or an SD card', async () => {
    const status = await api.openLocalFile()
    if (status) unlockScreen(status)
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

function unlockScreen(status: Status) {
  const database = status.database!
  const error = errorLine()
  const password = el('input', { type: 'password', autocomplete: 'off', placeholder: 'Master password', className: 'field' })
  const unlock = busyButton('Unlock', 'Unlock the database', async () => {
    const opened = await api.unlock(password.value)
    password.value = ''
    syncLine = 'Syncing…'
    await listScreen(opened)
  }, error.show, 'primary')
  enterPresses(unlock, password)
  error.hideOnInput(password)
  const another = button('Use another database…', 'Forget this one (its file stays where it is)', async () => {
    chooseScreen()
    await api.forgetDatabase()
  }, 'link')
  show(
    el('h1', {}, database.title),
    el('p', { className: 'muted' }, database.description),
    el('p', { className: 'muted' }, database.syncedWith ? `Syncs with ${database.syncedWith}` : ''),
    password,
    unlock,
    error.line,
    another,
  )
  password.focus()
}

// ------------------------------------------------------------ the list

async function listScreen(opened: Listing) {
  listing = opened
  const searchField = el('input', { type: 'search', placeholder: '', className: 'field search', value: query })
  const status = el('p', { className: 'status' }, syncLine)
  const list = el('ul', { className: 'entries' })
  const fill = () => {
    searchField.placeholder = `Search ${filterLabel(filter)}`
    const shown = search(listing!.entries, query, filter)
    list.replaceChildren(...shown.map(row))
    if (shown.length === 0) list.append(el('li', { className: 'muted empty' }, 'Nothing here.'))
  }
  searchField.addEventListener('input', () => {
    query = searchField.value
    fill()
  })
  status.addEventListener('click', () => api.syncNow())
  const menu = button('☰', 'Groups and tags', () => drawer(() => fill()), 'icon')
  show(el('header', { className: 'bar' }, menu, searchField), status, list)
  fill()
  onSynced = (synced) => {
    status.textContent = synced.text
    status.classList.toggle('problem', synced.problem)
    if (synced.changed) void api.listing().then((fresh) => ((listing = fresh), fill()))
  }
}

function filterLabel(f: Filter) {
  return f.kind === 'tag' ? f.tag : PHONE_GROUPS.find((g) => g.group === f.group)!.label
}

function row(entry: Entry): HTMLLIElement {
  const item = el('li', { tabIndex: 0 }, icon(entry), el('span', {}, el('b', {}, entry.title || '(no title)'), el('small', {}, entry.username)))
  item.addEventListener('click', () => void entryScreen(entry.id))
  item.addEventListener('contextmenu', (e) => {
    e.preventDefault()
    quickCopy(entry)
  })
  return item
}

function icon(entry: Entry) {
  const custom = entry.customIcon && listing?.customIcons[entry.customIcon]
  if (custom) return el('img', { className: 'icon', src: custom, alt: '' })
  return el('span', { className: 'icon letter' }, (entry.title || '?').slice(0, 1).toUpperCase())
}

/** A long press: copy without opening the entry. */
function quickCopy(entry: Entry) {
  const choices: [string, () => Promise<number>][] = [
    ['Copy user name', () => api.copyField(entry.id, 'UserName')],
    ['Copy password', () => api.copyField(entry.id, 'Password')],
  ]
  if (entry.otp) choices.push(['Copy TOTP code', () => api.copyTotp(entry.id)])
  const sheet = el('div', { className: 'sheet' })
  const close = () => sheet.remove()
  sheet.append(
    el('div', { className: 'panel' }, el('b', {}, entry.title), ...choices.map(([label, copy]) => button(label, label, () => void copied(copy()).then(close)))),
  )
  sheet.addEventListener('click', (e) => e.target === sheet && close())
  document.body.append(sheet)
}

async function copied(copy: Promise<number>, what = 'Copied') {
  try {
    snack(`${what} · clears in ${await copy} s`)
  } catch (e) {
    snack(String(e))
  }
}

function drawer(changed: () => void) {
  const panel = el('nav', { className: 'drawer' })
  const shade = el('div', { className: 'shade' }, panel)
  const close = () => shade.remove()
  shade.addEventListener('click', (e) => e.target === shade && close())
  const choose = (f: Filter) => () => {
    filter = f
    close()
    changed()
  }
  const item = (f: Filter, label: string, count: number) =>
    el('button', { type: 'button', className: sameFilter(f, filter) ? 'chosen' : '', onclick: choose(f) }, el('span', {}, label), el('small', {}, String(count)))
  const entries = listing!.entries
  panel.append(
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
  listing = null
  query = ''
  await api.lock()
  unlockScreen(await api.status())
}

// ------------------------------------------------------------ an entry

async function entryScreen(id: string) {
  const entry = await api.entry(id)
  const rows: Node[] = []
  const stops: (() => void)[] = []
  const line = (label: string, value: Node | string, copy?: () => Promise<number>) =>
    el('div', { className: 'line' }, el('span', {}, el('small', {}, label), typeof value === 'string' ? el('span', {}, value) : value), ...(copy ? [button('Copy', `Copy ${label.toLowerCase()}`, () => void copied(copy()))] : []))
  if (entry.username) rows.push(line('User name', entry.username, () => api.copyField(id, 'UserName')))
  if (entry.hasPassword) rows.push(line('Password', secret(id, 'Password'), () => api.copyField(id, 'Password')))
  if (entry.otp) {
    const [node, stop] = totpLine(id, line)
    rows.push(node)
    stops.push(stop)
  }
  if (entry.url) rows.push(line('URL', entry.url, () => api.copyField(id, 'URL')))
  for (const field of entry.fields) {
    rows.push(line(field.name, field.protected ? secret(id, field.name) : (field.value ?? ''), () => api.copyField(id, field.name)))
  }
  if (entry.notes) rows.push(el('div', { className: 'line notes' }, el('span', {}, el('small', {}, 'Notes'), el('span', {}, entry.notes))))
  if (entry.attachments.length) {
    rows.push(el('h2', {}, 'Attachments'), ...entry.attachments.map((a) => el('div', { className: 'line' }, el('span', {}, a.name, el('small', {}, formatSize(a.size))))))
  }
  if (entry.modified) rows.push(el('p', { className: 'muted' }, `Changed ${formatDateTime(entry.modified)}`))
  const back = button('←', 'Back to the list', () => void listScreen(listing!), 'icon')
  show(el('header', { className: 'bar' }, back, icon(entry), el('h1', {}, entry.title || '(no title)')), ...rows)
  leave = () => stops.forEach((stop) => stop())
}

/** A masked value with an eye to reveal it. */
function secret(id: string, field: string) {
  const value = el('span', { className: 'masked' }, '••••••••••••')
  let shown = false
  const eye = button('Show', 'Show or hide', async () => {
    shown = !shown
    value.textContent = shown ? await api.reveal(id, field) : '••••••••••••'
    value.classList.toggle('masked', !shown)
    eye.textContent = shown ? 'Hide' : 'Show'
  }, 'link')
  return el('span', {}, value, ' ', eye)
}

/** The TOTP code with its countdown, and what stops the countdown. */
function totpLine(id: string, line: (label: string, value: Node | string, copy?: () => Promise<number>) => HTMLElement): [HTMLElement, () => void] {
  const code = el('span', { className: 'code' })
  const left = el('small', {})
  const tick = async () => {
    const now = await api.totp(id)
    if (!now) return
    code.textContent = `${now.code.slice(0, now.code.length / 2)} ${now.code.slice(now.code.length / 2)}`
    left.textContent = ` ${now.remaining} s`
  }
  void tick()
  const timer = window.setInterval(() => void tick(), 1000)
  return [line('TOTP', el('span', {}, code, left), () => api.copyTotp(id)), () => clearInterval(timer)]
}

// ------------------------------------------------------------ sync

let onSynced = (synced: Synced) => {
  syncLine = synced.text
}
void listen<Synced>('synced', (e) => {
  syncLine = e.payload.text
  onSynced(e.payload)
})

void start()
