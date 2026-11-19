import type { Exported } from './api'
import { el } from './dom'

/** What both apps' *Export to a file* says alike (docs/spec.md, *Export to a
 *  file*, #240); each asks and answers in its own way. */

/** What comes before the formats, and after them. */
export const EXPORT_INTRO = 'Write the entries in use to a file another password manager imports:'
export const EXPORT_WARNING = 'The file holds every password in clear text: keep it safe, and delete it once it is imported.'

/** After the file was written: where, how many, and what it lacks. */
export function exportedReport({ file, entries, leftOut }: Exported): Node[] {
  return [
    el('p', {}, `${entries} ${entries === 1 ? 'entry' : 'entries'} written to ${file}.`),
    ...(leftOut.length ? [el('p', {}, 'Not in the file:'), el('ul', { className: 'skipped' }, ...leftOut.map((line) => el('li', {}, line)))] : []),
    el('p', { className: 'muted' }, 'It holds every password in clear text: delete it once it is imported.'),
  ]
}
