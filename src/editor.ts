import { api, type EntryData, type Saved } from './api'
import { button, el } from './dom'
import { EMPTY_ENTRY, chip, fieldRow, generatorPanel, input, readField, showHide, strengthMeter, withStar } from './editor-parts'
import { dateOf, formatTags, keep, parseTags, singleLine, startOfDay, textareaLines } from './entry-text'
import { ask } from './modal'
import { iconPicker } from './icon-picker'
import { tagInput } from './tag-input'
import { FAVORITE } from './search'

export interface EditorOptions {
  /** The entry to change; null creates one. */
  id: string | null
  /** Where a new entry goes. */
  group: string[]
  onSaved: (saved: Saved) => void
  onClose: () => void
  /** Starts in the password field (to change it) instead of the title. */
  focusPassword?: boolean
  /** The database's tags, offered when adding one. */
  knownTags: string[]
  /** The database's default user name, filled in on a new blank entry. */
  defaultUsername?: string
  /** What the Auto icon shows for this entry now. */
  autoIcon: string
  /** A new entry starts from this template's values (not its title or expiry). */
  from?: EntryData
  /** A template is edited, or made (it goes among the templates). */
  template?: boolean
}

/** The open editor, from the moment it starts loading. */
let active: {
  save: () => void
  close: () => void | Promise<void>
  /** The entries another device just changed. */
  changedElsewhere?: (ids: string[]) => void
} | null = null

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

/** Tells the editor that another device changed these entries. */
export function changedElsewhere(ids: string[]) {
  active?.changedElsewhere?.(ids)
}

/** What a new entry takes from a template: everything but its title, expiry,
 *  TOTP secret (every entry has its own) and star. */
function fromTemplate(template?: EntryData): Partial<EntryData> {
  return template ? { ...template, title: '', expires: null, otp: '', tags: template.tags.filter((t) => t !== FAVORITE) } : {}
}

/** Shows the editor in `container`. */
export async function openEditor(container: HTMLElement, options: EditorOptions) {
  // Counts as open while loading, so a second Ctrl+E or a click in the list waits.
  const loading = { save: () => {}, close: () => {} }
  active = loading
  let data: EntryData
  try {
    // A blank new entry (not from a template, not a template) gets the default user name.
    const blank = !options.from && !options.template
    data = options.id
      ? await api.editEntry(options.id)
      : { ...EMPTY_ENTRY, username: blank ? (options.defaultUsername ?? '') : '', ...fromTemplate(options.from), group: options.group }
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
  const tags = tagInput(data.tags.filter((t) => t !== FAVORITE), options.knownTags)
  let starred = data.tags.includes(FAVORITE)
  const star = chip('★ Favorite', 'Listed under Favorites', starred, (on) => (starred = on))
  // A date input holds a day: an expiry time on that day stays as it was.
  const expiryDay = data.expires ? dateOf(data.expires) : ''
  const expires = el('input', { type: 'date', value: expiryDay, ariaLabel: 'Expires' })
  const neverExpires = button('Never', 'Does not expire', () => (expires.value = ''), 'ghost')
  const icon = iconPicker(data.icon, options.autoIcon, (message) => showError(message))
  const notes = el('textarea', { value: data.notes, rows: 4, spellcheck: false })
  const error = el('p', { className: 'error', hidden: true })
  const fieldList = el('div', { className: 'fields' }, ...data.fields.map((f) => fieldRow(f)))

  // Under the heading, and scrolled to: at the bottom of a long form the
  // message ended up out of sight.
  const showError = (message: string) => {
    error.textContent = message
    error.hidden = false
    error.scrollIntoView({ block: 'nearest' })
  }

  const strength = strengthMeter(password, api.passwordStrength)

  const generator = generatorPanel(api.generatePassword, (chosen) => {
    password.value = chosen
    strength.refresh()
  }, showError)

  const trimmedLine = (text: string) => singleLine(text).trim()
  const collect = (): EntryData => ({
    title: keep(data.title, title.value, singleLine),
    username: keep(data.username, username.value, singleLine),
    password: keep(data.password, password.value, singleLine),
    url: keep(data.url, url.value.trim(), trimmedLine),
    notes: keep(data.notes, notes.value, textareaLines),
    otp: keep(data.otp, otp.value.trim(), trimmedLine),
    // Typing the tag Favorite stars the entry.
    tags: keep(data.tags, withStar(tags.value(), starred || tags.value().includes(FAVORITE), data.tags), (t) => parseTags(formatTags(t))),
    // Not edited: an entry stays in its KDBX group; a new one goes to the top.
    group: data.group,
    fields: [...fieldList.querySelectorAll<HTMLDivElement>('.field-row')].map(readField),
    icon: icon.value(),
    expires: expires.value === expiryDay ? data.expires : expires.value ? startOfDay(expires.value) : null,
  })

  let saving = false
  const save = async () => {
    if (saving) return
    saving = true
    try {
      const saved = await api.saveEntry(options.id, options.id ? data : null, collect(), options.template)
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
  const close = async () => {
    if (JSON.stringify(collect()) !== untouched) {
      const discard = await ask('Discard the unsaved changes?', 'Discard', 'Keep editing')
      if (!discard || active !== self) return
    }
    active = null
    options.onClose()
  }
  // Saving still works: this edit is applied to the file as it is now, wins
  // where both changed the entry, and the other version goes to its history.
  const changed = (ids: string[]) => {
    if (options.id && ids.includes(options.id)) {
      showError('This entry was just changed on another device. Saving keeps your version; the other one goes to the entry\'s history.')
    }
  }
  const self = { save, close, changedElsewhere: changed }
  active = self

  const row = (label: string, ...controls: Node[]) =>
    el('div', { className: 'edit-row' }, el('span', { className: 'label' }, label), el('div', { className: 'controls' }, ...controls))
  const form = el(
    'form',
    { className: 'editor', onsubmit: (e: SubmitEvent) => (e.preventDefault(), save()) },
    el('h2', {}, `${options.id ? 'Edit' : 'New'} ${options.template ? 'template' : 'entry'}`),
    error,
    row('Title', title),
    row('User name', username),
    row('Password', password, showHide(password), generator.open),
    row('', strength.element),
    generator.panel,
    row('URL', url),
    row('TOTP', otp, showHide(otp)),
    row('Tags', tags.element),
    ...(options.template ? [] : [row('', star)]),
    row('Expires', expires, neverExpires),
    row('Icon', icon.element),
    row('Notes', notes),
    el('h3', {}, 'Additional fields'),
    fieldList,
    button('+ Add field', 'Add a field', () => fieldList.append(fieldRow()), 'ghost add-field'),
    el('div', { className: 'buttons' },
      el('button', { type: 'submit', className: 'primary', textContent: 'Save', title: 'Save (Ctrl+S)' }),
      button('Cancel', 'Cancel (Esc)', close),
      ...(options.id ? [el('span', { className: 'hint' }, 'Saving keeps the previous version in history.')] : [])),
  )
  container.replaceChildren(form)
  const untouched = JSON.stringify(collect())
  ;(options.focusPassword ? password : title).focus()
}
