import type { Entry, MergePreview, Similar } from './api'
import { el } from './dom'
import { describeEntries, titleOf } from './entry-text'

/** What both apps' Find similar screens say and do alike (docs/spec.md,
 *  *Similar entries*); each draws it in its own way. */

export const SIMILAR_INTRO = 'Entries whose URLs are for the same site. Merging keeps one entry with everything the others have; they go to the recycle bin.'
export const NO_SIMILAR = 'No two entries are for the same site.'
export const NONE_TICKED = 'Tick the entries to merge or delete'

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
 * at first), the others are ticked. `row` lays out each entry's tick and Keep;
 * choosing another to keep ticks the one kept before in its place. Titles in
 * questions are in `quote`.
 */
export function siteChoice(
  site: Site,
  row: (entry: Entry, tick: HTMLInputElement, keep: HTMLInputElement) => HTMLElement,
  quote: (title: string) => string,
) {
  let keep = site.entries[0].id
  const rows = site.entries.map((entry) => {
    const tick = el('input', { type: 'checkbox', checked: entry.id !== keep, title: 'Merge or delete this one', ariaLabel: `Merge or delete ${titleOf(entry)}` })
    const keepRadio = el('input', { type: 'radio', name: `keep-${site.name}`, checked: entry.id === keep, title: 'Keep this one', ariaLabel: `Keep ${titleOf(entry)}` })
    keepRadio.addEventListener('change', () => {
      rows.find((r) => r.entry.id === keep)!.tick.checked = true
      keep = entry.id
      tick.checked = false
    })
    return { entry, tick, node: row(entry, tick, keepRadio) }
  })
  const kept = () => rows.find((r) => r.entry.id === keep)!.entry
  const ticked = () => rows.filter((r) => r.tick.checked).map((r) => r.entry)
  return {
    rows: rows.map((r) => r.node),
    kept,
    /** What Merge takes: the ticked ones but the kept one. */
    toMerge: () => ticked().filter((e) => e.id !== keep),
    /** What Delete takes: the ticked ones, the kept one too when it is ticked. */
    toDelete: ticked,
    /** The question before deleting `chosen`. */
    deleteQuestion: (chosen: Entry[]) => {
      const stays = chosen.some((e) => e.id === keep) ? '' : ` ${quote(titleOf(kept()))} stays as it is.`
      return `Move ${describeEntries(chosen, quote)} to the recycle bin?${stays}`
    },
  }
}

/** Takes a site's `section` off the screen (`container`), saying `none` when it was the last. */
export function leaveSite(container: HTMLElement, section: HTMLElement, none: () => HTMLElement) {
  section.replaceWith(...(container.querySelectorAll('section').length > 1 ? [] : [none()]))
}

/** The text under a similar entry's title. */
export const similarHint = (entry: Entry) => [entry.username, entry.url].filter(Boolean).join(' · ')

/** What a merge gives the kept entry (`preview`), as both apps show it before
 *  asking: the fields, tags, files and icon it gets, and the passkeys it does not. */
export function mergePreviewParts(preview: MergePreview): Node[] {
  const line = (name: string, value: string, hint = '') =>
    el('li', {}, el('span', { className: 'name' }, name), el('span', { className: 'value' }, value), hint ? el('span', { className: 'hint' }, hint) : '')
  const lines = [
    ...preview.fields.map((f) => line(f.name, f.value ?? 'hidden', f.fills ? 'was empty' : 'new field')),
    ...(preview.tags.length ? [line('Tags', preview.tags.join(', '))] : []),
    ...preview.files.map((name) => line('File', name)),
    ...(preview.icon ? [line('Icon', 'from another entry')] : []),
  ]
  const body = lines.length ? el('ul', { className: 'merge-preview' }, ...lines) : el('p', { className: 'muted' }, 'Nothing it does not have already.')
  // An entry holds one passkey: another one stays in its entry, in the recycle bin.
  const left = preview.passkeysLeft.map((title) =>
    el('p', { className: 'merge-warning' }, `The passkey of "${title}" is not carried over: this entry has its own. It stays in "${title}", in the recycle bin.`))
  return [body, ...left]
}
