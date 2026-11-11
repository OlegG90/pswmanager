import { button, el, enterPresses } from '../../../../src/dom'
import { fieldRow, input, newEntryData, showHide } from '../../../../src/editor-parts'
import { changeTracker, entryForm, formError } from '../../../../src/entry-form'
import { beforeExtension, titleOf } from '../../../../src/entry-text'
import { iconAction } from '../../../../src/icons'
import { sameFilter, search, tagCounts, TEMPLATES } from '../../../../src/search'
import { api, type EntryData, type EntryDetail, type Listing } from './api'
import { entryScreen } from './entry'
import { listScreen } from './list'
import { app, confirmSheet, iconButton, screens, sheet, show, snack } from './page'
import { browsePhone, onPhone } from './phone-files'

// The entry editor, and the new entry the list's + starts.

/** The list's +: a blank entry, or one from a template (asked when there are
 *  any); in Templates, a new template. */
export function newEntry(listing: Listing) {
  const back = () => listScreen(listing)
  if (sameFilter(app.filter, TEMPLATES)) return void editorScreen(null, listing, back, { template: true })
  const templates = search(listing.entries, '', TEMPLATES)
  if (!templates.length) return void editorScreen(null, listing, back)
  sheet((close) => [
    el('b', {}, 'New entry'),
    button('Blank entry', 'An empty entry', () => (close(), void editorScreen(null, listing, back)), 'item'),
    ...templates.map((t) => button(titleOf(t), 'A new entry with its fields, icon and tags', () => (close(), void newFromTemplate(t.id, listing, back)), 'item')),
  ])
}

/** A new entry with the template's fields, icon and tags, not its title. */
export async function newFromTemplate(id: string, listing: Listing, back: () => void) {
  try {
    await editorScreen(null, listing, back, { from: await api.editEntry(id) })
  } catch (e) {
    snack(String(e))
  }
}

// ------------------------------------------------------------ editing

/** Asks for a file's new name in a sheet; null when cancelled. */
function askName(current: string): Promise<string | null> {
  return new Promise((done) => {
    const name = input(current, { className: 'field', ariaLabel: 'New name' })
    sheet((close) => {
      const answer = (value: string | null) => (close(), done(value))
      const rename = button('Rename', 'Rename', () => answer(name.value), 'primary')
      enterPresses(rename, name)
      return [el('b', {}, 'Rename the file'), name, rename, button('Cancel', 'Cancel', () => answer(null), 'link')]
    })
    // The name without its extension is chosen, ready to type over.
    name.focus()
    name.setSelectionRange(0, beforeExtension(current))
  })
}

/** The editor, as `spec.md` *Editing* has it; a new blank entry when `id` is
 *  null. Cancel and Back return with `back`; saving shows the entry. */
export async function editorScreen(id: string | null, listing: Listing, back: () => void, start: { from?: EntryData; template?: boolean } = {}) {
  const template = !!start.template
  let data: EntryData
  let attachments: EntryDetail['attachments'] = []
  try {
    // A new entry goes to the top group (a template among the templates).
    data = id ? await api.editEntry(id) : newEntryData([], { ...start, defaultUsername: listing.database.defaultUsername })
    if (id) attachments = (await api.entry(id)).attachments
  } catch (e) {
    snack(String(e))
    return
  }
  const error = formError()
  const showError = error.show
  const fields = entryForm(data, {
    fieldClass: 'field',
    // A user name starts with a small letter; a URL gets the keyboard for one.
    attributes: { username: { autocapitalize: 'off' }, url: { type: 'url' } },
    knownTags: tagCounts(listing.entries).map(([tag]) => tag),
    attachments,
    files: {
      pick: () => onPhone('The file to attach', (done, picker) => browsePhone({ pick: api.attachFileAt, done, picker }), api.pickFileToAttach),
      release: api.releaseFiles,
      askName,
    },
    passwordStrength: api.passwordStrength,
    generatePassword: api.generatePassword,
    showError,
  })
  const { title, username, password, url, otp, tags, star, expires, notes, fieldList, files, strength, generator, collect } = fields
  // A TOTP secret from a QR code: with the camera (#280), or in a picture,
  // a screenshot of the site's code (#279).
  const scan = (camera: boolean) => {
    api.scanTotpQr(camera).then((uri) => {
      if (uri !== null) otp.value = uri
    }).catch((e) => showError(String(e)))
  }
  const scanQr = iconAction('scanQr', 'Read a QR code', () => sheet((close) => [
    el('b', {}, 'Read a QR code'),
    button('With the camera', 'Point the camera at the code the site shows', () => (close(), scan(true)), 'item'),
    button('From a picture…', 'A screenshot of the code', () => (close(), scan(false)), 'item'),
  ]), 'state')

  const dirty = changeTracker(collect, files)

  let saving = false
  const save = async () => {
    if (saving) return
    saving = true
    try {
      const saved = await api.saveEntry(id, id ? data : null, collect(), template, files.changes())
      app.showSaved(saved.id, saved.listing.entries, template)
      await entryScreen(saved.id, saved.listing)
      if (saved.conflicts.length) snack(`Also changed on another device: ${saved.conflicts.join(', ')}. That version is in the history.`)
    } catch (e) {
      showError(String(e))
    } finally {
      saving = false
    }
  }
  const leaveEditor = () => {
    files.release()
    back()
  }
  const close = () => {
    if (!dirty()) leaveEditor()
    else confirmSheet('Discard the changes?', 'Discard', leaveEditor)
  }
  // Android's Back cancels too, and stays on the editor when the changes are kept.
  const onBack = () => {
    screens.push(onBack)
    close()
  }
  const label = (text: string, ...controls: Node[]) => el('label', { className: 'edit-row' }, el('small', {}, text), ...controls)
  const together = (...controls: Node[]) => el('div', { className: 'together' }, ...controls)
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Cancel', close), el('h1', {}, `${id ? 'Edit' : 'New'} ${template ? 'template' : 'entry'}`), iconButton('check', 'Save', () => void save())),
    el('form', { className: 'editor', onsubmit: (e: SubmitEvent) => (e.preventDefault(), void save()) },
      error.line,
      label('Title', title),
      label('User name', username),
      label('Password', together(password, showHide(password), generator.open)),
      strength.element,
      generator.panel,
      label('URL', url),
      label('TOTP secret', together(otp, showHide(otp), scanQr)),
      label('Tags', tags.element),
      // A template has no star, and its files are not changed (as on Windows).
      ...(template ? [] : [star]),
      label('Expires', together(expires, button('Never', 'Does not expire', () => (expires.value = ''), 'link'))),
      label('Notes', notes),
      el('h2', {}, 'Additional fields'),
      fieldList,
      button('+ Add field', 'Add a field', () => fieldList.append(fieldRow()), 'link'),
      ...(template ? [] : [el('h2', {}, 'Files'), files.element]),
      ...(id ? [el('p', { className: 'muted' }, 'Saving keeps the previous version in the history.')] : [])),
  ], onBack)
  if (!id) title.focus()
  // Saving still works: only what was edited here is applied, and where both
  // changed a field the other version goes to the entry's history.
  screens.onSynced((synced) => {
    if (!id || !synced.changed) return
    void api.editEntry(id).then((now) => {
      if (JSON.stringify(now) !== JSON.stringify(data)) showError('This entry was just changed on another device. Saving keeps your version; the other one goes to the entry’s history.')
    }, () => showError('This entry was just deleted on another device. Saving brings it back.'))
  })
}
