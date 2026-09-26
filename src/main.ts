import { listen } from '@tauri-apps/api/event'
import { api, OTP, PASSWORD, URL_FIELD, USERNAME, type Entry, type EntryDetail, type Listing, type Saved, type Status } from './api'
import { el } from './dom'
import { closeEditor, editorKey, isEditing, openEditor } from './editor'
import { parseGroup } from './entry-text'
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

const EMPTY: Listing = { entries: [], customIcons: {} }
let listing = EMPTY
/** Site icons by host: a data URL, null when the cache has none (yet). */
const siteIcons = new Map<string, string | null>()
let shown: Entry[] = []
let selectedId: string | null = null
let current: EntryDetail | null = null
/** The current entry is asking whether to move it to the recycle bin. */
let confirmingDelete = false
/** Values revealed in the current entry, by field name. */
const revealed = new Map<string, string>()

// ---------------------------------------------------------------- unlock

function showStatus(status: Status) {
  $('database-path').textContent = status.database ?? 'No database chosen'
  $('database-path').classList.toggle('muted', !status.database)
  $('key-file-path').textContent = status.keyFile ?? 'No key file'
  $('clear-key-file').hidden = !status.keyFile
  $('notice').textContent = status.notice ?? ''
  $('notice').hidden = !status.notice
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

/** Asks the backend to lock; it answers with the `locked` event. */
function lock() {
  api.lock().catch((e) => notify(String(e)))
}

/** Forgets everything shown and returns to the unlock screen. */
async function showLocked() {
  closeEditor()
  stopTotp()
  listing = EMPTY
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
  const section = (label: string, kind: string, values: string[]) =>
    values.length ? [el('optgroup', { label }, ...values.map((v) => new Option(v, `${kind}:${v}`)))] : []
  filterSelect.replaceChildren(
    new Option('All entries', 'all'),
    ...section('Groups', 'group', groups),
    ...section('Tags', 'tag', tags),
  )
}

/** Filter options carry their kind before the first colon: `group:Work`, `tag:Favorite`. */
function currentFilter(): Filter {
  const [kind, ...rest] = filterSelect.value.split(':')
  const value = rest.join(':')
  if (kind === 'group') return { kind, path: value }
  if (kind === 'tag') return { kind, tag: value }
  return { kind: 'all' }
}

/** Re-runs the search and redraws the list, keeping the selection if it is still shown. */
function refresh() {
  shown = search(listing.entries, searchInput.value, currentFilter())
  list.replaceChildren(...shown.map(listItem))
  if (shown.length === 0) {
    list.append(el('li', { className: 'empty' }, listing.entries.length ? 'Nothing found' : 'The database is empty'))
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
  const img = el('img', { className: 'icon', alt: '' })
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
  const li = el(
    'li',
    { role: 'option' },
    iconImage(entry),
    el('div', { className: 'title' }, entry.title || '(no title)'),
    el('div', { className: 'subtitle' }, entry.username || entry.host || groupPath(entry)),
  )
  li.dataset.id = entry.id
  li.addEventListener('mousedown', () => select(entry.id))
  return li
}

function select(id: string | null) {
  if (isEditing()) return // the editor stays until it is saved or cancelled
  const same = id === selectedId
  if (!same) {
    revealed.clear()
    confirmingDelete = false
  }
  selectedId = id
  for (const li of list.children as HTMLCollectionOf<HTMLLIElement>) {
    const on = li.dataset.id === id
    li.setAttribute('aria-selected', String(on))
    if (on) li.scrollIntoView({ block: 'nearest' })
  }
  // A new search that keeps the same entry keeps its view as it is.
  if (same && current?.id === id) return
  stopTotp()
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
  return el('button', { type: 'button', title, onclick: onClick }, label)
}

function row(label: string, value: string, actions: HTMLButtonElement[], valueClass = ''): HTMLDivElement {
  return el(
    'div',
    { className: 'row' },
    el('span', { className: 'label' }, label),
    el('span', { className: `value ${valueClass}` }, value),
    el('span', { className: 'actions' }, ...actions),
  )
}

/** A protected value: masked until revealed. `keys` names the password's shortcuts. */
function secretRow(label: string, field: string, keys?: { reveal: string; copy: string }): HTMLDivElement {
  const value = revealed.get(field)
  const hint = (key?: string) => (key ? ` (${key})` : '')
  const actions = [
    button(value === undefined ? 'Show' : 'Hide', `Show / hide${hint(keys?.reveal)}`, () => toggleReveal(field)),
    button('Copy', `Copy${hint(keys?.copy)}`, () => copy(field, label)),
  ]
  return row(label, value ?? '••••••••', actions, 'secret')
}

/** How a field is labelled; the standard ones only appear here when protected. */
const LABELS: Record<string, string> = { [USERNAME]: 'User name', [URL_FIELD]: 'URL', otp: 'TOTP' }
const labelOf = (field: string) => LABELS[field] ?? field

function renderDetail() {
  const entry = current
  if (!entry) return
  const rows: Node[] = [el('h2', {}, iconImage(entry), el('span', {}, entry.title || '(no title)'))]
  if (entry.username) {
    rows.push(row('User name', entry.username, [button('Copy', 'Copy (Ctrl+B)', () => copy(USERNAME, 'User name'))]))
  }
  if (entry.hasPassword) rows.push(secretRow('Password', PASSWORD, { reveal: 'Ctrl+H', copy: 'Ctrl+C' }))
  if (entry.url) {
    const actions = [button('Copy', 'Copy', () => copy(URL_FIELD, 'URL'))]
    if (entry.host) actions.unshift(button('Open', 'Open in the browser (Ctrl+U)', openUrl))
    rows.push(row('URL', entry.url, actions))
  }
  if (entry.fields.some((f) => f.name === OTP)) rows.push(totpRow(entry.id))
  for (const field of entry.fields.filter((f) => f.name !== OTP)) {
    if (field.protected) {
      rows.push(secretRow(labelOf(field.name), field.name))
    } else if (field.value) {
      rows.push(row(field.name, field.value, [button('Copy', 'Copy', () => copy(field.name, field.name))]))
    }
  }
  if (entry.notes) rows.push(el('p', { className: 'notes' }, entry.notes))
  const meta = [groupPath(entry), entry.tags.join(', ')].filter(Boolean).join(' · ')
  if (meta) rows.push(el('p', { className: 'meta' }, meta))
  const confirm = el('button', { type: 'button', className: 'primary', textContent: 'Move to the recycle bin', onclick: deleteNow })
  rows.push(
    confirmingDelete
      ? el('div', { className: 'buttons confirm' },
          el('span', {}, `Move "${entry.title || '(no title)'}" to the recycle bin?`), confirm,
          button('Cancel', 'Cancel (Esc)', cancelDelete))
      : el('div', { className: 'buttons' },
          button('Edit', 'Edit (Ctrl+E)', editEntry),
          button('Delete', 'Move to the recycle bin (Del)', deleteEntry)),
  )
  detail.replaceChildren(...rows)
  if (confirmingDelete) confirm.focus()
}

// ---------------------------------------------------------------- TOTP

let totpTimer: number | undefined

function stopTotp(timer = totpTimer) {
  if (timer !== totpTimer) return // an older row's timer, already replaced
  clearInterval(totpTimer)
  totpTimer = undefined
}

/** The current code with its countdown; a new code is fetched when it runs out. */
function totpRow(id: string): HTMLDivElement {
  const value = el('span', { className: 'value secret totp' }, '…')
  const div = el('div', { className: 'row' }, el('span', { className: 'label' }, 'TOTP'), value,
    el('span', { className: 'actions' }, button('Copy', 'Copy (Ctrl+T)', copyTotp)))
  let remaining = 0
  const show = (code: string) => value.replaceChildren(code, el('span', { className: 'countdown' }, `${remaining} s`))
  let code = ''
  let fetching = false
  const tick = async () => {
    if (fetching) return
    if (remaining <= 0) {
      fetching = true
      try {
        const next = await api.totp(id)
        if (!next) return stopTotp(timer)
        code = `${next.code.slice(0, next.code.length / 2)} ${next.code.slice(next.code.length / 2)}`
        remaining = next.remaining
      } catch (e) {
        stopTotp(timer)
        value.textContent = String(e)
        return
      } finally {
        fetching = false
      }
    }
    show(code)
    remaining--
  }
  stopTotp()
  const timer = window.setInterval(tick, 1000)
  totpTimer = timer
  tick()
  return div
}

async function copyTotp() {
  if (!current) return
  try {
    const seconds = await api.copyTotp(current.id)
    notify(`TOTP code copied · clears in ${seconds} s`)
  } catch (e) {
    notify(String(e))
  }
}

// ---------------------------------------------------------------- editing

/** New entries go into the group the list is filtered to. */
function groupForNew(): string[] {
  const filter = currentFilter()
  return filter.kind === 'group' ? parseGroup(filter.path) : []
}

/** Set while the editor loads, so a second Ctrl+E does not open another. */
let openingEditor = false

function startEditor(id: string | null) {
  if (isEditing() || openingEditor) return
  openingEditor = true
  stopTotp()
  openEditor(detail, {
    id,
    group: groupForNew(),
    onSaved: afterSave,
    onClose: () => {
      current = null
      select(selectedId)
    },
  })
    .catch((e) => notify(String(e)))
    .finally(() => (openingEditor = false))
}

function editEntry() {
  if (current) startEditor(current.id)
}

function afterSave(saved: Saved) {
  listing = saved.listing
  fillFilter()
  current = null
  selectedId = saved.id
  refresh()
  notify('Saved')
  searchInput.focus()
}

/** Del / the Delete button: asks first, in the entry view. */
function deleteEntry() {
  if (!current || isEditing()) return
  confirmingDelete = true
  renderDetail()
}

function cancelDelete() {
  confirmingDelete = false
  renderDetail()
  searchInput.focus()
}

async function deleteNow() {
  if (!current) return
  confirmingDelete = false
  try {
    listing = await api.deleteEntry(current.id)
    fillFilter()
    current = null
    refresh()
    searchInput.focus()
    notify('Moved to the recycle bin')
  } catch (e) {
    notify(String(e))
  }
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

/** Esc with nothing left to close: back to the tray. */
function hideWindow() {
  api.hideWindow().catch((e) => notify(String(e)))
}

function perform(action: Action, e: KeyboardEvent) {
  switch (action) {
    case 'copy-username':
      // A protected user name is among the fields instead.
      if (current?.username || current?.fields.some((f) => f.name === USERNAME)) copy(USERNAME, 'User name')
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
    case 'copy-totp':
      if (current?.fields.some((f) => f.name === OTP)) copyTotp()
      break
    case 'new-entry':
      startEditor(null)
      break
    case 'edit-entry':
      editEntry()
      break
    case 'delete-entry':
      deleteEntry()
      break
    case 'previous':
      move(-1)
      break
    case 'next':
      move(1)
      break
    case 'escape':
      if (confirmingDelete) {
        cancelDelete()
      } else if (searchInput.value) {
        searchInput.value = ''
        refresh()
        searchInput.focus()
      } else {
        hideWindow()
      }
      break
    case 'type-to-search':
      searchInput.focus()
      return // the key itself goes on into the search field
  }
  e.preventDefault()
}

document.addEventListener('keydown', (e) => {
  if (vault.hidden) {
    if (e.key === 'Escape') hideWindow()
    return
  }
  if (isEditing()) {
    // The form keeps its keys; only save, cancel and lock work on top.
    if (!editorKey(e) && e.ctrlKey && e.code === 'KeyL') lock()
    return
  }
  const target = e.target as HTMLElement
  const inTextField = target instanceof HTMLInputElement && target.type !== 'button'
  const inSelect = target instanceof HTMLSelectElement
  const fieldSelection = target instanceof HTMLInputElement && target.selectionStart !== target.selectionEnd
  const hasSelection = fieldSelection || !!document.getSelection()?.toString()
  const action = actionFor(e, { inTextField, inSelect, hasSelection })
  if (action) perform(action, e)
})

// ---------------------------------------------------------------- start

/** Reports use of the window at most every few seconds; the backend locks
 *  after the configured time without any. */
let lastTouch = 0
function reportActivity() {
  const now = Date.now()
  if (vault.hidden || now - lastTouch < 5000) return
  lastTouch = now
  api.touch().catch(() => {})
}
// Pointer movement alone does not count: hovering over the window is not using it.
for (const type of ['keydown', 'pointerdown', 'wheel']) {
  document.addEventListener(type, reportActivity, { passive: true, capture: true })
}

listen('locked', showLocked)
listen('window-shown', () => (vault.hidden ? passwordInput : searchInput).focus())

listen<string>('icon-ready', (e) => {
  siteIcons.delete(e.payload)
  loadSiteIcon(e.payload)
})

api.status().then(async (status) => {
  if (status.unlocked) showVault(await api.listing())
  else showUnlock(status)
})
