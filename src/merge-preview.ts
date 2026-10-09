import type { Entry, MergePreview } from './api'
import { el } from './dom'
import { titleOf } from './entry-text'

/** `"Mail"`, or `3 entries`. */
export const describeEntries = (entries: Entry[]) => (entries.length === 1 ? `"${titleOf(entries[0])}"` : `${entries.length} entries`)

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
