import { api, type Listing } from './api'
import { button, el } from './dom'
import { ask, dialog } from './modal'

/** What the import is for, before the file is picked (#238). */
const ABOUT =
  'Choose a file another password manager exported: CSV (Google Password Manager, Chrome, Edge, Firefox, ' +
  'Bitwarden, 1Password, KeePass) or Bitwarden’s JSON, which brings passkeys too. Everything in it is added ' +
  'as new entries in a group of their own; nothing here is changed or replaced. Such a file holds every ' +
  'password in clear text: delete it once the import is done (the app offers to).'

/**
 * Tools › Import from a file…: says what it does, lets the user pick the file,
 * imports it, shows what came over and what did not, and offers to delete the
 * file, which holds every password in clear text. `imported` gets the new
 * listing and the message to show.
 */
export async function importFromFile(imported: (listing: Listing, message: string) => void) {
  if (!(await ask(ABOUT, 'Choose the file'))) return
  const result = await api.importFile()
  if (!result) return
  const { file, group, added, skipped, listing } = result
  imported(listing, added ? `${added} ${added === 1 ? 'entry' : 'entries'} imported into “${group}”` : 'Nothing was imported')
  const deleting = await dialog(
    `${file} holds every password in clear text. Delete it now that its entries are here?`,
    false,
    (answer) => ({
      body: skipped.length
        ? [el('p', {}, `Not brought over (${skipped.length}):`),
            el('ul', { className: 'skipped' }, ...skipped.map((s) => el('li', {}, el('b', {}, s.title || 'Untitled'), `: ${s.why}`)))]
        : [],
      buttons: [button('Delete the file', `Delete ${file}`, () => answer(true), 'danger')],
    }),
    'Keep it',
  )
  if (deleting) await api.deleteImportedFile()
}
