import { OTP, PASSWORD, URL_FIELD, USERNAME, type Entry, type MergePreview, type Similar } from './api'
import { el } from './dom'
import { labelOf, titleOf } from './entry-text'

/** What both apps' Find similar screens say and do alike (docs/spec.md,
 *  *Similar entries*); each draws it in its own way. */

export const SIMILAR_INTRO =
  'Entries whose URLs are for the same site. Merging keeps one entry with everything the others have; they go to the recycle bin.'
export const NO_SIMILAR = 'No two entries are for the same site.'
export const NONE_TICKED = 'Tick the entries to merge into the kept one'

/** One site's entries, the most recently changed first. */
export interface Site {
  name: string
  entries: Entry[]
}

/** The sites the backend `found`, but those `left` as they are, with the
 *  entries the list has (in use); a site with one left is not one any more. */
export function similarSites(found: Similar[], entry: (id: string) => Entry | undefined, left: Set<string>): Site[] {
  return found
    .filter((s) => !left.has(s.site))
    .map((s) => ({ name: s.site, entries: s.ids.map(entry).filter((e): e is Entry => e?.kind === 'entry') }))
    .filter((s) => s.entries.length > 1)
}

/**
 * The choice among one site's entries: one is kept (the most recently changed,
 * at first), the others are ticked to merge into it. `row` lays out each
 * entry's tick and Keep; the kept one has no tick, and choosing another to
 * keep ticks the one kept before in its place.
 */
export function siteChoice(site: Site, row: (entry: Entry, tick: HTMLInputElement, keep: HTMLInputElement) => HTMLElement) {
  let keep = site.entries[0].id
  const rows = site.entries.map((entry) => {
    const kept = entry.id === keep
    const tick = el('input', { type: 'checkbox', checked: !kept, disabled: kept, title: 'Merge this one', ariaLabel: `Merge ${titleOf(entry)}` })
    const keepRadio = el('input', { type: 'radio', name: `keep-${site.name}`, checked: kept, title: 'Keep this one', ariaLabel: `Keep ${titleOf(entry)}` })
    keepRadio.addEventListener('change', () => {
      const before = rows.find((r) => r.entry.id === keep)!.tick
      before.disabled = false
      before.checked = true
      keep = entry.id
      tick.checked = false
      tick.disabled = true
    })
    return { entry, tick, node: row(entry, tick, keepRadio) }
  })
  return {
    rows: rows.map((r) => r.node),
    kept: () => rows.find((r) => r.entry.id === keep)!.entry,
    /** What Merge takes: the ticked ones. */
    toMerge: () => rows.filter((r) => r.tick.checked && r.entry.id !== keep).map((r) => r.entry),
  }
}

/** Takes a site's `section` off the screen (`container`), saying `none` when it was the last. */
export function leaveSite(container: HTMLElement, section: HTMLElement, none: () => HTMLElement) {
  section.replaceWith(...(container.querySelectorAll('section').length > 1 ? [] : [none()]))
}

/** The text under a similar entry's title. */
export const similarHint = (entry: Entry) => [entry.username, entry.url].filter(Boolean).join(' · ')

const MASK = '••••••••'
const PASSKEY = 'KPEX_PASSKEY_'

/**
 * The kept entry as the merge would leave it (`preview`): what the entry view
 * shows, secrets masked, and what the merge gives it in another colour with a
 * star. `icon` is the entry's icon as the app draws it.
 */
export function mergedCard(preview: MergePreview, icon: Node): HTMLElement {
  const { entry, added } = preview
  const isNew = (field: string) => added.fields.includes(field)
  const line = (label: string, value: Node | string, fresh: boolean) =>
    el('div', { className: fresh ? 'merge-line new' : 'merge-line' },
      el('span', { className: 'name' }, fresh ? `${label} *` : label),
      el('span', { className: 'value' }, value))
  const lines: HTMLElement[] = []
  if (entry.username) lines.push(line(labelOf(USERNAME), entry.username, isNew(USERNAME)))
  if (entry.hasPassword) lines.push(line(labelOf(PASSWORD), MASK, isNew(PASSWORD)))
  if (entry.url) lines.push(line(labelOf(URL_FIELD), entry.url, isNew(URL_FIELD)))
  if (entry.otp) lines.push(line(labelOf(OTP), MASK, isNew(OTP)))
  if (entry.passkey) lines.push(line('Passkey', 'Passkey', added.fields.some((f) => f.startsWith(PASSKEY))))
  if (entry.notes) lines.push(line('Notes', entry.notes, isNew('Notes')))
  for (const field of entry.fields.filter((f) => f.name !== OTP && !f.name.startsWith(PASSKEY))) {
    lines.push(line(field.name, field.protected ? MASK : (field.value ?? ''), isNew(field.name)))
  }
  if (entry.tags.length) {
    const tags = entry.tags.map((tag) => {
      const fresh = added.tags.includes(tag)
      return el('span', { className: fresh ? 'merge-tag new' : 'merge-tag' }, fresh ? `#${tag} *` : `#${tag}`)
    })
    lines.push(line('Tags', el('span', {}, ...tags), added.tags.length > 0))
  }
  for (const file of entry.attachments) lines.push(line('File', file.name, added.files.includes(file.name)))
  // An entry holds one passkey: another one stays in its entry, in the recycle bin.
  const left = preview.passkeysLeft.map((title) =>
    el('p', { className: 'merge-warning' }, `The passkey of "${title}" is not carried over: this entry has its own. It stays in "${title}", in the recycle bin.`))
  return el('div', { className: 'merge-card' },
    el('div', { className: added.icon ? 'merge-head new' : 'merge-head' }, icon, el('b', {}, added.icon ? `${titleOf(entry)} *` : titleOf(entry))),
    ...lines,
    ...left,
    el('p', { className: 'muted merge-legend' }, '* What the merge adds'))
}
