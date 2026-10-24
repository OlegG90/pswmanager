import { api, type BrowserAssociate, type BrowserPasskeyGet, type BrowserPasskeyRegister, type BrowserUpdate, type EntryChoice } from './api'
import { titleOf } from './entry-text'
import { ask, askText, choose } from './modal'

/** The dialog open for each question the app waits on. */
const open = new Map<number, HTMLDialogElement>()
/** The questions the app stopped waiting for while their dialogs were open. */
const expired = new Set<number>()

/** Shows question `id`'s dialog (`show` puts up one of modal's, which joins
 *  the page as it opens), so that it can close if the question expires. */
function asking<T>(id: number, show: () => Promise<T>): Promise<T> {
  const answer = show()
  open.set(id, document.body.lastElementChild as HTMLDialogElement)
  return answer.finally(() => open.delete(id))
}

/** The app stopped waiting for question `id` (`browser-expired`): its dialog
 *  closes as cancelled, since a late answer would reach nothing. */
export function expire(id: number) {
  const box = open.get(id)
  if (box) {
    expired.add(id)
    box.dispatchEvent(new Event('cancel'))
  }
}

/** Gives the app the answer to question `id`; one that expired reaches nothing. */
function reply(id: number, answer: Parameters<typeof api.browserAnswer>[1]) {
  expired.delete(id)
  return api.browserAnswer(id, answer)
}

/** A browser extension asks to connect to the open database (#273): the user
 *  names the connection, as KeePassXC asks; a name already in use is
 *  replaced only after asking. Cancel refuses the extension. */
export async function answerAssociate({ id, database, taken }: BrowserAssociate) {
  let name: string | null = null
  while (!expired.has(id)) {
    const typed = (await asking(id, () => askText(`A browser extension asks to connect to “${database}”. Give the connection a name to tell it apart in the settings:`,
      'Browser', 'Connect')))?.trim()
    if (!typed) break
    if (!taken.includes(typed) || await asking(id, () => ask(`There is a connection “${typed}” already. Replace it?`, 'Replace'))) {
      name = typed
      break
    }
  }
  await reply(id, name)
}

/** The extension offers another password for a login the database has:
 *  replaced only when the user says so. */
export async function answerUpdate({ id, title, username, newUsername, site }: BrowserUpdate) {
  const who = newUsername && newUsername !== username ? `, and the user name ${username || '(none)'} with ${newUsername},` : ''
  const yes = await asking(id, () => ask(`Replace the password of “${title || site}”${username ? ` (${username})` : ''}${who} with the one ${site || 'the browser'} offers? The old one goes into the entry's history.`, 'Replace'))
  await reply(id, yes)
}

/** How a passkey's entry is named in a list. */
const label = (c: EntryChoice) => (c.username ? `${titleOf(c)} (${c.username})` : titleOf(c))

/** A page asks to sign in with a passkey (#285): choosing one is the user's
 *  verification, as in KeePassXC; Cancel refuses. */
export async function answerPasskeyGet({ id, site, choices }: BrowserPasskeyGet) {
  const i = await asking(id, () => choose(`Sign in to ${site} with a passkey:`, choices.map(label)))
  await reply(id, i === null ? null : choices[i].id)
}

/** A page asks to make a passkey: in a new entry, or in one of the site's
 *  entries that has none; Cancel refuses. */
export async function answerPasskeyRegister({ id, site, user, entries }: BrowserPasskeyRegister) {
  const i = await asking(id, () => choose(`Save a passkey for ${user || 'this account'} on ${site} in:`, ['A new entry', ...entries.map(label)]))
  await reply(id, i === null ? null : { entry: i === 0 ? null : entries[i - 1].id })
}
