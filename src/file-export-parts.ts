import type { Exported, Module } from './api'
import { el } from './dom'

/** What both apps' *Export to a file* says alike (docs/spec.md, *Export to a
 *  file*, #240); each asks and answers in its own way. */

/** What comes before the formats, and after them. */
export const EXPORT_INTRO = 'Write the entries in use to a file another password manager imports:'
export const EXPORT_WARNING = 'The file holds every password in clear text: keep it safe, and delete it once it is imported.'

/** Before the format is chosen: the exporters turned on, from the core's
 *  registry (#412), each with what it carries. */
export function fileExportAbout(exporters: Module[]): Node[] {
  return [
    el('p', {}, EXPORT_INTRO),
    el('ul', { className: 'formats' }, ...exporters.map((m) => el('li', {}, el('b', {}, m.label), ` — ${m.hint}`))),
    el('p', { className: 'muted' }, EXPORT_WARNING),
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
