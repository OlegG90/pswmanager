import { api, type FileImported, type ImportKind, type Listing } from './api'
import { busyButton, button, choiceList, el, enterPresses, errorLine } from './dom'
import { IMPORT_SOURCES, IMPORT_TAGGED, IMPORT_WARNING, importedMessage, skippedList } from './file-import-parts'
import { SIMILAR_AFTER_IMPORT } from './similar-parts'

export interface ImportOptions {
  /** Gets the new listing and the message to show. */
  imported: (listing: Listing, message: string) => void
  /** Opens *Import from Google Authenticator*, its row in the list. */
  authenticator: () => void
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

  // The sources as a list, each its button with what it reads (#354). A
  // button only sets the picker's filter, as a file is read by what it holds;
  // Google Authenticator's export, QR codes rather than a file, has its own page.
  const start = () => {
    page.replaceChildren(header(), el('section', { className: 'choose' },
      el('p', {}, 'Add the entries another password manager or authenticator exported:'),
      choiceList([
        ...IMPORT_SOURCES.map(([kind, name, what]): [HTMLButtonElement, string] =>
          [busyButton(name, `Choose the ${name} file to import`, () => choose(kind), options.fail), what]),
        [button('Google Authenticator', 'Read the QR codes of its export', options.authenticator), 'its Export accounts QR codes, read from a snip or a picture'],
      ]),
      el('p', {}, IMPORT_TAGGED),
      el('p', { className: 'muted' }, IMPORT_WARNING)))
    page.querySelector('button')?.focus()
  }

  const choose = async (kind: ImportKind) => {
    const step = await api.importFile(kind)
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
    const { file, tag, added, skipped, listing } = result
    const message = importedMessage(added, tag)
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
      ...(added ? [el('p', { className: 'muted' }, SIMILAR_AFTER_IMPORT)] : []),
      ...skippedList(skipped),
      el('p', {}, `${file} holds every password in clear text. Delete it now that its entries are here?`),
      actions))
    actions.querySelector('button')?.focus()
  }

  start()
}
