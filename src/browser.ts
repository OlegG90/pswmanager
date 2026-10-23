import { api, type BrowserAssociate, type BrowserPasskeyGet, type BrowserPasskeyRegister, type BrowserUpdate, type EntryChoice } from './api'
import { titleOf } from './entry-text'
import { ask, askText, choose } from './modal'

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

/** How a passkey's entry is named in a list. */
const label = (c: EntryChoice) => (c.username ? `${titleOf(c)} (${c.username})` : titleOf(c))

/** A page asks to sign in with a passkey (#285): choosing one is the user's
 *  verification, as in KeePassXC; Cancel refuses. */
export async function answerPasskeyGet({ id, site, choices }: BrowserPasskeyGet) {
  const i = await choose(`Sign in to ${site} with a passkey:`, choices.map(label))
  await api.browserAnswer(id, i === null ? null : choices[i].id)
}

/** A page asks to make a passkey: in a new entry, or in one of the site's
 *  entries that has none; Cancel refuses. */
export async function answerPasskeyRegister({ id, site, user, entries }: BrowserPasskeyRegister) {
  const i = await choose(`Save a passkey for ${user || 'this account'} on ${site} in:`, ['A new entry', ...entries.map(label)])
  await api.browserAnswer(id, i === null ? null : { entry: i === 0 ? null : entries[i - 1].id })
}
