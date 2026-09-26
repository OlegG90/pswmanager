import { listen } from '@tauri-apps/api/event'
import { api, PASSWORD, URL_FIELD, USERNAME, type Entry, type EntryDetail, type Listing, type Status } from './api'
import { actionFor, type Action } from './keys'
import { filterChoices, groupPath, search, type Filter } from './search'

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T

const unlockForm = $<HTMLFormElement>('unlock')
const passwordInput = $<HTMLInputElement>('password')
const unlockError = $('unlock-error')
const vault = $('vault')
const searchInput = $<HTMLInputElement>('search')
const filterSelect = $<HTMLSelectElement>('filter')
const list = $<HTMLUListElement>('list')
const detail = $('detail')
const toast = $('toast')

const DEFAULT_ICON =
  'data:image/svg+xml,' +
  encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="2" y="2" width="20" height="20" rx="5" fill="#8a94a3"/>` +
      `<circle cx="9" cy="12" r="3.2" fill="none" stroke="#fff" stroke-width="2"/><path d="M12 12h7m-2 0v3" stroke="#fff" stroke-width="2" fill="none" stroke-linecap="round"/></svg>`,
  )

let listing: Listing = { entries: [], customIcons: {} }
/** Site icons by host: a data URL, null when the cache has none (yet). */
const siteIcons = new Map<string, string | null>()
let shown: Entry[] = []
let selectedId: string | null = null
let current: EntryDetail | null = null
/** Values revealed in the current entry, by field name. */
const revealed = new Map<string, string>()

// ---------------------------------------------------------------- unlock

function showStatus(status: Status) {
  $('database-path').textContent = status.database ?? 'No database chosen'
  $('database-path').classList.toggle('muted', !status.database)
  $('key-file-path').textContent = status.keyFile ?? 'No key file'
  $('clear-key-file').hidden = !status.keyFile
}

function showUnlock(status: Status) {
  showStatus(status)
  vault.hidden = true
  unlockForm.hidden = false
  passwordInput.focus()
}

async function run(action: () => Promise<void>) {
  unlockError.hidden = true
  try {
    await action()
  } catch (e) {
    unlockError.textContent = String(e)
    unlockError.hidden = false
  }
}

$('pick-database').addEventListener('click', () => run(async () => showStatus(await api.pickDatabase())))
$('pick-key-file').addEventListener('click', () => run(async () => showStatus(await api.pickKeyFile())))
$('clear-key-file').addEventListener('click', () => run(async () => showStatus(await api.clearKeyFile())))

unlockForm.addEventListener('submit', (e) => {
  e.preventDefault()
  const button = $<HTMLButtonElement>('unlock-button')
  button.disabled = true
  run(async () => {
    const password = passwordInput.value
    passwordInput.value = ''
    showVault(await api.unlock(password))
  }).finally(() => {
    button.disabled = false
    if (!unlockForm.hidden) passwordInput.focus()
  })
})

// ---------------------------------------------------------------- vault

function showVault(next: Listing) {
  listing = next
  unlockForm.hidden = true
  vault.hidden = false
  searchInput.value = ''
  fillFilter()
  refresh()
  searchInput.focus()
}

async function lock() {
  await api.lock()
  listing = { entries: [], customIcons: {} }
  shown = []
  selectedId = null
  current = null
  revealed.clear()
  list.replaceChildren()
  detail.replaceChildren()
  showUnlock(await api.status())
}

function fillFilter() {
  const { groups, tags } = filterChoices(listing.entries)
  const option = (value: string, label: string) => new Option(label, value)
  filterSelect.replaceChildren(option('all', 'All entries'))
  if (groups.length) {
    const g = document.createElement('optgroup')
    g.label = 'Groups'
    g.append(...groups.map((path) => option(`group:${path}`, path)))
    filterSelect.append(g)
  }
  if (tags.length) {
    const t = document.createElement('optgroup')
    t.label = 'Tags'
    t.append(...tags.map((tag) => option(`tag:${tag}`, tag)))
    filterSelect.append(t)
  }
}

function currentFilter(): Filter {
  const value = filterSelect.value
  if (value.startsWith('group:')) return { kind: 'group', path: value.slice(6) }
  if (value.startsWith('tag:')) return { kind: 'tag', tag: value.slice(4) }
  return { kind: 'all' }
}

/** Re-runs the search and redraws the list, keeping the selection if it is still shown. */
function refresh() {
  shown = search(listing.entries, searchInput.value, currentFilter())
  list.replaceChildren(...shown.map(listItem))
  if (shown.length === 0) {
    const empty = document.createElement('li')
    empty.className = 'empty'
    empty.textContent = listing.entries.length ? 'Nothing found' : 'The database is empty'
    list.append(empty)
  }
  select(shown.some((e) => e.id === selectedId) ? selectedId : (shown[0]?.id ?? null))
}

function iconFor(entry: Entry): string {
  if (entry.customIcon && listing.customIcons[entry.customIcon]) return listing.customIcons[entry.customIcon]
  if (entry.host) {
    if (!siteIcons.has(entry.host)) loadSiteIcon(entry.host)
    const icon = siteIcons.get(entry.host)
    if (icon) return icon
  }
  return DEFAULT_ICON
}

function iconImage(entry: Entry): HTMLImageElement {
  const img = document.createElement('img')
  img.className = 'icon'
  img.alt = ''
  setIcon(img, iconFor(entry))
  if (entry.host && !entry.customIcon) img.dataset.host = entry.host
  img.onerror = () => {
    img.onerror = null
    setIcon(img, DEFAULT_ICON)
  }
  return img
}

function setIcon(img: HTMLImageElement, src: string) {
  img.src = src
  img.classList.toggle('site', src !== DEFAULT_ICON)
}

async function loadSiteIcon(host: string) {
  siteIcons.set(host, null)
  const icon = await api.icon(host).catch(() => null)
  if (!icon) return
  siteIcons.set(host, icon)
  document.querySelectorAll<HTMLImageElement>('img[data-host]').forEach((img) => {
    if (img.dataset.host === host) setIcon(img, icon)
  })
}

function listItem(entry: Entry): HTMLLIElement {
  const li = document.createElement('li')
  li.role = 'option'
  li.dataset.id = entry.id
  const title = document.createElement('div')
  title.className = 'title'
  title.textContent = entry.title || '(no title)'
  const subtitle = document.createElement('div')
  subtitle.className = 'subtitle'
  subtitle.textContent = entry.username || entry.host || groupPath(entry)
  li.append(iconImage(entry), title, subtitle)
  li.addEventListener('mousedown', () => select(entry.id))
  return li
}

function select(id: string | null) {
  if (id !== selectedId) revealed.clear()
  selectedId = id
  for (const li of list.children as HTMLCollectionOf<HTMLLIElement>) {
    const on = li.dataset.id === id
    li.setAttribute('aria-selected', String(on))
    if (on) li.scrollIntoView({ block: 'nearest' })
  }
  if (!id) {
    current = null
    detail.replaceChildren()
    return
  }
  api
    .entry(id)
    .then((entry) => {
      if (selectedId !== id) return
      current = entry
      renderDetail()
    })
    .catch((e) => notify(String(e)))
}

function move(step: number) {
  if (shown.length === 0) return
  const i = shown.findIndex((e) => e.id === selectedId)
  const next = Math.min(shown.length - 1, Math.max(0, i + step))
  select(shown[next].id)
}

// ---------------------------------------------------------------- detail

function button(label: string, title: string, onClick: () => void): HTMLButtonElement {
  const b = document.createElement('button')
  b.type = 'button'
  b.textContent = label
  b.title = title
  b.addEventListener('click', onClick)
  return b
}

function row(label: string, value: Node | string, ...actions: HTMLButtonElement[]): HTMLDivElement {
  const div = document.createElement('div')
  div.className = 'row'
  const l = document.createElement('span')
  l.className = 'label'
  l.textContent = label
  const v = document.createElement('span')
  v.className = 'value'
  v.append(value)
  const a = document.createElement('span')
  a.className = 'actions'
  a.append(...actions)
  div.append(l, v, a)
  return div
}

/** A protected value: masked until revealed. `keys` names the password's shortcuts. */
function secretRow(label: string, field: string, keys?: { reveal: string; copy: string }): HTMLDivElement {
  const value = revealed.get(field)
  const hint = (key?: string) => (key ? ` (${key})` : '')
  const div = row(
    label,
    value ?? '••••••••',
    button(value === undefined ? 'Show' : 'Hide', `Show / hide${hint(keys?.reveal)}`, () => toggleReveal(field)),
    button('Copy', `Copy${hint(keys?.copy)}`, () => copy(field, label)),
  )
  div.querySelector('.value')!.classList.add('secret')
  return div
}

function renderDetail() {
  const entry = current
  if (!entry) return
  const heading = document.createElement('h2')
  const title = document.createElement('span')
  title.textContent = entry.title || '(no title)'
  heading.append(iconImage(entry), title)

  const rows: Node[] = [heading]
  if (entry.username) {
    rows.push(row('User name', entry.username, button('Copy', 'Copy (Ctrl+B)', () => copy(USERNAME, 'User name'))))
  }
  if (entry.hasPassword) rows.push(secretRow('Password', PASSWORD, { reveal: 'Ctrl+H', copy: 'Ctrl+C' }))
  if (entry.url) {
    const actions = [button('Copy', 'Copy', () => copy(URL_FIELD, 'URL'))]
    if (entry.host) actions.unshift(button('Open', 'Open in the browser (Ctrl+U)', openUrl))
    rows.push(row('URL', entry.url, ...actions))
  }
  for (const field of entry.fields) {
    if (field.protected) {
      rows.push(secretRow(field.name, field.name))
    } else if (field.value) {
      rows.push(row(field.name, field.value, button('Copy', 'Copy', () => copy(field.name, field.name))))
    }
  }
  if (entry.notes) {
    const notes = document.createElement('p')
    notes.className = 'notes'
    notes.textContent = entry.notes
    rows.push(notes)
  }
  const meta = [groupPath(entry), entry.tags.join(', ')].filter(Boolean).join(' · ')
  if (meta) {
    const p = document.createElement('p')
    p.className = 'meta'
    p.textContent = meta
    rows.push(p)
  }
  detail.replaceChildren(...rows)
}

async function toggleReveal(field: string) {
  if (!current) return
  if (revealed.has(field)) {
    revealed.delete(field)
  } else {
    const id = current.id
    const value = await api.reveal(id, field).catch((e) => (notify(String(e)), null))
    if (value === null || current?.id !== id) return
    revealed.set(field, value)
  }
  renderDetail()
}

async function copy(field: string, label: string) {
  if (!current) return
  try {
    const seconds = await api.copy(current.id, field)
    notify(`${label} copied · clears in ${seconds} s`)
  } catch (e) {
    notify(String(e))
  }
}

function openUrl() {
  if (current?.host) api.openUrl(current.id).catch((e) => notify(String(e)))
}

let toastTimer: number | undefined
function notify(message: string) {
  toast.textContent = message
  toast.hidden = false
  clearTimeout(toastTimer)
  toastTimer = window.setTimeout(() => (toast.hidden = true), 3000)
}

// ---------------------------------------------------------------- keys

searchInput.addEventListener('input', refresh)
filterSelect.addEventListener('change', refresh)
$('lock-button').addEventListener('click', lock)

function perform(action: Action, e: KeyboardEvent) {
  switch (action) {
    case 'copy-username':
      if (current?.username) copy(USERNAME, 'User name')
      break
    case 'copy-password':
      if (current?.hasPassword) copy(PASSWORD, 'Password')
      break
    case 'open-url':
      openUrl()
      break
    case 'toggle-password':
      if (current?.hasPassword) toggleReveal(PASSWORD)
      break
    case 'lock':
      lock()
      break
    case 'previous':
      move(-1)
      break
    case 'next':
      move(1)
      break
    case 'escape':
      if (searchInput.value) {
        searchInput.value = ''
        refresh()
      }
      searchInput.focus()
      break
    case 'type-to-search':
      searchInput.focus()
      return // the key itself goes on into the search field
  }
  e.preventDefault()
}

document.addEventListener('keydown', (e) => {
  if (vault.hidden) return
  const target = e.target as HTMLElement
  const inTextField = target instanceof HTMLInputElement && target.type !== 'button'
  const fieldSelection = target instanceof HTMLInputElement && target.selectionStart !== target.selectionEnd
  const hasSelection = fieldSelection || !!document.getSelection()?.toString()
  const action = actionFor(e, { inTextField, hasSelection })
  if (action) perform(action, e)
})

// ---------------------------------------------------------------- start

listen<string>('icon-ready', (e) => {
  siteIcons.delete(e.payload)
  loadSiteIcon(e.payload)
})

api.status().then(async (status) => {
  if (status.unlocked) showVault(await api.listing())
  else showUnlock(status)
})
