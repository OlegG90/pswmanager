import { OTP, PASSWORD, URL_FIELD, USERNAME } from '../../../../src/api'
import { button, el } from '../../../../src/dom'
import { formatDateTime, formatSize, labelOf, splitCode, titleOf } from '../../../../src/entry-text'
import { shownIcon } from '../../../../src/icons'
import { FAVORITE } from '../../../../src/search'
import { api, type EntryDetail, type Listing, type Version } from './api'
import { editorScreen, newFromTemplate } from './editor'
import type { IconName } from './icons'
import { icon, listScreen, toListSaying } from './list'
import { confirmSheet, copied, iconButton, screens, sheet, show, snack } from './page'
import { browsePhone, onPhone } from './phone-files'

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
  // The entry's URL or a further one (#254); not a version's.
  const opens = (field?: string): Action[] =>
    version === null ? [['Open in the browser', 'externalLink', () => void api.openUrl(id, field).catch((e) => snack(String(e)))]] : []
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
    rows.push(line('URL', entry.url, copy(URL_FIELD), entry.link ? opens() : []))
  }
  for (const field of entry.fields.filter((f) => f.name !== OTP)) {
    const label = labelOf(field.name)
    if (field.protected) rows.push(secretLine(id, label, field.name, version))
    else if (field.value) rows.push(line(label, field.value, copy(field.name), field.link ? opens(field.name) : []))
    // An empty field is what a template is for: nothing to copy, but it is there.
    else if (entry.kind === 'template') rows.push(line(label, '', null))
  }
  if (entry.notes) rows.push(el('div', { className: 'line notes' }, el('span', {}, el('small', {}, 'Notes'), el('span', {}, entry.notes))))
  if (entry.attachments.length) {
    const open = (name: string) => void api.openAttachment(id, name, version).catch((e) => snack(String(e)))
    const save = (name: string) => void onPhone<boolean>(`Where ${name} goes`,
      (done, picker) => browsePhone({ first: () => false, makeName: name, make: (folder, as) => api.saveAttachmentAt(id, name, version, folder, as).then(() => true), done, picker }),
      () => api.saveAttachment(id, name, version)).then((saved) => saved && snack(`${name} saved`), (e) => snack(String(e)))
    rows.push(el('h2', {}, 'Attachments'), ...entry.attachments.map((a) => line(a.name, formatSize(a.size), null, [
      ['Open', 'fileOutput', () => open(a.name)],
      ['Save to a file', 'download', () => save(a.name)],
    ])))
  }
  return [rows, stop]
}

/** A sync that changed entries shows entry `id` again, or the list when it is gone. */
function showAgainOnSync(id: string) {
  screens.onSynced((synced) => {
    if (synced.changed) void api.listing().then((fresh) => (fresh.entries.some((e) => e.id === id) ? entryScreen(id, fresh) : listScreen(fresh)))
  })
}

export async function entryScreen(id: string, listing: Listing) {
  const entry = await api.entry(id)
  const [rows, stop] = entryLines(entry, null)
  if (entry.modified) rows.push(el('p', { className: 'muted' }, `Changed ${formatDateTime(entry.modified)}`))
  if (entry.versions) rows.push(button(`History (${entry.versions})`, 'Older versions of this entry', () => void historyScreen(entry, listing), 'link'))
  const toList = () => listScreen(listing)
  const back = iconButton('back', 'Back to the list', toList)
  const inBin = entry.kind === 'trash'
  if (inBin) rows.push(trashButtons(entry))
  if (entry.kind === 'template') {
    rows.push(button('New entry from it', 'A new entry with its fields, icon and tags', () => void newFromTemplate(id, listing, () => void entryScreen(id, listing)), 'primary'))
  }
  const commands = inBin ? [] : entryCommands(entry, listing)
  show([el('header', { className: 'bar' }, back, icon(entry, listing), el('h1', {}, titleOf(entry)), ...commands), ...rows], toList)
  screens.onLeave(stop)
  showAgainOnSync(id)
}

/** The toolbar's star, pencil and ⋮ (with Delete) of an entry in use; a
 *  template has no star. */
function entryCommands(entry: EntryDetail, listing: Listing): HTMLElement[] {
  const id = entry.id
  const template = entry.kind === 'template'
  const starred = entry.tags.includes(FAVORITE)
  const star = iconButton('star', starred ? 'Not favorite' : 'Favorite', () =>
    void api.setTag([id], FAVORITE, !starred).then((fresh) => entryScreen(id, fresh), (e) => snack(String(e))), starred ? 'icon starred' : 'icon')
  const edit = iconButton('pencil', 'Edit', () => void editorScreen(id, listing, () => void entryScreen(id, listing), { template }))
  const remove = () =>
    confirmSheet(`Move “${titleOf(entry)}” to the recycle bin?`, 'Delete', () =>
      void api.deleteEntries([id]).then(toListSaying('Moved to the recycle bin'), (e) => snack(String(e))))
  const more = iconButton('more', 'More', () => sheet((close) => [el('b', {}, titleOf(entry)), button('Delete', 'Move to the recycle bin', () => (close(), remove()), 'item')]))
  return template ? [edit, more] : [star, edit, more]
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
        if (!now) return stop()
        code.textContent = splitCode(now.code)
        remaining = now.remaining
      } catch (e) {
        // A secret no code is made from (HOTP, an unreadable value): said once, not every second.
        stop()
        code.textContent = String(e)
        left.textContent = ''
        return
      } finally {
        fetching = false
      }
    } else remaining--
    left.textContent = ` ${remaining} s`
  }
  const stop = () => clearInterval(timer)
  const timer = window.setInterval(() => void tick(), 1000)
  void tick()
  return [line('TOTP', el('span', {}, code, left), () => api.copyTotp(id)), stop]
}
