import { api, type Status } from './api'
import { busyButton, el, enterPresses, errorLine } from './dom'
import { keyFileChoice } from './key-fields'
import { dialog } from './modal'

/**
 * Asks, in a modal dialog, for the master password and / or key file of a
 * copy of the database that another device changed the key of (`where`: "the
 * remote file", "the file on this PC"), and reads both copies with it.
 * Resolves with the new status, or null when the user cancelled.
 */
export async function enterOtherKey(where: string): Promise<Status | null> {
  const password = el('input', { type: 'password', ariaLabel: 'Its master password' })
  const error = errorLine()
  error.hideOnInput(password)
  const keyChoice = keyFileChoice(null, error.show)

  const status = await dialog<Status | null>(
    `${where[0].toUpperCase()}${where.slice(1)} opens with another master password or key file: another device changed it. ` +
      'Enter it to read that copy. If that change is the newer one, this database takes the new key from now on.',
    null,
    (answer) => {
      const read = busyButton('Read', 'Read the copy with this key',
        async () => answer(await api.enterOtherKey(password.value, keyChoice.value())), error.show, 'primary')
      enterPresses(read, password)
      return {
        body: [el('label', { className: 'field' }, el('span', {}, 'Its master password'), password), keyChoice.row, error.line],
        buttons: [read],
        focus: password,
      }
    },
    'Not now',
    'form',
  )
  password.value = ''
  return status
}
