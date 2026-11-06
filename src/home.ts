import { api, type GeneratorOptions } from './api'
import { button, el } from './dom'
import { chip } from './editor-parts'
import { HEALTH_GROUPS, type HealthGroup } from './health-parts'
import { NEEDS, type Need } from './home-parts'

export interface HomeOptions {
  /** The database's name, and where it is kept (a local file, or synced with…). */
  title: string
  place: string
  /** Entries in use. */
  entries: number
  /** Entries expired or expiring soon. */
  expired: number
  /** Opens Password health with this list alone. */
  health: (only: HealthGroup) => void
  /** Shows the Expired group. */
  showExpired: () => void
  newEntry: () => void
  /** Reads a TOTP QR code into a new entry. */
  scanQr: () => void
  importFile: () => void
  /** A new entry with this password. */
  newWithPassword: (password: string) => void
  /** Copies the password, the clipboard cleared as for a copied field. */
  copy: (password: string) => Promise<void>
  fail: (message: string) => void
}

/** The need chosen last and its settings, kept for the session as the
 *  editor's generator keeps its own. */
let need: Need = 'website'
const chosen = new Map<Need, GeneratorOptions>(NEEDS.map((n) => [n.need, n.options]))

/** A figure on Home: its count, and a click to its list while it has any. */
function figure(count: number, label: string, severity: string, open: () => void): HTMLElement {
  const parts = [el('span', { className: 'count' }, String(count)), el('span', { className: 'hint' }, label)]
  return count
    ? el('button', { type: 'button', className: `stat ${severity}`, title: `Show: ${label.toLowerCase()}`, onclick: open }, ...parts)
    : el('div', { className: 'stat' }, ...parts)
}

/** Fills `container` with Home (#351): the database, its password health,
 *  the quick actions and a password generator for a given need. */
export function renderHome(container: HTMLElement, options: HomeOptions) {
  // Password health is worked out in the backend; its figures come in when it answers.
  const stats = el('div', { className: 'stats home-stats' },
    ...HEALTH_GROUPS.map(({ label }) => figure(0, label, '', () => {})),
    figure(options.expired, 'Expired or expiring soon', 'warn', options.showExpired))
  api.passwordHealth().then((health) => {
    if (!stats.isConnected) return // Home was left meanwhile
    stats.replaceChildren(
      ...HEALTH_GROUPS.map(({ group, label, severity }) => figure(health[group].length, label, severity, () => options.health(group))),
      figure(options.expired, 'Expired or expiring soon', 'warn', options.showExpired))
  }, (e) => stats.isConnected && options.fail(String(e)))

  const actions = el('div', { className: 'tool-actions' },
    button('New entry', 'New entry (Ctrl+N)', options.newEntry, 'primary'),
    button('Scan a QR code…', 'A new entry with the TOTP of a QR code: a snip on the clipboard, or a picture', options.scanQr),
    button('Import from a file…', 'Add the entries another password manager exported', options.importFile))

  container.replaceChildren(
    el('header', {}, el('h1', {}, options.title)),
    el('p', { className: 'muted intro' }, `${options.place} · ${options.entries} ${options.entries === 1 ? 'entry' : 'entries'}`),
    stats,
    el('section', {}, el('h3', {}, 'Quick actions'), actions),
    el('section', {}, el('h3', {}, 'Generate a password'), generator(options)),
  )
}

/** The generator: the need, the password made for it, its length, and for
 *  Custom the kinds of character. */
function generator(options: HomeOptions): HTMLElement {
  const preview = el('code', { className: 'preview' })
  const hint = el('p', { className: 'hint' })
  const length = el('input', { type: 'range', ariaLabel: 'Length' })
  const lengthValue = el('span', { className: 'length-value' })
  const kinds = el('div', { className: 'options' })
  const card = el('div', { className: 'generator home-generator' })

  const settings = () => chosen.get(need)!
  // Only the password asked for last is shown: a quick run of changes to the
  // length may be answered out of order.
  let asked = 0
  const regenerate = async () => {
    const ask = ++asked
    try {
      const password = await api.generatePassword(settings())
      if (ask === asked) preview.textContent = password
    } catch (e) {
      if (ask !== asked) return
      preview.textContent = ''
      options.fail(String(e))
    }
  }
  // How far the track is filled, for the CSS: a range input cannot tell it.
  const fill = () => {
    const [min, max] = [Number(length.min), Number(length.max)]
    length.style.setProperty('--fill', `${((Number(length.value) - min) / (max - min)) * 100}%`)
  }
  const kind = (key: keyof GeneratorOptions, label: string) =>
    chip(label, label, settings()[key] as boolean, (on) => {
      chosen.set(need, { ...settings(), [key]: on })
      void regenerate()
    })

  const needs = el('div', { className: 'needs', role: 'group', ariaLabel: 'Made for' })
  const show = () => {
    const preset = NEEDS.find((n) => n.need === need)!
    needs.replaceChildren(...NEEDS.map((n) => chip(n.label, n.hint, n.need === need, () => {
      need = n.need
      show()
    })))
    hint.textContent = preset.hint
    ;[length.min, length.max] = preset.lengths.map(String)
    length.value = String(settings().length)
    lengthValue.textContent = length.value
    fill()
    kinds.hidden = need !== 'custom'
    kinds.replaceChildren(kind('upper', 'A–Z'), kind('lower', 'a–z'), kind('digits', '0–9'), kind('symbols', '!#$'), kind('excludeLookAlikes', 'No look-alikes'))
    void regenerate()
  }
  length.addEventListener('input', () => {
    chosen.set(need, { ...settings(), length: Number(length.value) })
    lengthValue.textContent = length.value
    fill()
    void regenerate()
  })

  const password = () => preview.textContent ?? ''
  card.append(
    needs, hint,
    el('div', { className: 'result' }, preview,
      button('Again', 'Another password', () => void regenerate(), 'ghost'),
      button('Copy', 'Copy the password', () => void options.copy(password()).catch((e) => options.fail(String(e))), 'primary'),
      button('New entry with it', 'A new entry with this password', () => options.newWithPassword(password()))),
    el('div', { className: 'length' }, el('span', { className: 'label' }, 'Length'), length, lengthValue),
    kinds,
  )
  show()
  return card
}
