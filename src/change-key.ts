import { api, type Status } from './api'
import { button, el } from './dom'
import { keyFileChoice, keyProblem, newPassword } from './key-fields'
import { ask, dialog } from './modal'

/**
 * Asks for the current master password and the new master password and / or
 * key file, in a modal dialog, and changes them. Resolves with the new status,
 * or null when the user cancelled. `keyFile` is the key file used now.
 */
export async function changeMasterKey(keyFile: string | null): Promise<Status | null> {
  const current = el('input', { type: 'password', ariaLabel: 'Current master password' })
  const error = el('p', { className: 'error', role: 'alert', hidden: true })
  const fail = (message: string) => {
    error.textContent = message
    error.hidden = false
  }
  // A wrong current password is said until it is typed again.
  current.addEventListener('input', () => (error.hidden = true))
  const typed = newPassword()
  const keyChoice = keyFileChoice(keyFile, fail, true)

  const status = await dialog<Status | null>('Change the master password and / or key file.', null, (answer) => {
    const change = button('Change', 'Save the database with the new key', async () => {
      error.hidden = true
      const problem = keyProblem(typed, keyChoice.value())
      if (problem) return fail(problem)
      const keyFileOnly = typed.password.value ? '' : ' The database will then have no master password: only the key file opens it.'
      const sure = await ask('Other devices, Keepass2Android too, will need the new key. The old one still opens the .bak files and ' +
        `the store's version history.${keyFileOnly}`, 'Change key')
      if (!sure) return
      change.disabled = true
      try {
        answer(await api.changeMasterKey(current.value, typed.password.value, keyChoice.value()))
      } catch (e) {
        fail(String(e))
      } finally {
        change.disabled = false
      }
    }, 'primary')
    for (const input of [current, typed.password, typed.repeat]) {
      input.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
          e.preventDefault()
          change.click()
        }
      })
    }
    return {
      body: [
        el('label', { className: 'field' }, el('span', {}, 'Current master password'), current),
        el('label', { className: 'field' }, el('span', {}, 'New master password'), typed.password),
        el('label', { className: 'field' }, el('span', {}, 'Repeat it'), typed.repeat),
        typed.strength,
        keyChoice.row,
        el('p', { className: 'muted' }, 'Leave the new password empty for a key file alone.'),
        error,
      ],
      buttons: [change],
      focus: current,
    }
  }, 'Cancel', 'form')
  current.value = ''
  typed.clear()
  return status
}
