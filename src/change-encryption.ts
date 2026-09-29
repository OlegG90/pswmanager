import { api, type Cipher, type DatabaseSettings, type Encryption, type Kdf } from './api'
import { busyButton, el, errorLine } from './dom'
import { formatSize } from './entry-text'
import { ask, dialog } from './modal'

const MB = 2 ** 20
/** Above this, or an unlock slower than SLOW_MS, a phone may struggle. */
const PHONE_MEMORY = 256 * MB
const SLOW_MS = 2000

const CIPHERS: Record<Cipher, string> = { aes256: 'AES-256', chaCha20: 'ChaCha20', other: 'Twofish (kept)' }
const KDFS: Record<Kdf, string> = { argon2id: 'Argon2id', argon2d: 'Argon2d', aes: 'AES-KDF', other: 'Another (kept)' }
/** Where a kind of key derivation the database does not have yet starts: KeePassXC's defaults. */
const ARGON2 = { iterations: 10, memory: 64 * MB, parallelism: 2 }
const AES_KDF = { iterations: 100_000, memory: 0, parallelism: 0 }

const isArgon2 = (kdf: Kdf) => kdf === 'argon2id' || kdf === 'argon2d'
const same = (a: Encryption, b: Encryption) => (Object.keys(a) as (keyof Encryption)[]).every((k) => a[k] === b[k])

/** The encryption in a few words: "AES-256 · Argon2id, 64 MB, 10 iterations, 2 threads". */
export function describeEncryption(e: Encryption): string {
  const count = (n: number, one: string) => `${n.toLocaleString('en')} ${one}${n === 1 ? '' : 's'}`
  const kdf = isArgon2(e.kdf)
    ? `${KDFS[e.kdf]}, ${formatSize(e.memory)}, ${count(e.iterations, 'iteration')}, ${count(e.parallelism, 'thread')}`
    : e.kdf === 'aes' ? `AES-KDF, ${count(e.iterations, 'round')}` : KDFS[e.kdf]
  return `${CIPHERS[e.cipher]} · ${kdf}`
}

/** A drop-down of `labels`, those in `offered` (and `value`'s own). */
function pick<T extends string>(label: string, value: T, labels: Record<T, string>, offered: T[]) {
  const shown = offered.includes(value) ? offered : [...offered, value]
  return el('select', { ariaLabel: label }, ...shown.map((v) => new Option(labels[v], v, false, v === value)))
}

/** A whole number from 1; its value and maximum are set as the key derivation is. */
function numberInput(label: string) {
  return el('input', { type: 'number', ariaLabel: label, min: '1', step: '1' })
}

/**
 * Asks for another cipher and / or key derivation in a modal dialog, with a
 * Test that times an unlock on this PC, and saves it. Resolves with the new
 * settings, or null when the user cancelled.
 */
export function changeEncryption(now: Encryption): Promise<DatabaseSettings | null> {
  const cipher = pick('Cipher', now.cipher, CIPHERS, ['aes256', 'chaCha20'])
  const kdf = pick('Key derivation', now.kdf, KDFS, ['argon2id', 'argon2d', 'aes'])
  const chosen = () => kdf.value as Kdf
  const iterations = numberInput('Iterations')
  iterations.value = String(now.iterations)
  const memory = numberInput('Memory (MB)')
  memory.max = '4096'
  const threads = numberInput('Threads')
  threads.max = '64'
  /** Argon2's memory and threads from `from`, at least 1 of each. */
  const showArgon2 = (from: Encryption | typeof ARGON2) => {
    memory.value = String(Math.max(1, Math.round(from.memory / MB)))
    threads.value = String(Math.max(1, from.parallelism))
  }
  showArgon2(now)
  const iterationsLabel = el('span', {})
  const argon2Only = [el('label', { className: 'field' }, el('span', {}, 'Memory (MB)'), memory),
    el('label', { className: 'field' }, el('span', {}, 'Threads'), threads)]
  const measured = el('p', { className: 'muted' })
  const warning = el('p', { className: 'error', hidden: true }, 'A phone may be slow to unlock it, or run out of memory.')
  const error = errorLine()
  /** The last Test: the values it timed and how long an unlock took. */
  let tested: { values: Encryption; ms: number } | null = null

  const wanted = (): Encryption => ({
    cipher: cipher.value as Cipher,
    kdf: chosen(),
    iterations: Number(iterations.value),
    memory: isArgon2(chosen()) ? Number(memory.value) * MB : 0,
    parallelism: isArgon2(chosen()) ? Number(threads.value) : 0,
  })
  const testedMs = () => (tested && same(tested.values, wanted()) ? tested.ms : null)
  const heavy = (ms: number | null) => wanted().memory > PHONE_MEMORY || (ms ?? 0) > SLOW_MS
  const update = () => {
    iterationsLabel.textContent = chosen() === 'aes' ? 'Rounds' : 'Iterations'
    iterations.max = String(chosen() === 'aes' ? 1_000_000_000 : 100)
    iterations.disabled = chosen() === 'other'
    for (const field of argon2Only) field.hidden = !isArgon2(chosen())
    if (testedMs() === null) measured.textContent = ''
    warning.hidden = !heavy(testedMs())
    error.hide()
  }
  // Another kind of key derivation starts from the database's own values when
  // it has that kind already, otherwise from KeePassXC's defaults.
  kdf.addEventListener('change', () => {
    const start = chosen() === 'aes' ? (now.kdf === 'aes' ? now : AES_KDF) : isArgon2(now.kdf) ? now : ARGON2
    iterations.value = String(start.iterations)
    if (isArgon2(chosen())) showArgon2(start)
    update()
  })
  for (const field of [cipher, iterations, memory, threads]) field.addEventListener('input', update)
  update()

  return dialog<DatabaseSettings | null>('Change how the database file is encrypted.', null, (answer) => {
    /** Times an unlock with the values shown, unless done already; null when it failed. */
    const measure = async (): Promise<number | null> => {
      const known = testedMs()
      if (known !== null) return known
      measured.textContent = 'Measuring…'
      try {
        const values = wanted()
        tested = { values, ms: await api.encryptionUnlockTime(values) }
        update()
        measured.textContent = `An unlock takes ${(tested.ms / 1000).toFixed(1)} s on this PC.`
        return tested.ms
      } catch (e) {
        measured.textContent = ''
        error.show(String(e))
        return null
      }
    }
    const test = busyButton('Test', 'Time an unlock with these settings on this PC', measure, error.show)
    // Timed before saving too, so a slow choice is always said.
    const save = busyButton('Change', 'Save the database with this encryption', async () => {
      const ms = same(wanted(), now) ? 0 : await measure()
      if (ms === null) return
      if (heavy(ms) && !(await ask('A phone may be slow to unlock the database with this, or run out of memory. Change anyway?', 'Change'))) {
        return
      }
      answer(await api.setEncryption(wanted()))
    }, error.show, 'primary')
    return {
      body: [
        el('label', { className: 'field' }, el('span', {}, 'Cipher'), cipher),
        el('label', { className: 'field' }, el('span', {}, 'Key derivation'), kdf),
        el('label', { className: 'field' }, iterationsLabel, iterations),
        ...argon2Only,
        el('div', { className: 'file-row' }, measured, test),
        warning,
        error.line,
      ],
      buttons: [save],
      focus: cipher,
    }
  }, 'Cancel', 'form')
}
