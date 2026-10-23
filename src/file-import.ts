import { api, type FileImported, type Listing } from './api'
import { busyButton, button, el, enterPresses, errorLine } from './dom'
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
  // The page's own element: an import that ends after the page was left is
  // still applied, but draws nothing.
  const page = el('div')
  container.replaceChildren(page)
  const header = () => el('header', {}, el('h1', {}, 'Import from a file'))

  const start = () => {
    page.replaceChildren(header(), el('section', { className: 'tool' }, ...fileImportAbout(),
      el('div', { className: 'tool-actions' },
        busyButton('Choose the file…', 'Choose the file to import', choose, options.fail, 'primary'))))
    page.querySelector('button')?.focus()
  }

  const choose = async () => {
    const step = await api.importFile()
    if (!step) return
    if (step.step === 'done') done(step)
    else askPassword(step.file)
  }

  // An encrypted Aegis export (#282): the file waits in the backend for its
  // password; a wrong one can be typed again.
  const askPassword = (file: string) => {
    const password = el('input', { type: 'password', autocomplete: 'off' })
    const error = errorLine()
    error.hideOnInput(password)
    const importIt = busyButton('Import', `Import ${file}`, async () => {
      try {
        const step = await api.importFileWithPassword(password.value)
        if (step.step === 'done') done(step)
      } catch (e) {
        error.show(String(e))
        password.select()
      }
    }, options.fail, 'primary')
    enterPresses(importIt, password)
    page.replaceChildren(header(), el('section', { className: 'tool' },
      el('p', {}, `${file} is encrypted. Its password, the one it was exported with:`),
      password, error.line,
      el('div', { className: 'tool-actions' }, importIt,
        button('Cancel', 'Do not import this file', () => void api.cancelImport().then(start, (e) => options.fail(String(e)))))))
    password.focus()
  }

  const done = (result: FileImported) => {
    const { file, group, added, skipped, listing } = result
    const message = added ? `${added} ${added === 1 ? 'entry' : 'entries'} imported into “${group}”` : 'Nothing was imported'
    options.imported(listing, message)
    if (!page.isConnected) return
    // The file goes only when the user says so here: it may be wanted again.
    const actions = el('div', { className: 'tool-actions' })
    actions.append(
      busyButton('Delete the file', `Delete ${file}`, async () => {
        await api.deleteImportedFile()
        actions.replaceChildren(el('p', { className: 'muted' }, `${file} was deleted.`), button('Import another file', 'Choose another file to import', start))
      }, options.fail, 'danger'),
      button('Import another file', 'Keep this file and choose another one', start),
    )
    page.replaceChildren(header(), el('section', { className: 'tool' },
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
