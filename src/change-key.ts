import { api, type Status } from './api'
import { button, el } from './dom'
import { keyFileChoice, newPassword } from './key-fields'

/**
 * Asks for the current master password and the new master password and / or
 * key file, in a modal dialog, and changes them. Resolves with the new status,
 * or null when the user cancelled. `keyFile` is the key file used now.
 */
export function changeMasterKey(keyFile: string | null): Promise<Status | null> {
  return new Promise((resolve) => {
    const box = el('dialog', { className: 'modal change-key' })
    const current = el('input', { type: 'password', ariaLabel: 'Current master password' })
    const error = el('p', { className: 'error', role: 'alert', hidden: true })
    const fail = (message: string) => {
      error.textContent = message
      error.hidden = false
    }
    // A wrong current password is said until it is typed again.
    current.addEventListener('input', () => (error.hidden = true))
    const typed = newPassword()
    const newKeyFile = keyFileChoice(keyFile, fail, true)
    const done = (status: Status | null) => {
      current.value = ''
      typed.clear()
      box.close()
      box.remove()
      resolve(status)
    }

    const change = button('Change', 'Save the database with the new key', async () => {
      error.hidden = true
      if (!typed.password.value && !newKeyFile.value()) return fail('Give a master password, a key file, or both')
      if (!typed.matches()) return fail('The two new passwords differ')
      change.disabled = true
      try {
        done(await api.changeMasterKey(current.value, typed.password.value, newKeyFile.value()))
      } catch (e) {
        fail(String(e))
      } finally {
        change.disabled = false
      }
    }, 'primary')

    box.append(
      el('p', {}, 'Change the master password and / or key file.'),
      el('label', { className: 'field' }, el('span', {}, 'Current master password'), current),
      el('label', { className: 'field' }, el('span', {}, 'New master password'), typed.password),
      el('label', { className: 'field' }, el('span', {}, 'Repeat it'), typed.repeat),
      typed.strength,
      newKeyFile.row,
      el('p', { className: 'muted' },
        'Other devices, Keepass2Android too, will need the new key. The old one still opens the .bak files and the store\'s version history.'),
      error,
      el('div', { className: 'buttons' }, change, button('Cancel', 'Cancel (Esc)', () => done(null))),
    )
    // Esc closes a modal dialog by itself: cancel, nothing changed.
    box.addEventListener('cancel', (e) => {
      e.preventDefault()
      done(null)
    })
    for (const input of [current, typed.password, typed.repeat]) {
      input.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
          e.preventDefault()
          change.click()
        }
      })
    }
    document.body.append(box)
    box.showModal()
    current.focus()
  })
}
