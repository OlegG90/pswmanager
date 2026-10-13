import type { ExportFormat, Exported } from './api'
import { el } from './dom'

/** What both apps' *Export to a file* says alike (docs/spec.md, *Export to a
 *  file*, #240); each asks and answers in its own way. */

/** The formats, each with what it carries. */
export const EXPORT_FORMATS: [ExportFormat, string, string][] = [
  ['csv', 'CSV', 'logins only, which browsers, Google Password Manager, Bitwarden and 1Password read; no passkeys'],
  ['cxf', 'CXF JSON', 'everything, passkeys too, in the new Credential Exchange format; few managers read it yet'],
]

/** Before the format is chosen. */
export function fileExportAbout(): Node[] {
  return [
    el('p', {}, 'Write the entries in use to a file another password manager imports:'),
    el('ul', { className: 'formats' }, ...EXPORT_FORMATS.map(([, name, what]) => el('li', {}, el('b', {}, name), ` — ${what}`))),
    el('p', { className: 'muted' }, 'The file holds every password in clear text: keep it safe, and delete it once it is imported.'),
  ]
}

/** After the file was written: where, how many, and what it lacks. */
export function exportedReport({ file, entries, leftOut }: Exported): Node[] {
  return [
    el('p', {}, `${entries} ${entries === 1 ? 'entry' : 'entries'} written to ${file}.`),
    ...(leftOut.length ? [el('p', {}, 'Not in the file:'), el('ul', { className: 'skipped' }, ...leftOut.map((line) => el('li', {}, line)))] : []),
    el('p', { className: 'muted' }, 'It holds every password in clear text: delete it once it is imported.'),
  ]
}
