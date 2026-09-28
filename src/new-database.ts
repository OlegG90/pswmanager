import { api, type Status } from './api'
import { button, el } from './dom'

const STRENGTH = ['Very weak', 'Weak', 'Fair', 'Strong', 'Very strong']

/**
 * The form that creates a new database: where its file goes, the master
 * password (twice, with its strength) and / or a key file. Resolves with the
 * new status, or null when the user went back.
 */
export function createDatabase(container: HTMLElement): Promise<Status | null> {
  return new Promise((resolve) => {
    let file: string | null = null
    let keyFile: string | null = null
    const where = el('span', { className: 'path muted' }, 'Choose where the file goes')
    const keyShown = el('span', { className: 'path muted' }, 'No key file')
    const password = el('input', { type: 'password', ariaLabel: 'Master password' })
    const repeat = el('input', { type: 'password', ariaLabel: 'Repeat the master password' })
    const strength = el('p', { className: 'muted' })
    const error = el('p', { className: 'error', role: 'alert', hidden: true })
    const fail = (message: string) => {
      error.textContent = message
      error.hidden = false
    }

    const chooseFile = button('Choose…', 'Where the new database file goes', async () => {
      const picked = await api.pickNewFile('Passwords.kdbx').catch((e) => (fail(String(e)), null))
      if (picked) {
        file = picked
        where.textContent = picked
        where.classList.remove('muted')
      }
    })
    const removeKey = button('Remove', 'No key file', () => {
      keyFile = null
      keyShown.textContent = 'No key file'
      removeKey.hidden = true
    })
    removeKey.hidden = true
    const chooseKey = button('Key file…', 'An existing key file (optional)', async () => {
      const picked = await api.pickKeyFilePath().catch((e) => (fail(String(e)), null))
      if (picked) {
        keyFile = picked
        keyShown.textContent = picked
        removeKey.hidden = false
      }
    })

    let timer: number | undefined
    password.addEventListener('input', () => {
      clearTimeout(timer)
      timer = window.setTimeout(async () => {
        if (!password.value) return (strength.textContent = '')
        const { score, crackTime } = await api.passwordStrength(password.value)
        strength.textContent = `${STRENGTH[score]} · cracked in ${crackTime}`
      }, 200)
    })

    const create = button('Create', 'Create the database', async () => {
      error.hidden = true
      if (!file) return fail('Choose where the file goes')
      if (!password.value && !keyFile) return fail('Give a master password, a key file, or both')
      if (password.value !== repeat.value) return fail('The two passwords differ')
      create.disabled = true
      try {
        const status = await api.createDatabase(file, password.value, keyFile)
        password.value = ''
        repeat.value = ''
        resolve(status)
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
      el('div', { className: 'file-row' }, keyShown, chooseKey, removeKey),
      error,
      el('div', { className: 'footer' }, el('span'),
        el('div', { className: 'buttons' }, button('Back', 'Back (Esc)', () => resolve(null)), create)),
    )
    chooseFile.focus()
  })
}
