import { api, type GeneratorOptions } from './api'
import { button, el } from './dom'
import { chip, fillRange } from './editor-parts'
import { statFigure } from './health'
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

/** Fills `container` with Home (#351): the database, its password health,
 *  the quick actions and a password generator for a given need. */
export function renderHome(container: HTMLElement, options: HomeOptions) {
  // Each figure opens its list while it has any. Password health is worked
  // out in the backend; its counts come in when it answers.
  const figure = (count: number, label: string, severity: string, open: () => void) =>
    statFigure(count, label, severity, { title: `Show: ${label.toLowerCase()}`, onclick: open })
  const figures = (count: (group: HealthGroup) => number) => [
    ...HEALTH_GROUPS.map(({ group, label, severity }) => figure(count(group), label, severity, () => options.health(group))),
    figure(options.expired, 'Expired or expiring soon', 'warn', options.showExpired),
  ]
  const stats = el('div', { className: 'stats home-stats' }, ...figures(() => 0))
  api.passwordHealth().then((health) => {
    if (stats.isConnected) stats.replaceChildren(...figures((group) => health[group].length)) // else Home was left meanwhile
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
    fillRange(length)
    kinds.hidden = need !== 'custom'
    kinds.replaceChildren(kind('upper', 'A–Z'), kind('lower', 'a–z'), kind('digits', '0–9'), kind('symbols', '!#$'), kind('excludeLookAlikes', 'No look-alikes'))
    void regenerate()
  }
  length.addEventListener('input', () => {
    chosen.set(need, { ...settings(), length: Number(length.value) })
    lengthValue.textContent = length.value
    fillRange(length)
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
