import { api, type BrowserAssociate, type BrowserUpdate } from './api'
import { ask, askText } from './modal'

/** A browser extension asks to connect to the open database (#273): the user
 *  names the connection, as KeePassXC asks; a name already in use is
 *  replaced only after asking. Cancel refuses the extension. */
export async function answerAssociate({ id, database, taken }: BrowserAssociate) {
  let name: string | null = null
  for (;;) {
    const typed = (await askText(`A browser extension asks to connect to “${database}”. Give the connection a name to tell it apart in the settings:`,
      'Browser', 'Connect'))?.trim()
    if (!typed) break
    if (!taken.includes(typed) || await ask(`There is a connection “${typed}” already. Replace it?`, 'Replace')) {
      name = typed
      break
    }
  }
  await api.browserAnswer(id, name)
}

/** The extension offers another password for a login the database has:
 *  replaced only when the user says so. */
export async function answerUpdate({ id, title, username, newUsername, site }: BrowserUpdate) {
  const who = newUsername && newUsername !== username ? `, and the user name ${username || '(none)'} with ${newUsername},` : ''
  const yes = await ask(`Replace the password of “${title || site}”${username ? ` (${username})` : ''}${who} with the one ${site || 'the browser'} offers? The old one goes into the entry's history.`, 'Replace')
  await api.browserAnswer(id, yes)
}
