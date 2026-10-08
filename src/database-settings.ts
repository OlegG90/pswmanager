/** What both apps' *Database* settings share (`docs/spec.md`, *Database
 *  settings*): the history limits offered, and the encryption form with its
 *  Test that times an unlock on this device. */
import type { Cipher, Encryption, Kdf } from './api'
import { busyButton, el } from './dom'
import { formatSize } from './entry-text'

export type Choice<T = number> = [value: T, label: string]

export const versions = (n: number) => (n === 0 ? 'None' : n === 1 ? '1 version' : `${n} versions`)
export const HISTORY_ITEMS: Choice[] = [0, 3, 5, 10, 20, 50, 100].map((n): Choice => [n, versions(n)])
export const HISTORY_SIZE: Choice[] = [1, 2, 4, 6, 10, 20, 64].map((m): Choice => [m * 2 ** 20, formatSize(m * 2 ** 20)])

/** `choices` with the file's own value among them: another client may have
 *  set no limit (-1), or a limit these do not offer. */
export function withValue(choices: Choice[], value: number, label: (value: number) => string): Choice[] {
  return choices.some(([v]) => v === value) ? choices : [...choices, [value, value < 0 ? 'No limit' : label(value)]]
}

/** What lowering the history limits removes, as the confirmation says it. */
export const versionsGoing = (going: number) =>
  `${going === 1 ? '1 older version' : `${going} older versions`} will be removed from the entries' history, in this file and in its synced copies.`

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

export interface EncryptionForm {
  /** The labelled fields, the Test row and the warning, in order. */
  fields: HTMLElement[]
  wanted: () => Encryption
  /** The values shown are the database's own. */
  unchanged: () => boolean
  /** Times an unlock with the values shown, unless timed already; null when
   *  it failed, which `onError` has said. */
  measure: () => Promise<number | null>
  /** Too heavy for a phone: too much memory, or an unlock this slow. */
  heavy: (ms: number) => boolean
}

/**
 * The form for another cipher and / or key derivation, starting from `now`.
 * `unlockTime` times an unlock with some values on this device, which
 * `device` names ("this PC", "this phone"); `onEdit` is called as the values
 * change (to hide an error said about the old ones).
 */
export function encryptionForm(
  now: Encryption,
  unlockTime: (e: Encryption) => Promise<number>,
  onError: (message: string) => void,
  device: string,
  onEdit: () => void = () => {},
): EncryptionForm {
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
  const heavy = (ms: number) => wanted().memory > PHONE_MEMORY || ms > SLOW_MS
  const update = () => {
    iterationsLabel.textContent = chosen() === 'aes' ? 'Rounds' : 'Iterations'
    iterations.max = String(chosen() === 'aes' ? 1_000_000_000 : 100)
    iterations.disabled = chosen() === 'other'
    for (const field of argon2Only) field.hidden = !isArgon2(chosen())
    if (testedMs() === null) measured.textContent = ''
    warning.hidden = !heavy(testedMs() ?? 0)
    onEdit()
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

  const measure = async (): Promise<number | null> => {
    const known = testedMs()
    if (known !== null) return known
    measured.textContent = 'Measuring…'
    try {
      const values = wanted()
      tested = { values, ms: await unlockTime(values) }
      update()
      measured.textContent = `An unlock takes ${(tested.ms / 1000).toFixed(1)} s on ${device}.`
      return tested.ms
    } catch (e) {
      measured.textContent = ''
      onError(String(e))
      return null
    }
  }
  const test = busyButton('Test', `Time an unlock with these settings on ${device}`, measure, onError)
  return {
    fields: [
      el('label', { className: 'field' }, el('span', {}, 'Cipher'), cipher),
      el('label', { className: 'field' }, el('span', {}, 'Key derivation'), kdf),
      el('label', { className: 'field' }, iterationsLabel, iterations),
      ...argon2Only,
      el('div', { className: 'file-row' }, measured, test),
      warning,
    ],
    wanted,
    unchanged: () => same(wanted(), now),
    measure,
    heavy,
  }
}
