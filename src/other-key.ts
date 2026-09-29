import { api, type Status } from './api'
import { button, el } from './dom'
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
  const error = el('p', { className: 'error', role: 'alert', hidden: true })
  const fail = (message: string) => {
    error.textContent = message
    error.hidden = false
  }
  password.addEventListener('input', () => (error.hidden = true))
  const keyChoice = keyFileChoice(null, fail)

  const status = await dialog<Status | null>(
    `${where[0].toUpperCase()}${where.slice(1)} opens with another master password or key file: another device changed it. ` +
      'Enter it to read that copy. If that change is the newer one, this database takes the new key from now on.',
    null,
    (answer) => {
      const read = button('Read', 'Read the copy with this key', async () => {
        read.disabled = true
        try {
          answer(await api.enterOtherKey(password.value, keyChoice.value()))
        } catch (e) {
          fail(String(e))
        } finally {
          read.disabled = false
        }
      }, 'primary')
      password.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
          e.preventDefault()
          read.click()
        }
      })
      return {
        body: [el('label', { className: 'field' }, el('span', {}, 'Its master password'), password), keyChoice.row, error],
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
