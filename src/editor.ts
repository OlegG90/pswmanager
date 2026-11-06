import { api, type Attachment, type AutoTypeData, type EntryData, type Saved } from './api'
import { button, el } from './dom'
import { chip, fieldNameList, fieldRow, input, newEntryData, showHide } from './editor-parts'
import { editorSession } from './editor-session'
import { changeTracker, entryForm, formError } from './entry-form'
import { keep, trimmedLine } from './entry-text'
import { ask, askText, beforeExtension, isAsking } from './modal'
import { iconPicker } from './icon-picker'
import { iconAction } from './icons'

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
  /** And then with these values: a QR code's TOTP, a password Home made (#351). */
  prefill?: Partial<EntryData>
  /** A template is edited, or made (it goes among the templates). */
  template?: boolean
}

/** The window's one editor (see [editorSession]). */
const session = editorSession({ ask, isAsking })

export const isEditing = session.isEditing

/** Keys while the editor is open: Ctrl+S saves, Esc cancels. True when handled. */
export function editorKey(e: KeyboardEvent): boolean {
  if (!session.isEditing()) return false
  if (e.ctrlKey && e.code === 'KeyS') {
    e.preventDefault()
    void session.save()
    return true
  }
  if (e.key === 'Escape') {
    e.preventDefault()
    void session.close()
    return true
  }
  return false
}

export const closeEditor = session.clear

/** Something outside the editor was chosen while it is open: asks whether to
 *  leave it (see the session's `leave`). True when there is no editor any more. */
export const leaveEditor = session.leave

/** Tells the editor that another device changed these entries. */
export const changedElsewhere = session.changedElsewhere

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
  const loaded = await session.load(async () => {
    const data = options.id ? await api.editEntry(options.id) : newEntryData(options.group, options)
    const attachments: Attachment[] = options.id ? (await api.entry(options.id)).attachments : []
    return { data, attachments }
  })
  if (!loaded) return // locked while loading
  const { data, attachments } = loaded

  const error = formError()
  const showError = error.show
  // A template's files are not changed (as before), so it has no files section.
  const fields = entryForm(data, {
    // The date input has no label of its own in this layout.
    attributes: { title: { placeholder: 'Title' }, expires: { ariaLabel: 'Expires' } },
    knownTags: options.knownTags,
    attachments: options.template ? [] : attachments,
    files: {
      pick: api.pickFileToAttach,
      release: api.releaseFiles,
      askName: (current) => askText('Rename the file to:', current, 'Rename', beforeExtension(current)),
    },
    passwordStrength: api.passwordStrength,
    generatePassword: api.generatePassword,
    showError,
  })
  const { title, username, password, url, otp, tags, star, expires, notes, fieldList, files, strength, generator } = fields
  const neverExpires = button('Never', 'Does not expire', () => (expires.value = ''), 'ghost')
  const icon = iconPicker(data.icon, options.autoIcon, (message) => showError(message))

  // A TOTP secret from a QR code (#279): the picture on the clipboard (a
  // snip of the site's code), or one picked.
  const scanQr = iconAction('scanQr', 'Read a QR code: a snip on the clipboard, or a picture', () => {
    api.scanTotpQr().then((uri) => {
      if (uri !== null) otp.value = uri
    }).catch((e) => showError(String(e)))
  })

  const autoType = autoTypeEditor(data.autoType)
  const collect = (): EntryData => ({ ...fields.collect(), icon: icon.value(), autoType: autoType.value() })

  // The field last used, for the way back into the form.
  let lastField: HTMLElement = title
  const { save, close } = session.open({
    id: options.id,
    // What counts as unchanged is taken once the form is drawn (below).
    dirty: () => dirty(),
    store: () => api.saveEntry(options.id, options.id ? data : null, collect(), options.template, files.changes()),
    onSaved: options.onSaved,
    onClosed: () => {
      files.release()
      options.onClose()
    },
    focus: () => lastField.focus(),
    showError,
  })

  const row = (label: string, ...controls: Node[]) =>
    el('div', { className: 'edit-row' }, el('span', { className: 'label' }, label), el('div', { className: 'controls' }, ...controls))
  const form = el(
    'form',
    { className: 'editor', onsubmit: (e: SubmitEvent) => (e.preventDefault(), save()) },
    el('h2', {}, `${options.id ? 'Edit' : 'New'} ${options.template ? 'template' : 'entry'}`),
    error.line,
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
    fieldNameList(fieldList),
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
  const dirty = changeTracker(collect, files)
  ;(options.focusPassword ? password : title).focus()
}
