import { api, type FileImported, type GauthProgress, type Listing } from './api'
import { busyButton, button, el } from './dom'
import { gauthAbout, importLabel, progressText } from './gauth-import-parts'
import { SIMILAR_AFTER_IMPORT } from './similar-parts'

export interface GauthOptions {
  /** Gets the new listing and the message to show. */
  imported: (listing: Listing, message: string) => void
  fail: (message: string) => void
}

/**
 * Tools › Import from Google Authenticator (#281), beside the sidebar: the
 * export's QR codes are read one by one (a snip on the clipboard, or a
 * picture), then their accounts are imported. Leaving the page lets the codes
 * go (main.ts calls api.gauthCancel).
 */
export function renderGauthImport(container: HTMLElement, options: GauthOptions) {
  const page = el('div')
  container.replaceChildren(page)
  const header = () => el('header', {}, el('h1', {}, 'Import from Google Authenticator'))

  const draw = (progress: GauthProgress | null) => {
    const actions = el('div', { className: 'tool-actions' },
      busyButton(progress ? 'Read another code…' : 'Read a code…', 'Read a QR code: a snip on the clipboard, or a picture', read, options.fail, progress ? '' : 'primary'))
    if (progress?.accounts) actions.prepend(busyButton(importLabel(progress), 'Import the accounts read', importIt, options.fail, 'primary'))
    page.replaceChildren(header(), el('section', { className: 'tool' },
      ...gauthAbout('Snip each code (Win+Shift+S) and press Read a code, or save it as a picture and pick it.'),
      ...(progress ? [el('p', { className: 'progress' }, progressText(progress))] : []),
      actions))
    page.querySelector<HTMLButtonElement>('.tool-actions button')?.focus()
  }

  const read = async () => {
    const progress = await api.gauthRead()
    if (progress && page.isConnected) draw(progress)
  }

  const importIt = async () => done(await api.gauthImport())

  const done = ({ group, added, skipped, listing }: FileImported) => {
    const message = added ? `${added} ${added === 1 ? 'entry' : 'entries'} imported into “${group}”` : 'Nothing was imported'
    options.imported(listing, message)
    if (!page.isConnected) return
    page.replaceChildren(header(), el('section', { className: 'tool' },
      el('p', {}, `${message}.`),
      ...(added ? [el('p', { className: 'muted' }, SIMILAR_AFTER_IMPORT)] : []),
      ...(skipped.length
        ? [el('p', {}, `Not brought over (${skipped.length}):`),
            el('ul', { className: 'skipped' }, ...skipped.map((s) => el('li', {}, el('b', {}, s.title || 'Untitled'), `: ${s.why}`)))]
        : []),
      el('div', { className: 'tool-actions' }, button('Import another export', 'Read the codes of another export', () => draw(null)))))
  }

  draw(null)
}
