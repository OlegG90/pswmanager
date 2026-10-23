import { el } from './dom'

/** What both apps' *Import from a file* says alike (docs/spec.md, *Import
 *  from a file*); each asks and answers in its own way. */

/** Before the file is picked: the formats, where the entries go, and that
 *  the file is best deleted. */
/** What an import brought, in a sentence (without the full stop). */
export const importedMessage = (added: number, group: string) =>
  added ? `${added} ${added === 1 ? 'entry' : 'entries'} imported into “${group}”` : 'Nothing was imported'

/** What did not come over, and why; nothing when all did. */
export function skippedList(skipped: { title: string; why: string }[]): Node[] {
  return skipped.length
    ? [el('p', {}, `Not brought over (${skipped.length}):`),
        el('ul', { className: 'skipped' }, ...skipped.map((s) => el('li', {}, el('b', {}, s.title || 'Untitled'), `: ${s.why}`)))]
    : []
}

export function fileImportAbout(): Node[] {
  return [
    el('p', {}, 'Choose a file another password manager exported:'),
    el('ul', { className: 'formats' },
      el('li', {}, el('b', {}, 'CSV'), ' — Google Password Manager, Chrome, Edge, Firefox, Bitwarden, 1Password, KeePass (no passkeys)'),
      el('li', {}, el('b', {}, 'JSON'), ' — Bitwarden (passkeys too)'),
      el('li', {}, el('b', {}, 'Aegis'), ' — the authenticator’s export, plain or encrypted (its password is asked for)')),
    el('p', {}, 'Its entries are added as new ones in a group of their own.'),
    el('p', { className: 'muted' }, 'Delete the file after the import: it holds every password in clear text.'),
  ]
}
