import type { Attachment, EntryData, GeneratorOptions, Strength } from './api'
import { el } from './dom'
import { chip, collectEntry, fieldRow, filesEditor, generatorPanel, input, strengthMeter, type FileUi } from './editor-parts'
import { dateOf } from './entry-text'
import { FAVORITE } from './search'
import { tagInput } from './tag-input'

/** Attributes an app gives some of the fields besides the form's own. */
type FieldAttributes = Partial<Record<'title' | 'username' | 'url' | 'expires', object>>

/** What the form needs from the app it is in. */
export interface EntryFormHost {
  /** The class every field has in this app's layout, if one. */
  fieldClass?: string
  attributes?: FieldAttributes
  /** The database's tags, offered when adding one. */
  knownTags: string[]
  /** The files the form starts with (none for a template, on Windows). */
  attachments: Attachment[]
  files: FileUi
  passwordStrength: (password: string) => Promise<Strength>
  generatePassword: (options: GeneratorOptions) => Promise<string>
  /** Says something went wrong, in the form. */
  showError: (message: string) => void
}

/**
 * The entry editor's form, as PswManager for Windows and for Android share
 * it: its inputs filled from `data`, the star, the files, the strength meter
 * and the generator, and the entry as they have it now (`collect`). The app
 * lays them out, adds what only it has (on Windows the icon and Auto-Type),
 * and saves and closes the editor its own way.
 */
export function entryForm(data: EntryData, host: EntryFormHost) {
  const extra = host.attributes ?? {}
  /** `attributes` with the app's field class and `className`, when there is one. */
  const withClass = (attributes: object, className = '') => {
    const classes = [host.fieldClass, className].filter(Boolean).join(' ')
    return classes ? { ...attributes, className: classes } : attributes
  }

  const title = input(data.title, withClass({ ...extra.title }))
  const username = input(data.username, withClass({ ...extra.username }))
  const password = input(data.password, withClass({ type: 'password' }, 'secret'))
  const url = input(data.url, withClass({ ...extra.url, placeholder: 'https://' }))
  const otp = input(data.otp, withClass({ type: 'password', placeholder: 'Secret or otpauth:// URI' }, 'secret'))
  const tags = tagInput(data.tags.filter((t) => t !== FAVORITE), host.knownTags)
  let starred = data.tags.includes(FAVORITE)
  const star = chip('★ Favorite', 'Listed under Favorites', starred, (on) => (starred = on))
  // A date input holds a day: an expiry time on that day stays as it was.
  const expires = el('input', withClass({ type: 'date', value: data.expires ? dateOf(data.expires) : '', ...extra.expires }))
  const notes = el('textarea', withClass({ value: data.notes, rows: 4, spellcheck: false }))
  const fieldList = el('div', { className: 'fields' }, ...data.fields.map((f) => fieldRow(f)))
  const files = filesEditor(host.attachments, host.files, host.showError)

  const strength = strengthMeter(password, host.passwordStrength)
  const generator = generatorPanel(host.generatePassword, (chosen) => {
    password.value = chosen
    strength.refresh()
  }, host.showError)

  const inputs = { title, username, password, url, otp, notes, tags, starred: () => starred, fieldList, expires }
  return {
    title, username, password, url, otp, tags, star, expires, notes, fieldList, files, strength, generator,
    /** The entry as the inputs have it now (see [collectEntry]). */
    collect: (): EntryData => collectEntry(data, inputs),
  }
}

/** Whether the editor has changes: `collect` differs from what it gave when
 *  this was called, or a file was added, renamed or removed. */
export function changeTracker(collect: () => EntryData, files: { changes: () => unknown[] }): () => boolean {
  const untouched = JSON.stringify(collect())
  return () => JSON.stringify(collect()) !== untouched || files.changes().length > 0
}
