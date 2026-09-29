import { api } from './api'
import { button, el } from './dom'

const STRENGTH = ['Very weak', 'Weak', 'Fair', 'Strong', 'Very strong']

/** A new master password typed twice, with its strength shown as it is typed. */
export function newPassword() {
  const password = el('input', { type: 'password', ariaLabel: 'Master password' })
  const repeat = el('input', { type: 'password', ariaLabel: 'Repeat the master password' })
  const strength = el('p', { className: 'muted' })
  let timer: number | undefined
  password.addEventListener('input', () => {
    clearTimeout(timer)
    timer = window.setTimeout(async () => {
      const typed = password.value
      if (!typed) return (strength.textContent = '')
      const { score, crackTime } = await api.passwordStrength(typed)
      // An answer to an older value is dropped.
      if (password.value === typed) strength.textContent = `${STRENGTH[score]} · cracked in ${crackTime}`
    }, 200)
  })
  return {
    password,
    repeat,
    strength,
    /** Both typed the same. */
    matches: () => password.value === repeat.value,
    clear: () => {
      password.value = ''
      repeat.value = ''
      strength.textContent = ''
    },
  }
}

/**
 * The key file a database is to use: none, an existing file, or (with
 * `offerNew`) a new one the app makes. `fail` reports a dialog that failed.
 */
export function keyFileChoice(initial: string | null, fail: (message: string) => void, offerNew = false) {
  let keyFile = initial
  const shown = el('span', { className: 'path muted' })
  const show = () => {
    shown.textContent = keyFile ?? 'No key file'
    shown.title = keyFile ?? ''
    remove.hidden = !keyFile
  }
  const take = (pick: () => Promise<string | null>) => async () => {
    const picked = await pick().catch((e) => (fail(String(e)), null))
    if (picked) {
      keyFile = picked
      show()
    }
  }
  const remove = button('Remove', 'No key file', () => {
    keyFile = null
    show()
  })
  const choose = button('Key file…', 'An existing key file', take(api.pickKeyFilePath))
  const make = button('New…', 'Make a new key file where you choose; keep it apart from the database', take(api.createKeyFile))
  make.hidden = !offerNew
  show()
  return { row: el('div', { className: 'file-row' }, shown, choose, make, remove), value: () => keyFile }
}
