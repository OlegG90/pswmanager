import { api, type Status } from './api'
import { button, el } from './dom'
import { keyFileChoice, newPassword } from './key-fields'

/**
 * The form that creates a new database: where its file goes, the master
 * password (twice, with its strength) and / or a key file. Resolves with the
 * new status, or null when the user went back.
 */
export function createDatabase(container: HTMLElement): Promise<Status | null> {
  return new Promise((resolve) => {
    let file: string | null = null
    const where = el('span', { className: 'path muted' }, 'Choose where the file goes')
    const error = el('p', { className: 'error', role: 'alert', hidden: true })
    const fail = (message: string) => {
      error.textContent = message
      error.hidden = false
    }
    const typed = newPassword()
    const { password, repeat, strength } = typed
    const keyFile = keyFileChoice(null, fail)

    const chooseFile = button('Choose…', 'Where the new database file goes', async () => {
      error.hidden = true
      const picked = await api.pickNewFile('Passwords.kdbx', true).catch((e) => (fail(String(e)), null))
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

    const create = button('Create', 'Create the database', async () => {
      error.hidden = true
      if (!file) return fail('Choose where the file goes')
      if (!password.value && !keyFile.value()) return fail('Give a master password, a key file, or both')
      if (!typed.matches()) return fail('The two passwords differ')
      create.disabled = true
      try {
        done(await api.createDatabase(file, password.value, keyFile.value()))
      } catch (e) {
        fail(String(e))
      } finally {
        create.disabled = false
      }
    }, 'primary')

    container.replaceChildren(
      el('div', { className: 'intro' },
        el('span', { className: 'kicker' }, 'New database'),
        el('h1', {}, 'Create a database'),
        el('p', { className: 'muted' },
          'An empty KeePass (KDBX 4) file, which KeePassXC and Keepass2Android open too. Keep the master password safe: nothing can recover it.')),
      el('div', { className: 'file-row' }, where, chooseFile),
      el('label', { className: 'field' }, el('span', {}, 'Master password'), password),
      el('label', { className: 'field' }, el('span', {}, 'Repeat it'), repeat),
      strength,
      keyFile.row,
      error,
      el('div', { className: 'footer' }, el('span'),
        el('div', { className: 'buttons' }, button('Back', 'Back (Esc)', () => done(null)), create)),
    )
    // Enter in a password field creates.
    for (const input of [password, repeat]) {
      input.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') {
          e.preventDefault()
          create.click()
        }
      })
    }
    container.addEventListener('keydown', escape, true)
    chooseFile.focus()
  })
}
