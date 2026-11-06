import type { ImportKind } from './api'
import { el } from './dom'

/** What both apps' *Import from a file* says alike (docs/spec.md, *Import
 *  from a file*); each asks and answers in its own way. */

/** What an import brought, in a sentence (without the full stop). */
export const importedMessage = (added: number, tag: string) =>
  added ? `${added} ${added === 1 ? 'entry' : 'entries'} imported, tagged “${tag}”` : 'Nothing was imported'

/** The tool that tidies up the groups older imports made (#335), as both apps name it. */
export const TIDY_LABEL = 'Tidy up imported folders'
export const TIDY_HINT = 'Move the entries of older imports out of their folders, keeping the folders as tags'

/** What the tool asks before it moves the entries. */
export const tidyQuestion = (entries: number) =>
  `Move ${entries} ${entries === 1 ? 'entry' : 'entries'} out of the folders older imports made? The folders become tags, and the empty folders go to the trash.`

/** What did not come over, and why; nothing when all did. */
export function skippedList(skipped: { title: string; why: string }[]): Node[] {
  return skipped.length
    ? [el('p', {}, `Not brought over (${skipped.length}):`),
        el('ul', { className: 'skipped' }, ...skipped.map((s) => el('li', {}, el('b', {}, s.title || 'Untitled'), `: ${s.why}`)))]
    : []
}

/** The files an import reads, each with where it comes from. */
export const IMPORT_SOURCES: [ImportKind, string, string][] = [
  ['csv', 'CSV', 'Google Password Manager, Chrome, Edge, Firefox, Bitwarden, 1Password, KeePass (no passkeys)'],
  ['json', 'JSON', 'Bitwarden (passkeys too), or a Credential Exchange (CXF) file'],
  ['aegis', 'Aegis', 'the authenticator’s export, plain or encrypted (its password is asked for)'],
]

/** What follows the sources: where the entries go, and that the file is best deleted. */
export const IMPORT_TAGGED = 'Its entries are added as new ones with a tag of their own.'
export const IMPORT_WARNING = 'Delete the file after the import: it holds every password in clear text.'

/** Before the file is picked: the formats, where the entries go, and that
 *  the file is best deleted. */
export function fileImportAbout(): Node[] {
  return [
    el('p', {}, 'Choose a file another password manager exported:'),
    el('ul', { className: 'formats' }, ...IMPORT_SOURCES.map(([, name, what]) => el('li', {}, el('b', {}, name), ` — ${what}`))),
    el('p', {}, IMPORT_TAGGED),
    el('p', { className: 'muted' }, IMPORT_WARNING),
  ]
}
