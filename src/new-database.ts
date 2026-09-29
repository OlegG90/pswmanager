import { api, type Status } from './api'
import { button, el } from './dom'
import { busyButton, enterPresses, errorLine } from './form'
import { keyFileChoice, keyProblem, newPassword } from './key-fields'

/**
 * The form that creates a new database: where its file goes, the master
 * password (twice, with its strength) and / or a key file. Resolves with the
 * new status, or null when the user went back.
 */
export function createDatabase(container: HTMLElement): Promise<Status | null> {
  return new Promise((resolve) => {
    let file: string | null = null
    const where = el('span', { className: 'path muted' }, 'Choose where the file goes')
    const error = errorLine()
    const typed = newPassword()
    const keyChoice = keyFileChoice(null, error.show)

    const chooseFile = button('Choose…', 'Where the new database file goes', async () => {
      error.hide()
      const picked = await api.pickNewFile('Passwords.kdbx', true).catch((e) => (error.show(String(e)), null))
      if (picked) {
        file = picked
        where.textContent = picked
        where.classList.remove('muted')
      }
    })
    // Esc goes back to the add screen, while this form is up.
    const escape = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      e.stopPropagation()
      done(null)
    }
    const done = (status: Status | null) => {
      container.removeEventListener('keydown', escape, true)
      typed.clear()
      resolve(status)
    }

    const create = busyButton('Create', 'Create the database', async () => {
      error.hide()
      if (!file) return error.show('Choose where the file goes')
      const problem = keyProblem(typed, keyChoice.value())
      if (problem) return error.show(problem)
      done(await api.createDatabase(file, typed.password.value, keyChoice.value()))
    }, error, 'primary')

    container.replaceChildren(
      el('div', { className: 'intro' },
        el('span', { className: 'kicker' }, 'New database'),
        el('h1', {}, 'Create a database'),
        el('p', { className: 'muted' },
          'An empty KeePass (KDBX 4) file, which KeePassXC and Keepass2Android open too. Keep the master password safe: nothing can recover it.')),
      el('div', { className: 'file-row' }, where, chooseFile),
      el('label', { className: 'field' }, el('span', {}, 'Master password'), typed.password),
      el('label', { className: 'field' }, el('span', {}, 'Repeat it'), typed.repeat),
      typed.strength,
      keyChoice.row,
      error.line,
      el('div', { className: 'footer' }, el('span'),
        el('div', { className: 'buttons' }, button('Back', 'Back (Esc)', () => done(null)), create)),
    )
    // Enter in a password field creates.
    enterPresses(create, typed.password, typed.repeat)
    container.addEventListener('keydown', escape, true)
    chooseFile.focus()
  })
}
