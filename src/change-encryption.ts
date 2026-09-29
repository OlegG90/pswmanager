import { api, type Cipher, type DatabaseSettings, type Encryption, type Kdf } from './api'
import { button, el } from './dom'
import { formatSize } from './entry-text'
import { ask, dialog } from './modal'

const MIB = 2 ** 20
/** Above this, or an unlock slower than SLOW_MS, a phone may struggle. */
const PHONE_MEMORY = 256 * MIB
const SLOW_MS = 2000

const CIPHERS: Record<Cipher, string> = { aes256: 'AES-256', chaCha20: 'ChaCha20', other: 'Twofish (kept)' }
const KDFS: Record<Kdf, string> = { argon2id: 'Argon2id', argon2d: 'Argon2d', aesKdf: 'AES-KDF', other: 'Another (kept)' }
/** What a key derivation starts from when it is chosen anew: KeePassXC's own defaults. */
const ARGON2 = { iterations: 10, memory: 64 * MIB, parallelism: 2 }
const AES_KDF = { iterations: 100_000, memory: 0, parallelism: 0 }

const isArgon2 = (kdf: Kdf) => kdf === 'argon2id' || kdf === 'argon2d'

/** The encryption in a few words: "AES-256 · Argon2id, 64 MB, 10 iterations, 2 threads". */
export function describeEncryption(e: Encryption): string {
  const count = (n: number, one: string) => `${n.toLocaleString('en')} ${one}${n === 1 ? '' : 's'}`
  const kdf = isArgon2(e.kdf)
    ? `${KDFS[e.kdf]}, ${formatSize(e.memory)}, ${count(e.iterations, 'iteration')}, ${count(e.parallelism, 'thread')}`
    : e.kdf === 'aesKdf' ? `AES-KDF, ${count(e.iterations, 'round')}` : KDFS[e.kdf]
  return `${CIPHERS[e.cipher]} · ${kdf}`
}

/** A drop-down of `labels`, those in `offered` (and `value`'s own). */
function pick<T extends string>(label: string, value: T, labels: Record<T, string>, offered: T[]) {
  const shown = offered.includes(value) ? offered : [...offered, value]
  return el('select', { ariaLabel: label }, ...shown.map((v) => new Option(labels[v], v, false, v === value)))
}

function number(label: string, value: number, min: number, max: number) {
  return el('input', { type: 'number', ariaLabel: label, value: String(value), min: String(min), max: String(max), step: '1' })
}

/**
 * Asks for another cipher and / or key derivation in a modal dialog, with a
 * Test that times an unlock on this PC, and saves it. Resolves with the new
 * settings, or null when the user cancelled.
 */
export function changeEncryption(now: Encryption): Promise<DatabaseSettings | null> {
  const cipher = pick('Cipher', now.cipher, CIPHERS, ['aes256', 'chaCha20'])
  const kdf = pick('Key derivation', now.kdf, KDFS, ['argon2id', 'argon2d', 'aesKdf'])
  const iterations = number('Iterations', now.iterations, 1, 1_000_000_000)
  const memory = number('Memory (MiB)', Math.max(1, Math.round(now.memory / MIB)), 1, 4096)
  const threads = number('Threads', Math.max(1, now.parallelism), 1, 64)
  const iterationsLabel = el('span', {})
  const argon2Only = [el('label', { className: 'field' }, el('span', {}, 'Memory (MiB)'), memory),
    el('label', { className: 'field' }, el('span', {}, 'Threads'), threads)]
  const measured = el('p', { className: 'muted' })
  const warning = el('p', { className: 'error', hidden: true }, 'A phone may be slow to unlock it, or run out of memory.')
  const error = el('p', { className: 'error', role: 'alert', hidden: true })
  /** The last Test's time, for these very values; null when not tested. */
  let tested: { for: string; ms: number } | null = null

  const wanted = (): Encryption => ({
    cipher: cipher.value as Cipher,
    kdf: kdf.value as Kdf,
    iterations: Number(iterations.value),
    memory: isArgon2(kdf.value as Kdf) ? Number(memory.value) * MIB : 0,
    parallelism: isArgon2(kdf.value as Kdf) ? Number(threads.value) : 0,
  })
  const slow = () => tested?.for === JSON.stringify(wanted()) && tested.ms > SLOW_MS
  const heavy = () => wanted().memory > PHONE_MEMORY
  const update = () => {
    const argon2 = isArgon2(kdf.value as Kdf)
    iterationsLabel.textContent = kdf.value === 'aesKdf' ? 'Rounds' : 'Iterations'
    for (const field of argon2Only) field.hidden = !argon2
    iterations.disabled = kdf.value === 'other'
    if (tested?.for !== JSON.stringify(wanted())) measured.textContent = ''
    warning.hidden = !heavy() && !slow()
    error.hidden = true
  }
  // A key derivation chosen anew starts from sensible values.
  kdf.addEventListener('change', () => {
    const chosen = kdf.value as Kdf
    // The database's own values when it has the same kind already.
    const start = chosen === 'aesKdf' ? (now.kdf === 'aesKdf' ? now : AES_KDF) : isArgon2(now.kdf) ? now : ARGON2
    iterations.value = String(start.iterations)
    if (isArgon2(chosen)) {
      memory.value = String(Math.max(1, Math.round(start.memory / MIB)))
      threads.value = String(Math.max(1, start.parallelism))
    }
    update()
  })
  for (const field of [cipher, iterations, memory, threads]) field.addEventListener('input', update)
  update()

  return dialog<DatabaseSettings | null>('Change how the database file is encrypted.', null, (answer) => {
    const fail = (message: string) => {
      error.textContent = message
      error.hidden = false
    }
    const test = button('Test', 'Time an unlock with these settings on this PC', async () => {
      test.disabled = true
      measured.textContent = 'Measuring…'
      try {
        const values = wanted()
        const ms = await api.encryptionUnlockTime(values)
        tested = { for: JSON.stringify(values), ms }
        update()
        measured.textContent = `An unlock takes ${(ms / 1000).toFixed(1)} s on this PC.`
      } catch (e) {
        measured.textContent = ''
        fail(String(e))
      } finally {
        test.disabled = false
      }
    })
    const save = button('Change', 'Save the database with this encryption', async () => {
      if ((heavy() || slow()) && !(await ask('A phone may be slow to unlock the database with this, or run out of memory. Change anyway?', 'Change'))) {
        return
      }
      save.disabled = true
      try {
        answer(await api.setEncryption(wanted()))
      } catch (e) {
        fail(String(e))
      } finally {
        save.disabled = false
      }
    }, 'primary')
    return {
      body: [
        el('label', { className: 'field' }, el('span', {}, 'Cipher'), cipher),
        el('label', { className: 'field' }, el('span', {}, 'Key derivation'), kdf),
        el('label', { className: 'field' }, iterationsLabel, iterations),
        ...argon2Only,
        el('div', { className: 'file-row' }, measured, test),
        warning,
        error,
      ],
      buttons: [save],
      focus: cipher,
    }
  }, 'Cancel', 'form')
}
