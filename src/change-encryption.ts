import { api, type DatabaseSettings, type Encryption } from './api'
import { encryptionForm, HEAVY_QUESTION } from './database-settings'
import { busyButton, errorLine } from './dom'
import { ask, dialog } from './modal'

export { describeEncryption } from './database-settings'

/**
 * Asks for another cipher and / or key derivation in a modal dialog, with a
 * Test that times an unlock on this PC, and saves it. Resolves with the new
 * settings, or null when the user cancelled.
 */
export function changeEncryption(now: Encryption): Promise<DatabaseSettings | null> {
  const error = errorLine()
  const form = encryptionForm(now, api.encryptionUnlockTime, error.show, 'this PC', error.hide)

  return dialog<DatabaseSettings | null>('Change how the database file is encrypted.', null, (answer) => {
    // Timed before saving too, so a slow choice is always said.
    const save = busyButton('Change', 'Save the database with this encryption', async () => {
      const ms = form.unchanged() ? 0 : await form.measure()
      if (ms === null) return
      if (form.heavy(ms) && !(await ask(HEAVY_QUESTION, 'Change'))) {
        return
      }
      answer(await api.setEncryption(form.wanted()))
    }, error.show, 'primary')
    return {
      body: [...form.fields, error.line],
      buttons: [save],
      focus: form.fields[0].querySelector('select')!,
    }
  }, 'Cancel', 'form')
}
