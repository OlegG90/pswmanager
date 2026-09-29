import { DEFAULT_ICON, glyphIcon } from './glyphs'
import { listen } from '@tauri-apps/api/event'
import { getVersion } from '@tauri-apps/api/app'
import { api, OTP, PASSWORD, URL_FIELD, USERNAME, type Attachment, type DatabaseInfo, type EntryData, type Version, type VersionDetail, type DiskChange, type Entry, type EntryDetail, type KeyNeeded, type Listing, type Saved, type Status, type SyncStatus } from './api'
import { button, el } from './dom'
import { changedElsewhere, closeEditor, editorKey, isEditing, openEditor } from './editor'
import { menuButton } from './menu'
import { ask, askText, beforeExtension, choose, isAsking } from './modal'
import { formatDate, formatDateTime, formatSize } from './entry-text'
import { actionFor, type Action } from './keys'
import { ALL, expiry, FAVORITE, GROUPS, sameFilter, search, tagCounts, TEMPLATES, TRASH, type Filter } from './search'
import { renderSettings } from './settings'
import { renderChoose } from './choose'
import { renderHealth } from './health'
import { enterOtherKey } from './other-key'
import { clicked, type Choice } from './selection'

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T

const unlockForm = $<HTMLFormElement>('unlock')
const passwordInput = $<HTMLInputElement>('password')
const unlockError = $('unlock-error')
const vault = $('vault')
const searchInput = $<HTMLInputElement>('search')
const groupsList = $<HTMLUListElement>('groups')
const tagsList = $<HTMLUListElement>('tags')
const list = $<HTMLUListElement>('list')
const detail = $('detail')
const settingsView = $('settings')
const chooseView = $('choose')
const healthView = $('health')
const toast = $('toast')


const EMPTY: Listing = {
  entries: [],
  customIcons: {},
  database: { name: '', description: '', defaultUsername: '', historyMaxItems: -1, historyMaxSize: -1,
    encryption: { cipher: 'aes256', kdf: 'argon2id', iterations: 0, memory: 0, parallelism: 0 } }, // all but the user name: unused here, Settings fetches them
}
let unlocked = false
/** The settings screen is over the vault or the unlock screen, whichever is current. */
let settingsOpen = false
/** The password health report is over the vault. */
let healthOpen = false
/** Back from the choose-database screen to the unlock screen; null on the first run. */
let chooseBack: (() => void) | null = null
let listing = EMPTY
/** Site icons by host: a data URL, null when the cache has none (yet). */
const siteIcons = new Map<string, string | null>()
/** What the sidebar shows: a group or a tag. */
let filter: Filter = ALL
let shown: Entry[] = []
let selectedId: string | null = null
/** Several entries chosen with Ctrl / Shift+click; null while one (or none) is. */
let several: Choice | null = null
let current: EntryDetail | null = null
/** What the right column shows of the current entry: itself, its history, or
 *  one of its older versions (`index` in the history, 0 the newest). */
type View =
  | { kind: 'entry' }
  | { kind: 'history'; versions: Version[] }
  | { kind: 'version'; versions: Version[]; index: number; at: VersionDetail }
const ENTRY_VIEW: View = { kind: 'entry' }
let view: View = ENTRY_VIEW
/** The older version shown, for fetching its values and files. */
const shownVersion = () => (view.kind === 'version' ? view.index : null)
/** Values revealed in the current entry, by field name. */
const revealed = new Map<string, string>()

// ---------------------------------------------------------------- unlock

const SYNC_KINDS = { folder: 'Synced with a folder', dropbox: 'Synced with Dropbox', google: 'Synced with Google Drive', onedrive: 'Synced with OneDrive' }

function showStatus(status: Status) {
  const synced = status.syncedWith
  $('database-kind').textContent = status.syncKind ? SYNC_KINDS[status.syncKind] : 'Local file'
  $('database-path').textContent = synced ?? status.database ?? ''
  $('database-path').title = synced ? `Local file: ${status.database}` : (status.database ?? '')
  // With more than one database, a list picks which one opens.
  const select = $<HTMLSelectElement>('database-select')
  select.hidden = status.databases.length < 2
  const label = (d: DatabaseInfo) => (d.name === d.fileName ? d.name : `${d.name} (${d.fileName})`)
  select.replaceChildren(...status.databases.map((d) => new Option(label(d), d.file, false, d.file === status.database)))
  select.title = status.database ?? ''
  // Its name, as last unlocked (the file name until then), with the file under it.
  const current = status.databases.find((d) => d.file === status.database)
  $('database-name').textContent = current?.name ?? ''
  const description = $('database-description')
  description.textContent = description.title = current?.description ?? ''
  description.hidden = !current?.description
  $('key-file-path').textContent = status.keyFile ?? 'No key file'
  $('clear-key-file').hidden = !status.keyFile
  $('notice').textContent = status.notice ?? ''
  $('notice').hidden = !status.notice
}

/** The unlock screen, or the choose-database screen while there is no database. */
function showUnlock(status: Status) {
  vault.hidden = true
  if (!status.database) return showChoose(status)
  showStatus(status)
  chooseBack = null
  chooseView.hidden = true
  chooseView.replaceChildren()
  unlockForm.hidden = settingsOpen
  if (!settingsOpen) passwordInput.focus()
}

function showChoose(status: Status) {
  const back = () => run(async () => showUnlock(await api.status()))
  chooseBack = status.database ? back : null
  unlockForm.hidden = true
  chooseView.hidden = settingsOpen
  renderChoose(chooseView, status, { chosen: showUnlock, back })
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

$('change-database').addEventListener('click', () => run(async () => showChoose(await api.status())))
$('database-select').addEventListener('change', async (e) => {
  await run(async () => {
    showStatus(await api.selectDatabase((e.target as HTMLSelectElement).value))
    passwordInput.focus()
  })
  // A refused switch: the list shows again what opens.
  if (!unlockError.hidden) showStatus(await api.status())
})
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
  unlocked = true
  unlockForm.hidden = true
  vault.hidden = false
  keyDialogShown = false
  api.keyNeeded().then(showKeyNeeded, () => {})
  api.syncStatus().then(showSyncStatus, () => {})
  searchInput.value = ''
  filter = ALL
  fillSidebar()
  refresh()
  searchInput.focus()
}

/** Asks the backend to lock; it answers with the `locked` event. */
function lock() {
  api.lock().catch((e) => notify(String(e)))
}

/** Forgets everything shown and returns to the unlock screen. */
async function showLocked() {
  unlocked = false
  closeHealth()
  closeEditor()
  stopTotp()
  listing = EMPTY
  shown = []
  selectedId = null
  several = null
  current = null
  revealed.clear()
  list.replaceChildren()
  detail.replaceChildren()
  showUnlock(await api.status())
}

/** The sidebar: the fixed groups, then the tags with their counts. A tag no
 *  entry has any more goes back to All. */
function fillSidebar() {
  const tags = tagCounts(listing.entries)
  const chosen = filter
  if (chosen.kind === 'tag' && !tags.some(([tag]) => tag === chosen.tag)) filter = ALL
  const item = (choice: Filter, label: string, count?: number) => {
    const b = el('button', { type: 'button', className: 'side-item', title: label }, el('span', { className: 'name' }, label))
    if (count !== undefined) b.append(el('span', { className: 'count' }, String(count)))
    if (sameFilter(choice, filter)) b.setAttribute('aria-current', 'true')
    b.addEventListener('click', () => showFilter(choice))
    return el('li', {}, b)
  }
  groupsList.replaceChildren(...GROUPS.map(({ group, label }) => item({ kind: 'group', group }, label)))
  tagsList.replaceChildren(
    ...tags.map(([tag, count]) => {
      const li = item({ kind: 'tag', tag }, tag, count)
      li.append(menuButton(`More for the tag ${tag}`, [
        { label: 'Rename…', title: 'Rename in every entry (to a name another tag has: merge them)', action: () => renameTag(tag) },
        { label: 'Remove…', title: 'Take the tag off every entry; the entries stay', action: () => removeTag(tag), danger: true },
      ]))
      return li
    }),
  )
  $('tags-heading').hidden = tags.length === 0
}

function showFilter(choice: Filter) {
  filter = choice
  fillSidebar()
  refresh()
  searchInput.focus()
}

/** Re-runs the search and redraws the list, keeping the selection if it is still shown. */
function refresh() {
  shown = search(listing.entries, searchInput.value, filter)
  list.replaceChildren(...shown.map(listItem))
  $('entry-count').textContent = count(shown.length)
  $('empty-trash').hidden = !sameFilter(filter, TRASH) || !listing.entries.some(inTrash)
  const newLabel = sameFilter(filter, TEMPLATES) ? 'New template' : 'New entry'
  $('new-entry').textContent = newLabel
  $('new-entry').title = `${newLabel} (Ctrl+N)`
  if (shown.length === 0) {
    const empty = searchInput.value ? 'Nothing found' : sameFilter(filter, ALL) ? 'The database is empty' : 'No entries here'
    list.append(el('li', { className: 'empty' }, empty))
  }
  // Several chosen: those still shown stay chosen.
  if (several) {
    const still = several.chosen.filter((id) => shown.some((e) => e.id === id))
    if (still.length > 1) return chooseSeveral({ ...several, chosen: still })
    several = null
    if (still.length) return select(still[0])
  }
  select(shown.some((e) => e.id === selectedId) ? selectedId : (shown[0]?.id ?? null))
}

/** An entry's own image, else the drawing chosen for it, else its site's icon, else the key. */
function iconFor(entry: Entry): string {
  if (entry.customIcon && listing.customIcons[entry.customIcon]) return listing.customIcons[entry.customIcon]
  if (entry.icon !== null) return glyphIcon(entry.icon)
  return autoIcon(entry)
}

/** What the Auto choice shows: the site's icon, else the key. */
function autoIcon(entry: Entry | null): string {
  if (entry?.host) {
    if (!siteIcons.has(entry.host)) loadSiteIcon(entry.host)
    const icon = siteIcons.get(entry.host)
    if (icon) return icon
  }
  return DEFAULT_ICON
}

function iconImage(entry: Entry): HTMLImageElement {
  const img = el('img', { className: 'icon', alt: '' })
  setIcon(img, iconFor(entry))
  if (entry.host && !entry.customIcon && entry.icon === null) img.dataset.host = entry.host
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
  const title = el('div', { className: 'title' }, entry.title || '(no title)')
  const expires = editable(entry) ? expiry(entry, Date.now()) : null
  if (expires) title.append(el('span', { className: `badge ${expires}` }, expires === 'soon' ? 'Expires soon' : 'Expired'))
  const li = el(
    'li',
    { role: 'option' },
    iconImage(entry),
    title,
    el('div', { className: 'subtitle' }, entry.username || entry.host || ''),
  )
  if (editable(entry)) li.append(starButton(entry))
  li.dataset.id = entry.id
  li.addEventListener('mousedown', (e) => pick(entry.id, e))
  return li
}

/** A click in the list: one entry, or several with Ctrl / Shift. */
function pick(id: string, e: MouseEvent) {
  if (isEditing()) return
  if (!e.ctrlKey && !e.shiftKey) {
    several = null
    return select(id)
  }
  e.preventDefault() // Shift+click would select the page's text
  const now = several ?? (selectedId ? { chosen: [selectedId], anchor: selectedId } : null)
  const next = clicked(shown.map((s) => s.id), now, id, { ctrl: e.ctrlKey, shift: e.shiftKey })
  if (next.chosen.length > 1) return chooseSeveral(next)
  several = null
  select(next.chosen[0])
}

/** Several entries chosen: the list marks them, the right column offers what
 *  can be done to all of them. */
function chooseSeveral(choice: Choice) {
  several = choice
  selectedId = null
  current = null
  revealed.clear()
  stopTotp()
  markList()
  renderSeveral()
}

/** Marks the chosen entries in the list. */
function markList() {
  for (const li of list.children as HTMLCollectionOf<HTMLLIElement>) {
    const id = li.dataset.id ?? ''
    li.setAttribute('aria-selected', String(several ? several.chosen.includes(id) : id === selectedId))
  }
}

const isFavorite = (entry: Entry) => entry.tags.includes(FAVORITE)

/** The star: in the list and the entry view, it stars the entry or takes the
 *  star off, as a tag (no sync until the next sync moment). */
function starButton(entry: Entry): HTMLButtonElement {
  const on = isFavorite(entry)
  const star = button(on ? '★' : '☆', on ? 'Remove from Favorites' : 'Add to Favorites', () => toggleFavorite(entry), 'ghost star')
  star.setAttribute('aria-pressed', String(on))
  // Clicking the star in the list does not select the entry.
  star.addEventListener('mousedown', (e) => e.stopPropagation())
  return star
}

async function toggleFavorite(entry: Entry) {
  if (isEditing()) return
  const on = !isFavorite(entry)
  try {
    applyListing(await api.setTag([entry.id], FAVORITE, on), on ? 'Added to Favorites' : 'Removed from Favorites', false)
  } catch (e) {
    notify(String(e))
  }
}

function select(id: string | null) {
  if (isEditing()) return // the editor stays until it is saved or cancelled
  const same = id === selectedId
  if (!same) revealed.clear()
  selectedId = id
  several = null
  markList()
  list.querySelector(`[aria-selected='true']`)?.scrollIntoView({ block: 'nearest' })
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
      view = ENTRY_VIEW
      renderDetail()
    })
    .catch((e) => notify(String(e)))
}

function move(step: number) {
  if (shown.length === 0) return
  // From several chosen, the arrows go on from the last one clicked.
  const i = shown.findIndex((e) => e.id === (several?.anchor ?? selectedId))
  const next = Math.min(shown.length - 1, Math.max(0, i + step))
  select(shown[next].id)
}

// ---------------------------------------------------------------- detail

/** A field's row. Clicking its value does what its Copy button does;
 *  `copyKey` is the Copy shortcut, for the button's tooltip. */
function row(
  label: string,
  value: string,
  copyValue: () => void,
  { actions = [], copyKey, valueClass = '' }: { actions?: HTMLButtonElement[]; copyKey?: string; valueClass?: string } = {},
): HTMLDivElement {
  return el(
    'div',
    { className: 'row' },
    el('span', { className: 'label' }, label),
    copyOnClick(el('span', { className: `value ${valueClass}` }, value), copyValue),
    el('span', { className: 'actions' }, ...actions, button('Copy', copyKey ? `Copy (${copyKey})` : 'Copy', copyValue)),
  )
}

/** Makes clicking `value` copy it — unless the click ended selecting part of
 *  its text, which stays a plain selection. */
function copyOnClick<T extends HTMLElement>(value: T, copyValue: () => void): T {
  value.classList.add('copyable')
  value.title = 'Click to copy'
  value.addEventListener('click', () => {
    const selection = document.getSelection()
    if (selection && !selection.isCollapsed && value.contains(selection.anchorNode)) return
    copyValue()
  })
  return value
}

/** A protected value: masked until revealed. `keys` names the password's shortcuts. */
function secretRow(label: string, field: string, keys?: { reveal: string; copy: string }): HTMLDivElement {
  const value = revealed.get(field)
  const hint = (key?: string) => (key ? ` (${key})` : '')
  const reveal = button(value === undefined ? 'Show' : 'Hide', `Show / hide${hint(keys?.reveal)}`, () => toggleReveal(field))
  return row(label, value ?? '••••••••', () => copy(field, label), { actions: [reveal], copyKey: keys?.copy, valueClass: 'secret' })
}


/** Shown before the tags of an entry that is not in use. */
const PLACES: Record<Entry['kind'], string> = { entry: '', template: 'Template', trash: 'In the trash' }

const hasTotp = (entry: EntryDetail) => entry.fields.some((f) => f.name === OTP)
/** Fields by the names people know them by (the standard ones appear among
 *  the others only when protected, or as a version's differences). */
const NAMES: Record<string, string> = { Title: 'Title', [USERNAME]: 'User name', [PASSWORD]: 'Password', [URL_FIELD]: 'URL', Notes: 'Notes', [OTP]: 'TOTP' }
const labelOf = (field: string) => NAMES[field] ?? field

function renderDetail() {
  const shown = current
  // An entry fetched just as the editor opened must not draw over it.
  if (!shown || isEditing()) return
  if (view.kind === 'history') return renderHistory(shown, view.versions)
  const version = view.kind === 'version' ? view : null
  // An older version is shown like the entry, read only.
  const entry = version ? version.at : shown
  const tags = entry.tags.filter((t) => t !== FAVORITE).join(', ')
  const meta = [PLACES[entry.kind], tags].filter(Boolean).join(' · ')
  const heading = el('div', { className: 'heading' }, el('h2', {}, entry.title || '(no title)'))
  if (meta) heading.append(el('span', { className: 'meta' }, meta))
  if (entry.expires) {
    const state = expiry(entry, Date.now())
    const text = `${state === 'expired' ? 'Expired' : 'Expires'} ${formatDate(entry.expires)}`
    heading.append(el('span', { className: `expiry ${state ?? ''}` }, text))
  }
  const when = entry.modified ? formatDateTime(entry.modified) : null
  if (version) {
    heading.append(el('span', { className: 'meta changed' }, `Version saved ${when ?? '(no date)'} · read only`))
  } else {
    if (when) heading.append(el('span', { className: 'meta changed' }, `Changed ${when}`))
    if (entry.versions) {
      heading.append(button(`History (${entry.versions})`, 'Older versions of this entry', openHistory, 'ghost history'))
    }
  }
  const header = el('header', {}, iconImage(entry), heading)
  if (!version && editable(entry)) header.append(starButton(entry))
  const rows: Node[] = [header]
  if (entry.username) {
    rows.push(row('User name', entry.username, () => copy(USERNAME, 'User name'), { copyKey: 'Ctrl+B' }))
  }
  if (entry.hasPassword) rows.push(secretRow('Password', PASSWORD, { reveal: 'Ctrl+H', copy: 'Ctrl+C' }))
  if (entry.url) {
    // The entry's own address: not offered for an older version.
    const actions = entry.host && !version ? [button('Open', 'Open in the browser (Ctrl+U)', openUrl)] : []
    rows.push(row('URL', entry.url, () => copy(URL_FIELD, 'URL'), { actions }))
  }
  // An older version's TOTP secret is shown as a secret, not as codes.
  if (hasTotp(entry)) rows.push(version ? secretRow('TOTP', OTP) : totpRow(entry.id))
  for (const field of entry.fields.filter((f) => f.name !== OTP)) {
    if (field.protected) {
      rows.push(secretRow(labelOf(field.name), field.name))
    } else if (field.value) {
      rows.push(row(field.name, field.value, () => copy(field.name, field.name)))
    }
  }
  if (entry.notes) rows.push(el('p', { className: 'notes' }, entry.notes))
  if (entry.attachments.length) {
    rows.push(el('h3', {}, 'Attachments'), ...entry.attachments.map((file) => fileRow(file, !version && editable(entry))))
  }
  if (version) {
    if (version.at.differs.length) {
      rows.push(el('h3', {}, 'The entry now'), ...version.at.differs.map((d) =>
        el('div', { className: 'row now' },
          el('span', { className: 'label' }, labelOf(d.name)),
          el('span', { className: 'value' }, d.protected ? 'differs (not shown)' : d.current ?? 'not in the entry now'))))
    }
    const restorable = editable(shown) || isTemplate(shown)
    rows.push(el('div', { className: 'buttons' },
      ...(restorable ? [button('Restore this version', 'Make it the entry\'s content again; the current one goes into its history', restoreVersion, 'primary')] : []),
      button('Back', 'Back to the history (Esc)', back)))
  } else if (editable(entry)) {
    rows.push(
      el('div', { className: 'buttons' },
        button('Edit', 'Edit (Ctrl+E)', editEntry, 'primary'),
        button('Attach file…', 'Attach a file to this entry', attachFile),
        button('Delete', 'Move to the recycle bin (Del)', deleteEntry)),
    )
  } else if (isTemplate(entry)) {
    rows.push(
      el('div', { className: 'buttons' },
        button('New entry from it', 'A new entry with its fields, icon and tags', () => newFromTemplate(entry.id), 'primary'),
        button('Edit', 'Edit the template (Ctrl+E)', editEntry),
        button('Delete', 'Move to the recycle bin (Del)', deleteEntry)),
    )
  } else if (inTrash(entry)) {
    rows.push(
      el('div', { className: 'buttons' },
        button('Restore', 'Put it back where it was', () => restore([entry.id]), 'primary'),
        button('Delete permanently…', 'Delete it for good, on every device (Del)', deleteEntry, 'danger')),
    )
  }
  detail.replaceChildren(...rows)
}

// ---------------------------------------------------------------- history

/** The entry's older versions, newest first: when, and what changed after. */
function renderHistory(entry: EntryDetail, versions: Version[]) {
  const heading = el('div', { className: 'heading' }, el('h2', {}, entry.title || '(no title)'),
    el('span', { className: 'meta' }, `History · ${versions.length} older ${versions.length === 1 ? 'version' : 'versions'}`))
  const items = versions.map((v, i) => {
    const item = el('button', { type: 'button', className: 'version', title: 'Show this version' },
      el('span', { className: 'when' }, v.modified ? formatDateTime(v.modified) : '(no date)'),
      el('span', { className: 'what' }, v.changed.length ? `then changed: ${v.changed.join(', ')}` : 'nothing shown changed after it'))
    item.addEventListener('click', () => openVersion(i))
    return el('li', {}, item)
  })
  detail.replaceChildren(
    el('header', {}, iconImage(entry), heading),
    el('ul', { className: 'versions' }, ...items),
    el('div', { className: 'buttons' }, button('Back', 'Back to the entry (Esc)', back)),
  )
}

async function openHistory() {
  const entry = current
  if (!entry || isEditing()) return
  try {
    const versions = await api.entryHistory(entry.id)
    if (current !== entry) return
    view = { kind: 'history', versions }
    revealed.clear()
    stopTotp()
    renderDetail()
  } catch (e) {
    notify(String(e))
  }
}

async function openVersion(index: number) {
  const entry = current
  if (!entry || view.kind !== 'history') return
  const versions = view.versions
  try {
    const at = await api.entryVersion(entry.id, index)
    if (current !== entry) return
    view = { kind: 'version', versions, index, at }
    revealed.clear()
    renderDetail()
  } catch (e) {
    notify(String(e))
  }
}

/** One step back: from a version to the history, from the history to the
 *  entry. False when the entry itself is shown. */
function back(): boolean {
  if (view.kind === 'entry') return false
  view = view.kind === 'version' ? { kind: 'history', versions: view.versions } : ENTRY_VIEW
  revealed.clear()
  renderDetail()
  return true
}

/** Asks first: the entry's current content goes into its history. */
async function restoreVersion() {
  const entry = current
  if (!entry || view.kind !== 'version') return
  const { index, at } = view
  const when = at.modified ? formatDateTime(at.modified) : 'this version'
  const yes = await ask(`Restore the version saved ${when}? The entry's current content goes into its history.`, 'Restore')
  if (!yes || current !== entry) return
  try {
    const next = await api.restoreVersion(entry.id, index, at.modified)
    view = ENTRY_VIEW
    applyListing(next, 'Restored the older version · the replaced one is in the history')
  } catch (e) {
    notify(String(e), 6)
    // The history may have moved on: show it as it is now.
    if (current === entry) {
      view = ENTRY_VIEW
      openHistory()
    }
  }
}

/** An attached file: its name and size; the content is only ever saved to disk.
 *  Only an entry in use has its files changed. */
function fileRow(file: Attachment, changeable: boolean): HTMLDivElement {
  const save = { label: 'Save…', title: 'Save to a file on this PC', action: () => saveAttachment(file.name) }
  const changes = [
    { label: 'Replace…', title: 'Replace with another file (its history keeps this one)', action: () => replaceAttachment(file.name) },
    { label: 'Rename…', title: 'Rename (its history keeps the old name)', action: () => renameAttachment(file.name) },
    { label: 'Remove…', title: 'Remove from this entry (its history keeps the file)', action: () => removeAttachment(file.name), danger: true },
  ]
  return el(
    'div',
    { className: 'row file' },
    el('span', { className: 'value' }, file.name),
    el('span', { className: 'size' }, formatSize(file.size)),
    el('span', { className: 'actions' },
      button('Open', 'Open in its app; changes made there are not saved', () => openAttachment(file.name)),
      menuButton(`More for ${file.name}`, changeable ? [save, ...changes] : [save])),
  )
}

// ---------------------------------------------------------------- several entries

/** What can be done to the chosen entries at once, each change saved as one. */
function renderSeveral() {
  if (!several || isEditing()) return
  const chosen = listing.entries.filter((e) => several!.chosen.includes(e.id))
  const inUse = chosenInUse()
  const trashed = chosenInTrash()
  const heading = el('div', { className: 'heading' }, el('h2', {}, `${chosen.length} entries chosen`),
    el('span', { className: 'meta' }, 'Ctrl+click adds or takes one off, Shift+click chooses a range'))
  // One bar at the bottom of the column, as an entry's.
  const actions: HTMLButtonElement[] = []
  const templates = chosenTemplates()
  if (templates.length) {
    actions.push(button('Delete…', 'Move these templates to the recycle bin (Del)', () => deleteSeveral(templates.map((e) => e.id)), 'danger'))
  }
  if (trashed.length) {
    const ids = trashed.map((e) => e.id)
    actions.push(
      button('Restore', 'Put them back where they were', () => restore(ids), 'primary'),
      button('Delete permanently…', 'Delete them for good, on every device (Del)', () => deleteForGood(ids), 'danger'))
  }
  if (inUse.length) {
    const ids = inUse.map((e) => e.id)
    const theirTags = [...new Set(inUse.flatMap((e) => e.tags.filter((t) => t !== FAVORITE)))].sort((a, b) => a.localeCompare(b))
    const untag = button('Remove tag…', 'Take a tag off these entries', () => untagSeveral(ids, theirTags))
    untag.disabled = theirTags.length === 0
    actions.push(
      button('Add tag…', 'Give these entries a tag', () => tagSeveral(ids)),
      untag,
      button('★ Favorite', 'Add these entries to Favorites', () => changeSeveral(ids, FAVORITE, true)),
      button('Not favorite', 'Remove these entries from Favorites', () => changeSeveral(ids, FAVORITE, false)),
      button('Delete…', 'Move these entries to the recycle bin (Del)', () => deleteSeveral(ids), 'danger'))
  }
  const rows: Node[] = [el('header', {}, heading)]
  if (actions.length) rows.push(el('div', { className: 'buttons' }, ...actions))
  detail.replaceChildren(...rows)
}

/** The chosen entries that can be changed here: not templates, not the trash's. */
function chosenInUse(): Entry[] {
  return several ? listing.entries.filter((e) => several!.chosen.includes(e.id) && editable(e)) : []
}

function chosenInTrash(): Entry[] {
  return several ? listing.entries.filter((e) => several!.chosen.includes(e.id) && inTrash(e)) : []
}

function chosenTemplates(): Entry[] {
  return several ? listing.entries.filter((e) => several!.chosen.includes(e.id) && isTemplate(e)) : []
}

async function tagSeveral(ids: string[]) {
  const known = tagCounts(listing.entries).map(([tag]) => tag)
  const tag = await askText(`Add a tag to ${ids.length} entries:`, '', 'Add tag', 0, known)
  if (tag?.trim()) changeSeveral(ids, tag.trim(), true)
}

async function untagSeveral(ids: string[], tags: string[]) {
  const i = await choose(`Take which tag off ${ids.length} entries?`, tags)
  if (i !== null) changeSeveral(ids, tags[i], false)
}

async function changeSeveral(ids: string[], tag: string, on: boolean) {
  const what = tag === FAVORITE ? (on ? 'Added to Favorites' : 'Removed from Favorites') : `${on ? 'Tagged' : 'Took the tag off'} "${tag}"`
  try {
    applyListing(await api.setTag(ids, tag, on), `${what} · ${ids.length} entries`, false)
  } catch (e) {
    notify(String(e))
  }
}

async function deleteSeveral(ids: string[]) {
  if (!ids.length || !await ask(`Move ${ids.length} entries to the recycle bin?`, 'Move to the recycle bin')) return
  try {
    const next = await api.deleteEntries(ids)
    several = null
    applyListing(next, `Moved ${ids.length} entries to the recycle bin`)
  } catch (e) {
    notify(String(e))
  }
}

// ---------------------------------------------------------------- trash

function inTrash<T extends Entry>(entry: T | null): entry is T & { kind: 'trash' } {
  return entry?.kind === 'trash'
}

function count(n: number): string {
  return `${n} ${n === 1 ? 'entry' : 'entries'}`
}

/** Puts entries from the trash back where they were. */
async function restore(ids: string[]) {
  try {
    const next = await api.restoreEntries(ids)
    several = null
    applyListing(next, ids.length === 1 ? 'Restored' : `Restored ${count(ids.length)}`)
  } catch (e) {
    notify(String(e))
  }
}

/** Deletes entries in the trash for good (no ids: everything in it), after asking. */
async function deleteForGood(ids?: string[]) {
  const what = !ids ? 'everything in the trash'
    : ids.length === 1 ? `"${listing.entries.find((e) => e.id === ids[0])?.title || '(no title)'}"` : count(ids.length)
  const yes = await ask(`Delete ${what} permanently? This cannot be undone: other devices delete it too when they sync.`, 'Delete permanently')
  if (!yes) return
  try {
    const next = await (ids ? api.deleteForGood(ids) : api.emptyTrash())
    several = null
    applyListing(next, 'Deleted permanently')
  } catch (e) {
    notify(String(e))
  }
}

// ---------------------------------------------------------------- tags

/** How many entries have the tag: templates and the trash's too, as renaming
 *  and removing a tag change every entry (a template would bring it back). */
function entriesTagged(tag: string): string {
  return count(listing.entries.filter((e) => e.tags.includes(tag)).length)
}

/** Renames a tag in every entry; to a name another tag has, after asking, the two merge. */
async function renameTag(tag: string) {
  const to = (await askText(`Rename the tag "${tag}" (${entriesTagged(tag)}) to:`, tag, 'Rename'))?.trim()
  if (!to || to === tag) return
  if (listing.entries.some((e) => e.tags.includes(to))) {
    if (!await ask(`There is a tag "${to}" already. Merge "${tag}" into it?`, 'Merge')) return
  }
  try {
    const next = await api.renameTag(tag, to)
    if (sameFilter(filter, { kind: 'tag', tag })) filter = { kind: 'tag', tag: to }
    applyListing(next, `Renamed the tag "${tag}" to "${to}"`)
  } catch (e) {
    notify(String(e))
  }
}

async function removeTag(tag: string) {
  const yes = await ask(`Take the tag "${tag}" off ${entriesTagged(tag)}? The entries stay.`, 'Remove the tag')
  if (!yes) return
  try {
    applyListing(await api.removeTag(tag), `Removed the tag "${tag}"`)
  } catch (e) {
    notify(String(e))
  }
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
  const value = copyOnClick(el('span', { className: 'value secret totp' }, '…'), copyTotp)
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

function startEditor(id: string | null, start: { focusPassword?: boolean; from?: EntryData; template?: boolean } = {}) {
  if (isEditing()) return
  several = null
  stopTotp()
  openEditor(detail, {
    id,
    ...start,
    // New entries go to the top group.
    group: [],
    knownTags: tagCounts(listing.entries).map(([tag]) => tag),
    defaultUsername: listing.database.defaultUsername,
    autoIcon: autoIcon(id ? (listing.entries.find((e) => e.id === id) ?? null) : null),
    onSaved: afterSave,
    onClose: () => {
      current = null
      select(selectedId)
    },
  }).catch((e) => notify(String(e)))
}

/** An entry in use: starred, tagged, given files. Templates are edited and
 *  deleted too (see [isTemplate]); the trash's are only restored or deleted. */
function editable<T extends Entry>(entry: T | null): entry is T & { kind: 'entry' } {
  return entry?.kind === 'entry'
}

function isTemplate<T extends Entry>(entry: T | null): entry is T & { kind: 'template' } {
  return entry?.kind === 'template'
}

function editEntry() {
  if (view.kind !== 'entry') return
  if (editable(current) || isTemplate(current)) startEditor(current.id, { template: isTemplate(current) })
}

/** New entry (Ctrl+N): blank, or from one of the templates; in the
 *  Templates group, a new template. */
async function newEntry() {
  if (isEditing()) return
  if (sameFilter(filter, TEMPLATES)) return startEditor(null, { template: true })
  const templates = listing.entries.filter(isTemplate)
  if (!templates.length) return startEditor(null)
  const i = await choose('New entry:', ['Blank entry', ...templates.map((t) => t.title || '(no title)')])
  if (i === null) return searchInput.focus()
  if (i === 0) startEditor(null)
  else newFromTemplate(templates[i - 1].id)
}

/** A new entry with the template's fields, icon and tags, not its title. */
async function newFromTemplate(id: string) {
  try {
    startEditor(null, { from: await api.editEntry(id) })
  } catch (e) {
    notify(String(e))
  }
}

/** Shows a listing the backend sent after a change, then says what happened.
 *  The open entry is fetched again. */
function applyListing(next: Listing, message: string, focusSearch = true) {
  listing = next
  fillSidebar()
  // The entry is shown again from the top: a version's revealed values go.
  if (view.kind !== 'entry') revealed.clear()
  view = ENTRY_VIEW
  current = null
  refresh()
  if (focusSearch) searchInput.focus()
  notify(message)
}

function afterSave(saved: Saved) {
  selectedId = saved.id
  // The saved entry shows: among All (a template among the templates),
  // without the search, when the sidebar's choice or the search leaves it out.
  if (!search(saved.listing.entries, searchInput.value, filter).some((e) => e.id === saved.id)) {
    filter = saved.listing.entries.some((e) => e.id === saved.id && isTemplate(e)) ? TEMPLATES : ALL
    searchInput.value = ''
  }
  const replaced = saved.conflicts.join(', ')
  applyListing(saved.listing, replaced ? `Saved. Replaced a change made on another device (${replaced}); it is in the entry's history` : 'Saved')
}

/** Del / the Delete button: asks first. In the trash, deletes for good. */
async function deleteEntry() {
  const entry = current
  if (view.kind !== 'entry') return
  if (inTrash(entry) && !isEditing()) return deleteForGood([entry.id])
  if (!(editable(entry) || isTemplate(entry)) || isEditing()) return
  const yes = await ask(`Move "${entry.title || '(no title)'}" to the recycle bin?`, 'Move to the recycle bin')
  // An update from another device may refresh the view meanwhile; the choice still stands.
  if (!yes || selectedId !== entry.id) return searchInput.focus()
  try {
    applyListing(await api.deleteEntries([entry.id]), 'Moved to the recycle bin')
  } catch (e) {
    notify(String(e))
  }
}

async function attachFile() {
  const entry = current
  if (!editable(entry) || isEditing()) return
  try {
    const attached = await api.attachFile(entry.id)
    if (attached) applyListing(attached.listing, `Attached ${attached.name}`)
  } catch (e) {
    notify(String(e))
  }
}

async function replaceAttachment(name: string) {
  const entry = current
  if (!entry || isEditing()) return
  try {
    const listing = await api.replaceAttachment(entry.id, name)
    if (listing) applyListing(listing, `Replaced ${name} · the previous one is in the entry's history`)
  } catch (e) {
    notify(String(e))
  }
}

async function renameAttachment(name: string) {
  const entry = current
  if (!entry || isEditing()) return
  const to = await askText(`Rename "${name}" to:`, name, 'Rename', beforeExtension(name))
  if (to === null || to.trim() === name || selectedId !== entry.id) return
  try {
    const renamed = await api.renameAttachment(entry.id, name, to)
    applyListing(renamed.listing, `Renamed to ${renamed.name}`)
  } catch (e) {
    notify(String(e))
  }
}

/** Asks first, like deleting an entry. */
async function removeAttachment(name: string) {
  const entry = current
  if (!entry || isEditing()) return
  const yes = await ask(`Remove "${name}" from "${entry.title || '(no title)'}"? The entry's history keeps it.`, 'Remove')
  if (!yes || selectedId !== entry.id) return
  try {
    applyListing(await api.removeAttachment(entry.id, name), `Removed ${name}`)
  } catch (e) {
    notify(String(e))
  }
}

async function openAttachment(name: string) {
  if (!current) return
  try {
    await api.openAttachment(current.id, name, shownVersion())
    notify(`Opened a read-only copy of ${name} · deleted when the database locks`, 5)
  } catch (e) {
    notify(String(e))
  }
}

async function saveAttachment(name: string) {
  if (!current) return
  try {
    if (await api.saveAttachment(current.id, name, shownVersion())) notify(`Saved ${name}`)
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
    const value = await api.reveal(id, field, shownVersion()).catch((e) => (notify(String(e)), null))
    if (value === null || current?.id !== id) return
    revealed.set(field, value)
  }
  renderDetail()
}

async function copy(field: string, label: string) {
  if (!current) return
  try {
    const seconds = await api.copy(current.id, field, shownVersion())
    notify(`${label} copied · clears in ${seconds} s`)
  } catch (e) {
    notify(String(e))
  }
}

function openUrl() {
  if (view.kind === 'entry' && current?.host) api.openUrl(current.id).catch((e) => notify(String(e)))
}

/** What the entry view shows now: the entry, or the older version open. */
const shownDetail = (): EntryDetail | null => (view.kind === 'version' ? view.at : current)

let toastTimer: number | undefined
function notify(message: string, seconds = 3) {
  toast.textContent = message
  toast.hidden = false
  clearTimeout(toastTimer)
  toastTimer = window.setTimeout(() => (toast.hidden = true), seconds * 1000)
}

/** The sync line in the toolbar; hidden for a database that is not synced. */
function showSyncStatus(status: SyncStatus) {
  const line = $('sync-status')
  line.hidden = !status.remote || !status.text && !status.busy
  line.textContent = status.busy ? 'Syncing…' : status.text
  line.title = line.textContent
  line.classList.toggle('problem', status.problem && !status.busy)
  $('sync-button').hidden = !status.remote
  $<HTMLButtonElement>('sync-button').disabled = status.busy
}

/** The copy that needs a key, as the app last said (`key-needed`); null when none does. */
let keyNeededFor: string | null = null
/** A key was asked for in a dialog since the unlock: that happens once, then the button stays. */
let keyDialogShown = false

/** The *Enter key…* button while a copy needs a key, and the dialog the first time. */
function showKeyNeeded({ local, remote }: KeyNeeded) {
  keyNeededFor = local ? 'the file on this PC' : remote ? 'the remote file' : null
  $('key-button').hidden = !keyNeededFor
  if (keyNeededFor) askForOtherKey()
}

/** Asks for the key another device changed a copy to, in a dialog once per
 *  unlock unless `again` (the button). */
async function askForOtherKey(again = false) {
  if (!keyNeededFor || !unlocked || isAsking() || isEditing() || settingsOpen || (keyDialogShown && !again)) return
  keyDialogShown = true
  // The app tells the window what the key given changed (`key-needed`).
  if (await enterOtherKey(keyNeededFor)) notify('Read with the key given')
}

/** The file changed on disk and was read again: show the new state in place. */
function showDiskChange({ listing: next, changed }: DiskChange) {
  if (!unlocked) return
  // Focus stays where the user left it.
  applyListing(next, 'Updated from another device', false)
  changedElsewhere(changed)
}

// ---------------------------------------------------------------- password health

async function openHealth() {
  if (healthOpen || !unlocked || isEditing()) return
  try {
    await renderHealth(healthView, { done: closeHealth, fix: changePassword, icon: iconOf })
  } catch (e) {
    return notify(String(e))
  }
  if (!unlocked) return // locked while checking
  healthOpen = true
  vault.hidden = true
  healthView.hidden = false
  healthView.querySelector('button')?.focus()
}

function closeHealth() {
  if (!healthOpen) return
  healthOpen = false
  healthView.hidden = true
  healthView.replaceChildren()
  if (unlocked && !settingsOpen) {
    vault.hidden = false
    searchInput.focus()
  }
}

function iconOf(id: string): HTMLElement {
  const entry = listing.entries.find((e) => e.id === id)
  return entry ? iconImage(entry) : el('img', { className: 'icon', alt: '', src: DEFAULT_ICON })
}

/** "Change password" in the report: the entry, shown in the list, in the editor. */
function changePassword(id: string) {
  closeHealth()
  searchInput.value = ''
  filter = ALL
  fillSidebar()
  refresh()
  select(id)
  startEditor(id, { focusPassword: true })
}

// ---------------------------------------------------------------- settings

/** The settings screen. A changed database setting goes into the listing (the
 *  default user name a new entry gets), and the entry shown is fetched again
 *  when it closes (new history limits may have trimmed its history). */
const drawSettings = () =>
  renderSettings(settingsView, closeSettings, (message) => notify(message, 6), (database) => {
    listing = { ...listing, database }
    current = null
  })

async function openSettings() {
  if (settingsOpen) return
  try {
    await drawSettings()
  } catch (e) {
    return notify(String(e))
  }
  closeHealth()
  settingsOpen = true
  unlockForm.hidden = true
  chooseView.hidden = true
  vault.hidden = true
  settingsView.hidden = false
  settingsView.querySelector('button')?.focus()
}

/** Back to the vault, or to the unlock screen when locked meanwhile. */
function closeSettings() {
  if (!settingsOpen) return
  settingsOpen = false
  settingsView.hidden = true
  settingsView.replaceChildren()
  if (unlocked) {
    vault.hidden = false
    searchInput.focus()
    if (!current) select(selectedId)
  } else if (chooseView.childElementCount) {
    chooseView.hidden = false
    chooseView.querySelector<HTMLElement>('input:checked')?.focus()
  } else {
    // Locked while the settings were open: the unlock screen, or the
    // choose-database screen when there is no database.
    api.status().then(showUnlock, (e) => notify(String(e)))
  }
}

// ---------------------------------------------------------------- keys

searchInput.addEventListener('input', refresh)
$('lock-button').addEventListener('click', lock)
$('new-entry').addEventListener('click', newEntry)
$('empty-trash').addEventListener('click', () => deleteForGood())
$('settings-button').addEventListener('click', openSettings)
$('health-button').addEventListener('click', openHealth)
$('sync-button').addEventListener('click', () => api.syncNow().catch((e) => notify(String(e))))
$('key-button').addEventListener('click', () => askForOtherKey(true))

/** Esc with nothing left to close: back to the tray. */
function hideWindow() {
  api.hideWindow().catch((e) => notify(String(e)))
}

function perform(action: Action, e: KeyboardEvent) {
  switch (action) {
    case 'copy-username':
      // A protected user name is among the fields instead.
      if (shownDetail()?.username || shownDetail()?.fields.some((f) => f.name === USERNAME)) copy(USERNAME, 'User name')
      break
    case 'copy-password':
      if (shownDetail()?.hasPassword) copy(PASSWORD, 'Password')
      break
    case 'open-url':
      openUrl()
      break
    case 'toggle-password':
      if (shownDetail()?.hasPassword) toggleReveal(PASSWORD)
      break
    case 'lock':
      lock()
      break
    case 'copy-totp':
      // The code is the entry's own: not while an older version is shown.
      if (view.kind === 'entry' && current && hasTotp(current)) copyTotp()
      break
    case 'new-entry':
      newEntry()
      break
    case 'edit-entry':
      editEntry()
      break
    case 'delete-entry':
      // Several chosen: in the trash, they are deleted for good; elsewhere,
      // moved to it. The list shows one or the other, never both.
      if (several) {
        const [moved, trashed] = [[...chosenInUse(), ...chosenTemplates()], chosenInTrash()]
        if (moved.length) deleteSeveral(moved.map((x) => x.id))
        else if (trashed.length) deleteForGood(trashed.map((x) => x.id))
      } else deleteEntry()
      break
    case 'previous':
      move(-1)
      break
    case 'next':
      move(1)
      break
    case 'escape':
      // From a version to the history, from the history to the entry first.
      if (back()) break
      if (searchInput.value) {
        searchInput.value = ''
        refresh()
        searchInput.focus()
      } else {
        hideWindow()
      }
      break
    case 'settings':
      openSettings()
      break
    case 'type-to-search':
      searchInput.focus()
      return // the key itself goes on into the search field
  }
  e.preventDefault()
}

document.addEventListener('keydown', (e) => {
  if (isAsking()) return // the question on screen has the keys
  if (healthOpen) {
    if (e.key === 'Escape') {
      e.preventDefault()
      closeHealth()
    } else if (e.ctrlKey && e.code === 'KeyL') {
      e.preventDefault()
      lock()
    }
    return
  }
  if (settingsOpen) {
    if (e.key === 'Escape') {
      e.preventDefault()
      closeSettings()
    } else if (unlocked && e.ctrlKey && e.code === 'KeyL') {
      e.preventDefault()
      lock()
    }
    return
  }
  if (!unlocked) {
    if (e.key === 'Escape') {
      if (!chooseView.hidden && chooseBack) chooseBack()
      else hideWindow()
    }
    if (e.ctrlKey && e.code === 'Comma') {
      e.preventDefault()
      openSettings()
    }
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
  if (!unlocked || now - lastTouch < 5000) return
  lastTouch = now
  api.touch().catch(() => {})
}
// Pointer movement alone does not count: hovering over the window is not using it.
for (const type of ['keydown', 'pointerdown', 'wheel']) {
  document.addEventListener(type, reportActivity, { passive: true, capture: true })
}

listen('locked', showLocked)
listen<DiskChange>('database-changed', (e) => showDiskChange(e.payload))
listen<string>('database-error', (e) => notify(e.payload, 10))
listen<SyncStatus>('sync-status', (e) => showSyncStatus(e.payload))
// A copy opens with a key this device does not know, or no longer does.
listen<KeyNeeded>('key-needed', (e) => showKeyNeeded(e.payload))
listen('window-shown', () => {
  if (settingsOpen || healthOpen) return
  if (unlocked) searchInput.focus()
  else if (!chooseView.hidden) chooseView.querySelector<HTMLElement>('input:checked')?.focus()
  else passwordInput.focus()
})
// A database named on the command line of a second launch was chosen.
listen('status-changed', async () => {
  if (!unlocked) showUnlock(await api.status())
})
listen<string>('notice', (e) => {
  if (unlocked) notify(e.payload, 10)
  else {
    unlockError.textContent = e.payload
    unlockError.hidden = false
  }
})
listen('open-settings', () => {
  if (!isAsking()) openSettings()
})
// Changed from the tray while the screen is open.
listen('settings-changed', () => {
  if (settingsOpen) drawSettings().catch((e) => notify(String(e)))
})

listen<string>('icon-ready', (e) => {
  siteIcons.delete(e.payload)
  loadSiteIcon(e.payload)
})

// Which copy this is, under the name on the unlock screen.
getVersion().then((version) => ($('app-version').textContent = `Version ${version}`), () => {})

api.status().then(async (status) => {
  if (status.unlocked) showVault(await api.listing())
  else showUnlock(status)
})
