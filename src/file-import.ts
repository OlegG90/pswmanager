import { api, type Listing } from './api'
import { button, el } from './dom'
import { fileImportAbout } from './file-import-parts'
import { dialog } from './modal'

/**
 * Tools › Import from a file…: says what it does, lets the user pick the file,
 * imports it, shows what came over and what did not, and offers to delete the
 * file, which holds every password in clear text. `imported` gets the new
 * listing and the message to show.
 */
export async function importFromFile(imported: (listing: Listing, message: string) => void) {
  const choosing = await dialog('Import from a file', false, (answer) => ({
    body: fileImportAbout(),
    buttons: [button('Choose the file', 'Choose the file to import', () => answer(true), 'primary')],
  }))
  if (!choosing) return
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
