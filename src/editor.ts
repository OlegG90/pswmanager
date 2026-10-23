import { api, type Attachment, type AutoTypeData, type EntryData, type Saved } from './api'
import { button, el } from './dom'
import { EMPTY_ENTRY, chip, collectEntry, fieldRow, filesEditor, generatorPanel, input, showHide, strengthMeter } from './editor-parts'
import { dateOf, keep, trimmedLine } from './entry-text'
import { ask, askText, beforeExtension, isAsking } from './modal'
import { iconPicker } from './icon-picker'
import { iconAction } from './icons'
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
  /** Closes without saving or asking: nothing of the edit is kept. */
  discard: () => void
  /** Puts the keyboard back where it was in the form. */
  focus: () => void
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

/**
 * Something outside the editor was chosen while it is open: asks whether to
 * leave it. Yes closes it without saving (the entry shows as it was), No keeps
 * the user in it. True when there is no editor any more.
 */
export async function leaveEditor(): Promise<boolean> {
  const open = active
  if (!open) return true
  if (isAsking()) return false
  const leave = await ask('Leave editing? The changes are not saved.', 'Yes', 'No')
  // Saved, cancelled or locked while asking: what was chosen is not done.
  if (active !== open) return false
  if (leave) open.discard()
  else open.focus()
  return leave
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

/** The entry's Auto-Type settings (#272): on or off, its own sequence, and
 *  sequences for windows whose title matches. What the form only trimmed
 *  stays as the file has it, so an untouched entry saves unchanged. */
function autoTypeEditor(data: AutoTypeData) {
  let enabled = data.enabled
  const on = chip('Auto-Type on', 'Auto-Type types this entry', enabled, (pressed) => (enabled = pressed))
  const sequence = input(data.sequence, { placeholder: 'The group\'s, else {USERNAME}{TAB}{PASSWORD}{ENTER}', className: 'sequence' })
  const association = (a = { window: '', sequence: '' }) => {
    const row = el('div', { className: 'field-row association' },
      input(a.window, { placeholder: 'Window title, * for any text', className: 'name', ariaLabel: 'Window title' }),
      input(a.sequence, { placeholder: 'Sequence (empty: the entry\'s)', className: 'value', ariaLabel: 'Sequence' }),
      button('✕', 'Remove this window', () => row.remove(), 'ghost icon'))
    return row
  }
  const list = el('div', { className: 'fields' }, ...data.associations.map(association))
  const add = button('+ Add window', 'A sequence of its own for windows whose title matches', () => list.append(association()), 'ghost add-field')
  const trimmed = (list: AutoTypeData['associations']) => list.map((a) => ({ window: a.window.trim(), sequence: a.sequence.trim() }))
  const value = (): AutoTypeData => {
    const associations = trimmed([...list.querySelectorAll<HTMLDivElement>('.association')].map((row) => {
      const [window, sequence] = row.querySelectorAll('input')
      return { window: window.value, sequence: sequence.value }
    })).filter((a) => a.window)
    return {
      enabled,
      sequence: keep(data.sequence, sequence.value.trim(), trimmedLine),
      associations: keep(data.associations, associations, trimmed),
    }
  }
  return { on, sequence, list, add, value }
}

/** Shows the editor in `container`. */
export async function openEditor(container: HTMLElement, options: EditorOptions) {
  // Counts as open while loading, so a second Ctrl+E or a click in the list waits.
  const loading = { save: () => {}, close: () => {}, discard: () => {}, focus: () => {} }
  active = loading
  let data: EntryData
  let attachments: Attachment[] = []
  try {
    // A blank new entry (not from a template, not a template) gets the default user name.
    const blank = !options.from && !options.template
    data = options.id
      ? await api.editEntry(options.id)
      : { ...EMPTY_ENTRY, username: blank ? (options.defaultUsername ?? '') : '', ...fromTemplate(options.from), group: options.group }
    if (options.id) attachments = (await api.entry(options.id)).attachments
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
  // A template's files are not changed (as before), so it has no files section.
  const files = filesEditor(options.template ? [] : attachments, {
    pick: api.pickFileToAttach,
    release: api.releaseFiles,
    askName: (current) => askText('Rename the file to:', current, 'Rename', beforeExtension(current)),
  }, (message) => showError(message))

  // Under the heading, and scrolled to: at the bottom of a long form the
  // message ended up out of sight.
  const showError = (message: string) => {
    error.textContent = message
    error.hidden = false
    error.scrollIntoView({ block: 'nearest' })
  }

  // A TOTP secret from a QR code (#279): the picture on the clipboard (a
  // snip of the site's code), or one picked.
  const scanQr = iconAction('scanQr', 'Read a QR code: a snip on the clipboard, or a picture', () => {
    api.scanTotpQr().then((uri) => {
      if (uri !== null) otp.value = uri
    }).catch((e) => showError(String(e)))
  })

  const strength = strengthMeter(password, api.passwordStrength)

  const generator = generatorPanel(api.generatePassword, (chosen) => {
    password.value = chosen
    strength.refresh()
  }, showError)

  const inputs = { title, username, password, url, otp, notes, tags, starred: () => starred, fieldList, expires }
  const autoType = autoTypeEditor(data.autoType)
  const collect = (): EntryData => ({ ...collectEntry(data, inputs), icon: icon.value(), autoType: autoType.value() })

  let saving = false
  const save = async () => {
    if (saving) return
    saving = true
    try {
      const saved = await api.saveEntry(options.id, options.id ? data : null, collect(), options.template, files.changes())
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
  const discard = () => {
    active = null
    files.release()
    options.onClose()
  }
  const close = async () => {
    if (JSON.stringify(collect()) !== untouched || files.changes().length) {
      const yes = await ask('Discard the unsaved changes?', 'Discard', 'Keep editing')
      if (!yes || active !== self) return
    }
    discard()
  }
  // Saving still works: this edit is applied to the file as it is now, wins
  // where both changed the entry, and the other version goes to its history.
  const changed = (ids: string[]) => {
    if (options.id && ids.includes(options.id)) {
      showError('This entry was just changed on another device. Saving keeps your version; the other one goes to the entry\'s history.')
    }
  }
  // The field last used, for the way back into the form.
  let lastField: HTMLElement = title
  const focus = () => lastField.focus()
  const self = { save, close, discard, focus, changedElsewhere: changed }
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
    row('TOTP', otp, showHide(otp), scanQr),
    row('Tags', tags.element),
    ...(options.template ? [] : [row('', star)]),
    row('Expires', expires, neverExpires),
    row('Icon', icon.element),
    row('Notes', notes),
    el('h3', {}, 'Additional fields'),
    fieldList,
    button('+ Add field', 'Add a field', () => fieldList.append(fieldRow()), 'ghost add-field'),
    ...(options.template ? [] : [el('h3', {}, 'Files'), files.element]),
    el('h3', {}, 'Auto-Type'),
    row('', autoType.on),
    row('Sequence', autoType.sequence),
    autoType.list,
    autoType.add,
    el('div', { className: 'buttons' },
      el('button', { type: 'submit', className: 'primary', textContent: 'Save', title: 'Save (Ctrl+S)' }),
      button('Cancel', 'Cancel (Esc)', close),
      ...(options.id ? [el('span', { className: 'hint' }, 'Saving keeps the previous version in history.')] : [])),
  )
  form.addEventListener('focusin', (e) => (lastField = e.target as HTMLElement))
  container.replaceChildren(form)
  const untouched = JSON.stringify(collect())
  ;(options.focusPassword ? password : title).focus()
}
