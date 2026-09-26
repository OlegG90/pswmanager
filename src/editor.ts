import { api, type EntryData, type FieldData, type GeneratorOptions, type Saved } from './api'
import { button, el } from './dom'
import { formatGroup, formatTags, keep, parseGroup, parseTags, singleLine, textareaLines } from './entry-text'

export interface EditorOptions {
  /** The entry to change; null creates one. */
  id: string | null
  /** Where a new entry goes. */
  group: string[]
  onSaved: (saved: Saved) => void
  onClose: () => void
}

const EMPTY: EntryData = { title: '', username: '', password: '', url: '', notes: '', otp: '', tags: [], group: [], fields: [] }
const STRENGTH = ['Very weak', 'Weak', 'Fair', 'Strong', 'Very strong']

/** Kept for the session, so the generator opens as it was last used. */
let generatorOptions: GeneratorOptions = {
  length: 20,
  upper: true,
  lower: true,
  digits: true,
  symbols: true,
  excludeLookAlikes: true,
}

/** The open editor, from the moment it starts loading. */
let active: { save: () => void; close: () => void } | null = null

export const isEditing = () => active !== null

/** Keys while the editor is open: Ctrl+S saves, Esc cancels. True when handled. */
export function editorKey(e: KeyboardEvent): boolean {
  if (!active) return false
  if (e.ctrlKey && e.code === 'KeyS') {
    e.preventDefault()
    active.save()
    return true
  }
  if (e.key === 'Escape') {
    e.preventDefault()
    active.close()
    return true
  }
  return false
}

export function closeEditor() {
  active = null
}

const input = (value: string, props: object = {}) => el('input', { value, spellcheck: false, ...props })

function showHide(target: HTMLInputElement): HTMLButtonElement {
  const toggle = button('Show', 'Show / hide', () => {
    const hidden = target.type === 'password'
    target.type = hidden ? 'text' : 'password'
    toggle.textContent = hidden ? 'Hide' : 'Show'
  })
  return toggle
}

/** The password generator panel; `use` receives the chosen password. */
function generatorPanel(use: (password: string) => void, onError: (message: string) => void) {
  const preview = el('code', { className: 'preview' })
  const regenerate = async () => {
    try {
      preview.textContent = await api.generatePassword(generatorOptions)
    } catch (e) {
      preview.textContent = ''
      onError(String(e))
    }
  }
  const option = (key: keyof GeneratorOptions, label: string) => {
    const box = el('input', { type: 'checkbox', checked: generatorOptions[key] })
    box.addEventListener('change', () => {
      generatorOptions = { ...generatorOptions, [key]: box.checked }
      regenerate()
    })
    return el('label', {}, box, label)
  }
  // 8–64: the range the backend allows.
  const length = el('input', { type: 'number', min: 8, max: 64, value: String(generatorOptions.length) })
  length.addEventListener('input', () => {
    generatorOptions = { ...generatorOptions, length: Number(length.value) || 20 }
    regenerate()
  })
  const panel = el(
    'div',
    { className: 'generator', hidden: true },
    el('div', { className: 'options' }, el('label', {}, 'Length ', length), option('upper', 'A–Z'), option('lower', 'a–z'),
      option('digits', '0–9'), option('symbols', '!#$'), option('excludeLookAlikes', 'No look-alikes')),
    el('div', { className: 'options' }, preview, button('Again', 'Another password', regenerate),
      button('Use', 'Use this password', () => {
        use(preview.textContent ?? '')
        panel.hidden = true
      }, 'primary')),
  )
  const open = button('Generate…', 'Generate a password', () => {
    panel.hidden = !panel.hidden
    if (!panel.hidden) regenerate()
  })
  return { panel, open }
}

/** The field each row started from, to keep values the form only reformatted. */
const originals = new WeakMap<HTMLElement, FieldData>()

/** One additional field: name, value (a textarea keeps line breaks an <input>
 *  would drop; a protected value is masked by CSS), protected, remove. */
function fieldRow(field: FieldData = { name: '', value: '', protected: false }): HTMLDivElement {
  const name = input(field.name, { placeholder: 'Name', className: 'name' })
  const value = el('textarea', { value: field.value, rows: 1, spellcheck: false, className: 'value' })
  const fit = () => {
    value.style.height = 'auto'
    value.style.height = `${value.scrollHeight + 2}px`
  }
  value.addEventListener('input', fit)
  requestAnimationFrame(fit)
  const protect = el('input', { type: 'checkbox', checked: field.protected, title: 'Protected' })
  const mask = () => value.classList.toggle('masked', protect.checked)
  protect.addEventListener('change', mask)
  mask()
  const row = el('div', { className: 'field-row' }, name, value, el('label', {}, protect, 'Protected'),
    button('×', 'Remove', () => row.remove()))
  originals.set(row, field)
  return row
}

function readField(row: HTMLElement): FieldData {
  const [name, protect] = row.querySelectorAll('input')
  const value = row.querySelector('textarea')!
  const original = originals.get(row)!
  return {
    name: keep(original.name, name.value.trim(), (n) => singleLine(n).trim()),
    value: keep(original.value, value.value, textareaLines),
    protected: protect.checked,
  }
}

/** Shows the editor in `container`. */
export async function openEditor(container: HTMLElement, options: EditorOptions) {
  // Counts as open while loading, so a second Ctrl+E or a click in the list waits.
  const loading = { save: () => {}, close: () => {} }
  active = loading
  let data: EntryData
  let groups: string[][]
  try {
    ;[data, groups] = await Promise.all([
      options.id ? api.editEntry(options.id) : Promise.resolve({ ...EMPTY, group: options.group }),
      api.groupPaths(),
    ])
  } catch (e) {
    if (active === loading) active = null
    throw e
  }
  if (active !== loading) return // locked while loading

  const title = input(data.title, { placeholder: 'Title' })
  const username = input(data.username)
  const password = input(data.password, { type: 'password', className: 'secret' })
  const url = input(data.url, { placeholder: 'https://' })
  const otp = input(data.otp, { type: 'password', className: 'secret', placeholder: 'Secret or otpauth:// URI' })
  const group = input(formatGroup(data.group), { placeholder: 'Top level' })
  group.setAttribute('list', 'group-list')
  const tags = input(formatTags(data.tags), { placeholder: 'Separated by commas' })
  const notes = el('textarea', { value: data.notes, rows: 4, spellcheck: false })
  const error = el('p', { className: 'error', hidden: true })
  const strength = el('div', { className: 'strength' })
  const fieldList = el('div', { className: 'fields' }, ...data.fields.map((f) => fieldRow(f)))

  const showError = (message: string) => {
    error.textContent = message
    error.hidden = false
  }

  let strengthTimer: number | undefined
  const showStrength = () => {
    clearTimeout(strengthTimer)
    strengthTimer = window.setTimeout(async () => {
      if (!password.value) {
        strength.replaceChildren()
        return
      }
      const { score, crackTime } = await api.passwordStrength(password.value)
      strength.dataset.score = String(score)
      strength.replaceChildren(el('span', { className: 'bar' }), `${STRENGTH[score]} · cracked in ${crackTime}`)
    }, 200)
  }
  password.addEventListener('input', showStrength)
  showStrength()

  const generator = generatorPanel((chosen) => {
    password.value = chosen
    showStrength()
  }, showError)

  const trimmedLine = (text: string) => singleLine(text).trim()
  const collect = (): EntryData => ({
    title: keep(data.title, title.value, singleLine),
    username: keep(data.username, username.value, singleLine),
    password: keep(data.password, password.value, singleLine),
    url: keep(data.url, url.value.trim(), trimmedLine),
    notes: keep(data.notes, notes.value, textareaLines),
    otp: keep(data.otp, otp.value.trim(), trimmedLine),
    tags: keep(data.tags, parseTags(tags.value), (t) => parseTags(formatTags(t))),
    group: keep(data.group, parseGroup(group.value), (g) => parseGroup(formatGroup(g))),
    fields: [...fieldList.querySelectorAll<HTMLDivElement>('.field-row')].map(readField),
  })

  let saving = false
  const save = async () => {
    if (saving) return
    saving = true
    try {
      const saved = await api.saveEntry(options.id, collect())
      // Locking while the save ran closed this editor: the vault is gone.
      if (active !== self) return
      active = null
      options.onSaved(saved)
    } catch (e) {
      if (active === self) showError(String(e))
    } finally {
      saving = false
    }
  }
  let warned = false
  const close = () => {
    // Esc also closes pop-ups (the group list, an input method), so changes
    // are only thrown away on a second Esc.
    if (!warned && JSON.stringify(collect()) !== untouched) {
      warned = true
      showError('Unsaved changes: press Esc again to discard them.')
      return
    }
    active = null
    options.onClose()
  }
  const self = { save, close }
  active = self

  const row = (label: string, ...controls: Node[]) =>
    el('div', { className: 'edit-row' }, el('span', { className: 'label' }, label), el('div', { className: 'controls' }, ...controls))
  const form = el(
    'form',
    { className: 'editor', onsubmit: (e: SubmitEvent) => (e.preventDefault(), save()) },
    el('h2', {}, options.id ? 'Edit entry' : 'New entry'),
    row('Title', title),
    row('User name', username),
    row('Password', password, showHide(password), generator.open),
    row('', strength),
    generator.panel,
    row('URL', url),
    row('TOTP', otp, showHide(otp)),
    row('Group', group, el('datalist', { id: 'group-list' }, ...groups.map((g) => new Option(formatGroup(g))))),
    row('Tags', tags),
    row('Notes', notes),
    el('h3', {}, 'Fields'),
    fieldList,
    button('Add field', 'Add a field', () => fieldList.append(fieldRow())),
    error,
    el('div', { className: 'buttons' },
      el('button', { type: 'submit', className: 'primary', textContent: 'Save', title: 'Save (Ctrl+S)' }),
      button('Cancel', 'Cancel (Esc)', close)),
  )
  container.replaceChildren(form)
  const untouched = JSON.stringify(collect())
  title.focus()
}
