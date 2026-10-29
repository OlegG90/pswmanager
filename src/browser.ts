import { api, type BrowserAssociate, type BrowserPasskeyGet, type BrowserPasskeyRegister, type BrowserUpdate, type EntryChoice } from './api'
import { titleOf } from './entry-text'
import { ask, askText, choose } from './modal'

/** The questions the app waits on, each aborted when it stops waiting. The
 * wait itself (120 s) and the expiry decision live in Rust (questions.rs);
 * this map only closes the dialogs. */
const waiting = new Map<number, AbortController>()

/** The signal that closes question `id`'s dialogs once it expires. */
function asked(id: number): AbortSignal {
  const controller = new AbortController()
  waiting.set(id, controller)
  return controller.signal
}

/** The app stopped waiting for question `id` (`browser-expired`): its dialog
 *  closes as cancelled, since a late answer would reach nothing. */
export function expire(id: number) {
  waiting.get(id)?.abort()
}

/** Gives the app the answer to question `id`; one that expired reaches nothing. */
function reply(id: number, answer: Parameters<typeof api.browserAnswer>[1]) {
  waiting.delete(id)
  return api.browserAnswer(id, answer)
}

/** A browser extension asks to connect to the open database (#273): the user
 *  names the connection, as KeePassXC asks; a name already in use is
 *  replaced only after asking. Cancel refuses the extension. */
export async function answerAssociate({ id, database, taken }: BrowserAssociate) {
  const signal = asked(id)
  let name: string | null = null
  while (!signal.aborted) {
    const typed = (await askText(`A browser extension asks to connect to “${database}”. Give the connection a name to tell it apart in the settings:`,
      'Browser', 'Connect', undefined, [], signal))?.trim()
    if (!typed) break
    if (!taken.includes(typed) || await ask(`There is a connection “${typed}” already. Replace it?`, 'Replace', 'Cancel', signal)) {
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
  const yes = await ask(`Replace the password of “${title || site}”${username ? ` (${username})` : ''}${who} with the one ${site || 'the browser'} offers? The old one goes into the entry's history.`, 'Replace', 'Cancel', asked(id))
  await reply(id, yes)
}

/** How a passkey's entry is named in a list. */
const label = (c: EntryChoice) => (c.username ? `${titleOf(c)} (${c.username})` : titleOf(c))

/** A page asks to sign in with a passkey (#285): choosing one is the user's
 *  verification, as in KeePassXC; Cancel refuses. */
export async function answerPasskeyGet({ id, site, choices }: BrowserPasskeyGet) {
  const i = await choose(`Sign in to ${site} with a passkey:`, choices.map(label), asked(id))
  await reply(id, i === null ? null : choices[i].id)
}

/** A page asks to make a passkey: in a new entry, or in one of the site's
 *  entries that has none; Cancel refuses. */
export async function answerPasskeyRegister({ id, site, user, entries }: BrowserPasskeyRegister) {
  const i = await choose(`Save a passkey for ${user || 'this account'} on ${site} in:`, ['A new entry', ...entries.map(label)], asked(id))
  await reply(id, i === null ? null : { entry: i === 0 ? null : entries[i - 1].id })
}
