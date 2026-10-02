/** Parts of the entry editor that PswManager for Windows and for Android share. */
import type { Attachment, EntryData, FieldData, FileChange, GeneratorOptions, StagedFile, Strength } from './api'
import { button, el } from './dom'
import type { MenuItem } from './menu'
import { dateOf, formatSize, formatTags, keep, parseTags, singleLine, startOfDay, textareaLines, trimmedLine } from './entry-text'
import { FAVORITE } from './search'

export const EMPTY_ENTRY: EntryData = { title: '', username: '', password: '', url: '', notes: '', otp: '', tags: [], group: [], fields: [], icon: { kind: 'auto' }, expires: null }
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

export const input = (value: string, props: object = {}) => el('input', { value, spellcheck: false, ...props })

export function showHide(target: HTMLInputElement): HTMLButtonElement {
  const toggle = button('Show', 'Show / hide', () => {
    const hidden = target.type === 'password'
    target.type = hidden ? 'text' : 'password'
    toggle.textContent = hidden ? 'Hide' : 'Show'
  })
  return toggle
}

/** A button that stays pressed or not, like a check box. */
export function chip(label: string, title: string, pressed: boolean, onChange: (pressed: boolean) => void): HTMLButtonElement {
  const chip = button(label, title, () => {
    const next = chip.getAttribute('aria-pressed') !== 'true'
    chip.setAttribute('aria-pressed', String(next))
    onChange(next)
  }, 'chip')
  chip.setAttribute('aria-pressed', String(pressed))
  return chip
}

/** The password generator panel; `use` receives the chosen password. */
export function generatorPanel(generate: (options: GeneratorOptions) => Promise<string>, use: (password: string) => void, onError: (message: string) => void) {
  const preview = el('code', { className: 'preview' })
  const regenerate = async () => {
    try {
      preview.textContent = await generate(generatorOptions)
    } catch (e) {
      preview.textContent = ''
      onError(String(e))
    }
  }
  const option = (key: keyof GeneratorOptions, label: string) =>
    chip(label, label, generatorOptions[key] as boolean, (on) => {
      generatorOptions = { ...generatorOptions, [key]: on }
      regenerate()
    })
  // 8–64: the range the backend allows.
  const length = el('input', { type: 'range', min: 8, max: 64, value: String(generatorOptions.length), ariaLabel: 'Length' })
  const lengthValue = el('span', { className: 'length-value' }, String(generatorOptions.length))
  // How far the track is filled, for the CSS: a range input cannot tell it.
  const fill = () => length.style.setProperty('--fill', `${((Number(length.value) - 8) / (64 - 8)) * 100}%`)
  fill()
  length.addEventListener('input', () => {
    generatorOptions = { ...generatorOptions, length: Number(length.value) || 20 }
    lengthValue.textContent = String(generatorOptions.length)
    fill()
    regenerate()
  })
  const show = (visible: boolean) => {
    panel.hidden = !visible
    open.setAttribute('aria-pressed', String(visible))
  }
  const panel = el(
    'div',
    { className: 'generator', hidden: true },
    el('div', { className: 'result' }, preview, button('Again', 'Another password', regenerate, 'ghost'),
      button('Use', 'Use this password', () => {
        use(preview.textContent ?? '')
        show(false)
      }, 'primary')),
    el('div', { className: 'length' }, el('span', { className: 'label' }, 'Length'), length, lengthValue),
    el('div', { className: 'options' }, option('upper', 'A–Z'), option('lower', 'a–z'),
      option('digits', '0–9'), option('symbols', '!#$'), option('excludeLookAlikes', 'No look-alikes')),
  )
  const open = chip('Generate', 'Generate a password', false, (on) => {
    show(on)
    if (on) regenerate()
  })
  return { panel, open }
}

/** The strength indicator under `password`, updated as it is typed; `refresh`
 *  after the value is set from code. */
export function strengthMeter(password: HTMLInputElement, estimate: (password: string) => Promise<Strength>) {
  const element = el('div', { className: 'strength' })
  let timer: number | undefined
  const refresh = () => {
    clearTimeout(timer)
    timer = window.setTimeout(async () => {
      if (!password.value) {
        element.replaceChildren()
        return
      }
      const { score, crackTime } = await estimate(password.value)
      element.dataset.score = String(score)
      // At least one bar, so "very weak" still shows as a red mark.
      const bars = [1, 2, 3, 4].map((i) => el('span', { className: i <= Math.max(score, 1) ? 'bar on' : 'bar' }))
      element.replaceChildren(el('span', { className: 'bars' }, ...bars), `${STRENGTH[score]} · cracked in ${crackTime}`)
    }, 200)
  }
  password.addEventListener('input', refresh)
  refresh()
  return { element, refresh }
}

/** The tags with the star (the tag Favorite) on or off; a star the entry had
 *  keeps its place among the tags, so an untouched entry saves unchanged. */
export function withStar(tags: string[], on: boolean, original: string[]): string[] {
  const rest = tags.filter((t) => t !== FAVORITE)
  if (!on) return rest
  const at = original.indexOf(FAVORITE)
  return at < 0 ? [...rest, FAVORITE] : [...rest.slice(0, at), FAVORITE, ...rest.slice(at)]
}

/** The field each row started from, to keep values the form only reformatted. */
const originals = new WeakMap<HTMLElement, FieldData>()

/** One additional field: name, value (a textarea keeps line breaks an <input>
 *  would drop; a protected value is masked by CSS), protected, remove. */
export function fieldRow(field: FieldData = { name: '', value: '', protected: false }): HTMLDivElement {
  const name = input(field.name, { placeholder: 'Name', className: 'name' })
  const value = el('textarea', { value: field.value, rows: 1, spellcheck: false, className: 'value' })
  const fit = () => {
    value.style.height = 'auto'
    value.style.height = `${value.scrollHeight + 2}px`
  }
  value.addEventListener('input', fit)
  requestAnimationFrame(fit)
  const mask = (on: boolean) => value.classList.toggle('masked', on)
  const protect = chip('Protected', 'Protected values are masked like the password', field.protected, mask)
  mask(field.protected)
  const row = el('div', { className: 'field-row' }, name, value, protect,
    button('✕', 'Remove field', () => row.remove(), 'ghost icon'))
  originals.set(row, field)
  return row
}

export function readField(row: HTMLElement): FieldData {
  const name = row.querySelector('input')!
  const value = row.querySelector('textarea')!
  const protect = row.querySelector('.chip')!
  const original = originals.get(row)!
  return {
    name: keep(original.name, name.value.trim(), trimmedLine),
    value: keep(original.value, value.value, textareaLines),
    protected: protect.getAttribute('aria-pressed') === 'true',
  }
}

/** The editor's inputs, as both apps have them. */
export interface EditorInputs {
  title: HTMLInputElement
  username: HTMLInputElement
  password: HTMLInputElement
  url: HTMLInputElement
  otp: HTMLInputElement
  notes: HTMLTextAreaElement
  tags: { value: () => string[] }
  starred: () => boolean
  /** The additional fields' rows (`fieldRow`). */
  fieldList: HTMLElement
  /** A date input: a day. */
  expires: HTMLInputElement
}

/** The entry as the editor's inputs have it now. Values the form only
 *  reformats stay as they were in `data`, so an untouched entry saves
 *  unchanged; what the editor does not edit (the group: an entry stays in its
 *  KDBX group, a new one goes to the top) comes from `data`. */
export function collectEntry(data: EntryData, inputs: EditorInputs): EntryData {
  const typedTags = inputs.tags.value()
  // An expiry time on the day shown stays as it was.
  const expiryDay = data.expires ? dateOf(data.expires) : ''
  const expires = inputs.expires.value
  return {
    ...data,
    title: keep(data.title, inputs.title.value, singleLine),
    username: keep(data.username, inputs.username.value, singleLine),
    password: keep(data.password, inputs.password.value, singleLine),
    url: keep(data.url, inputs.url.value.trim(), trimmedLine),
    notes: keep(data.notes, inputs.notes.value, textareaLines),
    otp: keep(data.otp, inputs.otp.value.trim(), trimmedLine),
    // Typing the tag Favorite stars the entry.
    tags: keep(data.tags, withStar(typedTags, inputs.starred() || typedTags.includes(FAVORITE), data.tags), (t) => parseTags(formatTags(t))),
    fields: [...inputs.fieldList.querySelectorAll<HTMLDivElement>('.field-row')].map(readField),
    expires: expires === expiryDay ? data.expires : expires ? startOfDay(expires) : null,
  }
}

/** What the files section needs from the app: picking a file (held by the
 *  backend, see StagedFile), letting go of picked files, asking for a new
 *  name, and a file's menu button. */
export interface FileUi {
  pick: () => Promise<StagedFile | null>
  release: (files: number[]) => Promise<void>
  /** A new name for the file; null when cancelled. */
  askName: (current: string) => Promise<string | null>
  menu: (title: string, items: MenuItem[]) => HTMLElement
}

/** One file in the files section: one the entry has (`original`), or one
 *  picked here; `staged` is new content picked for it. */
interface FileState {
  original: string | null
  staged: StagedFile | null
  name: string
}

/** What will happen to the file on Save, as its line says it. */
function fileNote(f: FileState): string {
  if (f.original === null) return 'new'
  return [f.name !== f.original ? `renamed from ${f.original}` : '', f.staged ? 'replaced' : ''].filter(Boolean).join(' · ')
}

/**
 * The entry's files in the editor, as SafeInCloud and Keepass2Android have
 * them: each file's menu renames, replaces or removes it, and new ones are
 * added; each line says what Save will do to its file. Nothing changes until
 * the entry is saved: `changes` are sent with it. `release` lets go of every
 * picked file (the editor closed without saving).
 */
export function filesEditor(initial: Attachment[], ui: FileUi, onError: (message: string) => void) {
  const states: FileState[] = []
  const list = el('div', { className: 'files' })
  const held = () => states.flatMap((f) => (f.staged ? [f.staged.content] : []))

  const pick = async (): Promise<StagedFile | null> => {
    try {
      return await ui.pick()
    } catch (e) {
      onError(String(e))
      return null
    }
  }
  const row = (state: FileState, size: number) => {
    const name = el('span', { className: 'name' })
    const note = el('small', { className: 'note' })
    const sizeText = el('span', { className: 'size' })
    const draw = () => {
      name.textContent = state.name
      note.textContent = fileNote(state)
      sizeText.textContent = formatSize(state.staged?.size ?? size)
    }
    const rename = async () => {
      const to = (await ui.askName(state.name))?.trim()
      if (!to || to === state.name) return
      if (states.some((f) => f !== state && f.name === to)) return onError(`The entry already has a file named “${to}”`)
      state.name = to
      draw()
    }
    const replace = async () => {
      const picked = await pick()
      if (!picked) return
      if (state.staged) void ui.release([state.staged.content])
      state.staged = picked
      draw()
    }
    const remove = () => {
      if (state.staged) void ui.release([state.staged.content])
      states.splice(states.indexOf(state), 1)
      line.remove()
    }
    const items: MenuItem[] = [
      { label: 'Rename…', title: 'Give the file another name', action: () => void rename() },
      ...(state.original ? [{ label: 'Replace…', title: 'Replace with another file (the entry’s history keeps this one)', action: () => void replace() }] : []),
      { label: 'Remove', title: 'Remove from the entry (its history keeps the file)', action: remove, danger: true },
    ]
    const line = el('div', { className: 'attachment-row' }, el('span', { className: 'what' }, name, note), sizeText, ui.menu(`More for ${state.name}`, items))
    draw()
    list.append(line)
  }
  for (const file of initial) {
    const state: FileState = { original: file.name, staged: null, name: file.name }
    states.push(state)
    row(state, file.size)
  }
  const add = button('+ Add file…', 'Attach a file (up to 20 MB)', async () => {
    const picked = await pick()
    if (!picked) return
    const state: FileState = { original: null, staged: picked, name: picked.name }
    states.push(state)
    row(state, picked.size)
  }, 'ghost add-field')

  const changes = (): FileChange[] => {
    const kept = new Set(states.map((f) => f.original))
    const removed: FileChange[] = initial.filter((f) => !kept.has(f.name)).map((f) => ({ kind: 'remove', name: f.name }))
    const changed: FileChange[] = states.flatMap((f): FileChange[] => {
      if (f.original === null) return [{ kind: 'add', name: f.name, content: f.staged!.content }]
      if (f.name === f.original && !f.staged) return []
      return [{ kind: 'change', name: f.original, to: f.name, content: f.staged?.content ?? null }]
    })
    return [...removed, ...changed]
  }
  return {
    element: el('div', { className: 'files-editor' }, list, add),
    changes,
    release: () => {
      const files = held()
      if (files.length) void ui.release(files)
    },
  }
}
