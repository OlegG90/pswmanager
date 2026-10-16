import { api, type Listing } from './api'
import { busyButton, button, el } from './dom'
import { fileImportAbout } from './file-import-parts'

export interface ImportOptions {
  /** Gets the new listing and the message to show. */
  imported: (listing: Listing, message: string) => void
  fail: (message: string) => void
}

/**
 * Tools › Import from a file, beside the sidebar: says what it does, lets the
 * user pick the file, imports it, shows what came over and what did not, and
 * offers to delete the file, which holds every password in clear text.
 */
export function renderImport(container: HTMLElement, options: ImportOptions) {
  const header = () => el('header', {}, el('h1', {}, 'Import from a file'))

  const start = () => {
    container.replaceChildren(header(), el('section', { className: 'tool' }, ...fileImportAbout(),
      el('div', { className: 'tool-actions' },
        busyButton('Choose the file…', 'Choose the file to import', choose, options.fail, 'primary'))))
    container.querySelector('button')?.focus()
  }

  const choose = async () => {
    const result = await api.importFile()
    if (!result) return
    const { file, group, added, skipped, listing } = result
    const message = added ? `${added} ${added === 1 ? 'entry' : 'entries'} imported into “${group}”` : 'Nothing was imported'
    options.imported(listing, message)
    // Kept unless deleted here: leaving the panel keeps it, as before.
    const actions = el('div', { className: 'tool-actions' })
    actions.append(
      busyButton('Delete the file', `Delete ${file}`, async () => {
        await api.deleteImportedFile()
        actions.replaceChildren(el('p', { className: 'muted' }, `${file} was deleted.`), button('Import another file', 'Choose another file to import', start))
      }, options.fail, 'danger'),
      button('Import another file', 'Keep this file and choose another one', start),
    )
    container.replaceChildren(header(), el('section', { className: 'tool' },
      el('p', {}, `${message}.`),
      ...(skipped.length
        ? [el('p', {}, `Not brought over (${skipped.length}):`),
            el('ul', { className: 'skipped' }, ...skipped.map((s) => el('li', {}, el('b', {}, s.title || 'Untitled'), `: ${s.why}`)))]
        : []),
      el('p', {}, `${file} holds every password in clear text. Delete it now that its entries are here?`),
      actions))
    actions.querySelector('button')?.focus()
  }

  start()
}
