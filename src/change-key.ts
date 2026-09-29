import { api, type Status } from './api'
import { el } from './dom'
import { busyButton, enterPresses, errorLine } from './form'
import { keyFileChoice, keyProblem, newPassword } from './key-fields'
import { ask, dialog } from './modal'

/**
 * Asks for the current master password and the new master password and / or
 * key file, in a modal dialog, and changes them. Resolves with the new status,
 * or null when the user cancelled. `keyFile` is the key file used now.
 */
export async function changeMasterKey(keyFile: string | null): Promise<Status | null> {
  const current = el('input', { type: 'password', ariaLabel: 'Current master password' })
  const error = errorLine()
  // A wrong current password is said until it is typed again.
  error.hideOnInput(current)
  const typed = newPassword()
  const keyChoice = keyFileChoice(keyFile, error.show, true)

  const status = await dialog<Status | null>('Change the master password and / or key file.', null, (answer) => {
    const change = busyButton('Change', 'Save the database with the new key', async () => {
      error.hide()
      const problem = keyProblem(typed, keyChoice.value())
      if (problem) return error.show(problem)
      const keyFileOnly = typed.password.value ? '' : ' The database will then have no master password: only the key file opens it.'
      const sure = await ask('Other devices, Keepass2Android too, will need the new key. The old one still opens the .bak files and ' +
        `the store's version history.${keyFileOnly}`, 'Change key')
      if (sure) answer(await api.changeMasterKey(current.value, typed.password.value, keyChoice.value()))
    }, error, 'primary')
    enterPresses(change, current, typed.password, typed.repeat)
    return {
      body: [
        el('label', { className: 'field' }, el('span', {}, 'Current master password'), current),
        el('label', { className: 'field' }, el('span', {}, 'New master password'), typed.password),
        el('label', { className: 'field' }, el('span', {}, 'Repeat it'), typed.repeat),
        typed.strength,
        keyChoice.row,
        el('p', { className: 'muted' }, 'Leave the new password empty for a key file alone.'),
        error.line,
      ],
      buttons: [change],
      focus: current,
    }
  }, 'Cancel', 'form')
  current.value = ''
  typed.clear()
  return status
}
