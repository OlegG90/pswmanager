import { busyButton, button, el, enterPresses, errorLine } from '../../../../src/dom'
import { describeEntries, titleOf } from '../../../../src/entry-text'
import { EXPORT_FORMATS, exportedReport, fileExportAbout } from '../../../../src/file-export-parts'
import { fileImportAbout, TIDY_HINT, TIDY_LABEL, tidyQuestion } from '../../../../src/file-import-parts'
import { gauthAbout, importLabel, progressText } from '../../../../src/gauth-import-parts'
import { anyCriterion, criteriaChecks, leaveSite, mergedCard, NO_CRITERION, NO_SIMILAR, NONE_TICKED, SIMILAR_AFTER_IMPORT, SIMILAR_INTRO, similarCriteria, similarHint, similarSites, type Site, siteChoice } from '../../../../src/similar-parts'
import { api, type Exported, type ExportFormat, type GauthProgress, type Imported, type ImportStep, type Listing, type Similar } from './api'
import { icon, listScreen } from './list'
import { confirmSheet, iconButton, screen, screens, sheet, show, snack } from './page'
import { browsePhone, onPhone } from './phone-files'

// Tools (#183): import from another app, a file or Google Authenticator,
// export to a file, and similar entries.

/** Tools (#183): the commands that work on the whole database, from the toolbar's wrench. */
export function toolsSheet(listing: Listing) {
  sheet((close) => [
    el('b', {}, 'Tools'),
    button('Import from another app…', 'Passwords and passkeys from another password manager on this phone', () => (close(), importSheet()), 'item'),
    button('Import from a file…', 'A file another password manager exported (CSV, Bitwarden JSON)', () => (close(), fileImportSheet()), 'item'),
    button('Import from Google Authenticator…', 'The accounts of its export, read from its QR codes', () => (close(), gauthSheet()), 'item'),
    button('Export to a file…', 'A file another password manager imports (CSV, CXF JSON)', () => (close(), fileExportSheet()), 'item'),
    button('Find similar entries', 'Entries alike in title, user name or site, to merge them', () => (close(), void similarScreen(listing)), 'item'),
    // Only while entries are in a group an older import made (#335).
    ...(listing.inImportGroups
      ? [button(TIDY_LABEL, TIDY_HINT, () => {
          close()
          confirmSheet(tidyQuestion(listing.inImportGroups), 'Tidy up', () =>
            void api.tidyImported().then((next) => (listScreen(next), snack('Imported folders tidied up')), (e) => snack(String(e))))
        }, 'item')]
      : []),
  ])
}

/** Import from Google Authenticator (#281): the export's QR codes read one
 *  by one (the camera, or a screenshot), then their accounts imported. The
 *  codes read go when the sheet closes, imported or not. */
function gauthSheet() {
  void api.gauthStart()
  sheet((close) => {
    // Redrawn in place as codes are read, so the sheet stays open.
    const body = el('div', { className: 'sheet-body' })
    const read = async (camera: boolean) => {
      const next = await api.gauthRead(camera)
      if (next) draw(next)
    }
    const draw = (progress: GauthProgress | null) => {
      body.replaceChildren(
        el('b', {}, 'Import from Google Authenticator'),
        ...gauthAbout('Read each code with the camera, or pick a screenshot of it.'),
        ...(progress ? [el('p', { className: 'progress' }, progressText(progress))] : []),
        busyButton(progress ? 'Next code with the camera' : 'With the camera', 'Point the camera at the code', () => read(true), (message) => snack(message)),
        busyButton('From a picture…', 'A screenshot of the code', () => read(false), (message) => snack(message)),
        ...(progress?.accounts
          ? [busyButton(importLabel(progress), 'Import the accounts read', async () => {
              const done = await api.gauthImport()
              close()
              showImported(done)
            }, (message) => snack(message), 'primary')]
          : []),
        button('Cancel', 'Let the codes read go', close, 'link'))
    }
    draw(null)
    return [body]
  }, () => void api.gauthCancel())
}

/** Find similar (#215): the entries that are for the same site and account, as
 *  on Windows (docs/spec.md, *Similar entries*). Per section one is kept (the
 *  most recently changed, at first) and the others are ticked. Proceed shows the
 *  kept one as the merge would leave it, with Merge and Cancel (back to the
 *  sections, ticks as they were); Leave as is takes the section off until the
 *  screen is opened again from Tools (`left` keeps that while it shows again
 *  after a change). */
async function similarScreen(listing: Listing, left = new Set<string>()) {
  let found: Similar[]
  try {
    found = anyCriterion() ? await api.similarEntries(similarCriteria()) : []
  } catch (e) {
    snack(String(e))
    return
  }
  const toList = () => listScreen(listing)
  /** The screen count once this one shows (see show()). */
  let shown = 0
  /** This screen again, from `fresh`, unless another one came meanwhile. */
  const reshow = (fresh: Listing) => {
    if (screens.current === shown) void similarScreen(fresh, left)
  }
  /** Shows `nodes` as this screen (the sites, or a merge's card); Back goes to `back`. */
  const display = (nodes: Node[], back: () => void) => {
    show(nodes, back, 'similar')
    shown = screens.current
    // Entries another device changed: the sites again, from the list as it is now.
    screens.onSynced((synced) => {
      if (synced.changed) void api.listing().then(reshow)
    })
  }
  const failed = (e: unknown) => snack(String(e))
  const quote = (title: string) => `“${title}”`
  const none = () => el('p', { className: 'muted' }, anyCriterion() ? NO_SIMILAR : NO_CRITERION)

  const site = (site: Site) => {
    const choice = siteChoice(site, (entry, tick, keep) =>
      el('li', {},
        tick,
        icon(entry, listing),
        el('span', {}, el('b', {}, titleOf(entry)), el('small', {}, similarHint(entry))),
        el('label', { className: 'keep' }, keep, 'Keep')))
    const proceed = async () => {
      const others = choice.toMerge()
      if (!others.length) return snack(NONE_TICKED)
      const kept = choice.kept()
      const into = quote(titleOf(kept))
      const ids = others.map((e) => e.id)
      const preview = await api.mergePreview(kept.id, ids)
      const sites = [...screen.childNodes]
      const cancel = () => display(sites, toList)
      display([
        el('header', { className: 'bar' }, iconButton('back', 'Back to the similar entries', cancel), el('h1', {}, `Merge into ${into}`)),
        el('p', { className: 'muted' }, `${into} as the merge would leave it. ${describeEntries(others, quote)} ${others.length === 1 ? 'goes' : 'go'} to the recycle bin.`),
        mergedCard(preview, icon(kept, listing)),
        busyButton('Merge', 'Merge them into it', () =>
          api.mergeEntries(kept.id, ids).then((fresh) => (snack(`Merged into ${into}`), reshow(fresh))), failed, 'primary'),
        button('Cancel', 'Back to the similar entries', cancel, 'wide'),
      ], cancel)
    }
    const section: HTMLElement = el('section', {},
      el('h2', {}, site.name),
      el('ul', { className: 'entries' }, ...choice.rows),
      el('div', { className: 'similar-actions' },
        busyButton('Proceed', 'See the kept entry as the merge would leave it', proceed, failed),
        button('Leave as is', 'Leave these entries as they are', () => {
          left.add(site.name)
          leaveSite(screen, section, none)
        }, 'link')))
    return section
  }

  const byId = new Map(listing.entries.map((e) => [e.id, e]))
  const sites = similarSites(found, (id) => byId.get(id), left)
  display([
    el('header', { className: 'bar' }, iconButton('back', 'Back to the list', toList), el('h1', {}, 'Similar entries')),
    el('p', { className: 'muted' }, SIMILAR_INTRO),
    // A changed tick scans again (#267); Leave as is holds until then.
    criteriaChecks(() => void similarScreen(listing)),
    ...(sites.length ? sites.map(site) : [none()]),
  ], toList)
}

/** Importing from another password manager on this phone (#152): what it does, then Android's list of apps. */
function importSheet() {
  sheet((close) => [
    el('b', {}, 'Import from another app'),
    el('p', {}, 'Android lists the password managers on this phone that can hand over their passwords. Everything the one you choose hands over is added as new entries: nothing here is changed or replaced.'),
    el('p', { className: 'muted' }, 'Passwords, passkeys, TOTP secrets and notes come over; attached files do not. The entries get a tag of their own, and show under All.'),
    button('Choose the app', 'Choose the app to import from', () => (close(), void runImport()), 'primary'),
    button('Cancel', 'Cancel', close, 'link'),
  ])
}

/** Imports with `importing`; the sheet after it ends with `after`, if given. */
async function runImport() {
  showImported(await api.importFromApp().catch((e) => (snack(String(e)), null)))
}

/** A file's import done, or its password asked for (an encrypted Aegis
 *  export, #282): the file waits in the backend until it is imported or the
 *  sheet is closed; a wrong password can be typed again. */
function afterFileImport(step: ImportStep | null) {
  if (step?.step !== 'password') return showImported(step)
  sheet((close) => {
    const password = el('input', { type: 'password', autocomplete: 'off', placeholder: 'The export’s password', className: 'field' })
    const error = errorLine()
    error.hideOnInput(password)
    const importIt = busyButton('Import', `Import ${step.file}`, async () => {
      try {
        const done = await api.importFileWithPassword(password.value)
        close()
        afterFileImport(done)
      } catch (e) {
        error.show(String(e))
        password.select()
      }
    }, (message) => error.show(message), 'primary')
    enterPresses(importIt, password)
    setTimeout(() => password.focus())
    return [el('b', {}, 'The export’s password'), el('p', {}, `${step.file} is encrypted: type the password it was exported with.`),
      password, error.line, importIt, button('Cancel', 'Do not import this file', close, 'link')]
  }, () => void api.cancelImport())
}

/** The list with what an import brought, and the sheet saying so. */
function showImported(imported: Imported | null) {
  if (!imported) return
  if (imported.added) listScreen(imported.listing)
  importedSheet(imported)
}

const isExportName = (name: string) => /\.(csv|json)$/i.test(name)

/** Import from a file (#238): what it takes, then the phone's folders (All
 *  files access, #207) or Android's file picker. */
function fileImportSheet() {
  sheet((close) => [
    el('b', {}, 'Import from a file'),
    ...fileImportAbout(),
    button('Choose the file', 'Choose the file to import', () => {
      close()
      onPhone<ImportStep>('The file to import', (done, picker) => browsePhone({ first: isExportName, pick: api.importFileAt, done, picker }), api.importFromFile)
        .then(afterFileImport, (e) => snack(String(e)))
    }, 'primary'),
    button('Cancel', 'Cancel', close, 'link'),
  ])
}

/** Export to a file (#240): the format, the user checked again (the
 *  fingerprint, or the master password), then where the file goes: the
 *  phone's folders (All files access, #207) or Android's save picker. */
function fileExportSheet() {
  sheet((close) => [
    el('b', {}, 'Export to a file'),
    ...fileExportAbout(),
    ...EXPORT_FORMATS.map(([format, name]) => button(name, `Export as ${name}`, () => (close(), void confirmExport(() => void exportAs(format))), 'item')),
    button('Cancel', 'Cancel', close, 'link'),
  ])
}

/** The fingerprint, or the master password; `then` once the export is allowed. */
async function confirmExport(then: () => void) {
  if (await api.biometricReady().catch(() => false)) {
    try {
      await api.confirmExport(null)
      return then()
    } catch (e) {
      if (String(e) === 'cancelled') return
      // The fingerprint could not: the master password instead.
    }
  }
  sheet((close) => {
    const password = el('input', { type: 'password', className: 'field', ariaLabel: 'Master password' })
    const error = el('p', { className: 'error', hidden: true })
    const ok = busyButton('Export', 'Export', async () => {
      await api.confirmExport(password.value)
      close()
      then()
    }, (message) => {
      error.textContent = message
      error.hidden = false
    }, 'primary')
    enterPresses(ok, password)
    setTimeout(() => password.focus())
    return [el('b', {}, 'Enter the master password to export'), password, error, ok, button('Cancel', 'Cancel', close, 'link')]
  })
}

/** Where the file goes, then what was written. */
async function exportAs(format: ExportFormat) {
  try {
    const name = await api.exportName(format)
    const exported = await onPhone<Exported>('Where the file goes',
      (done, picker) => browsePhone({ first: () => false, makeName: name, make: (folder, as) => api.exportAt(format, folder, as), done, picker }),
      () => api.exportTo(format))
    if (exported) sheet((close) => [el('b', {}, 'Exported'), ...exportedReport(exported), button('OK', 'OK', close, 'primary')])
  } catch (e) {
    snack(String(e))
  }
}

/** What an import brought, and what it could not; after a file, deleting it
 *  (one read by its path), or a reminder to (the app may not delete another
 *  app's file). */
function importedSheet({ added, exporter, tag, skipped, fromFile, deletable }: Imported) {
  sheet((close) => [
    el('b', {}, `${added} ${added === 1 ? 'entry' : 'entries'} from ${exporter}`),
    el('p', {}, added ? `Tagged “${tag}”, and shown under All.` : 'Nothing was added.'),
    ...(added ? [el('p', { className: 'muted' }, SIMILAR_AFTER_IMPORT)] : []),
    ...(skipped.length
      ? [el('h2', {}, `Not brought over (${skipped.length})`), el('ul', { className: 'skipped' }, ...skipped.map((s) => el('li', {}, el('b', {}, s.title || 'Untitled'), `: ${s.why}`)))]
      : []),
    ...(deletable
      ? [el('p', {}, `${exporter} holds every password in clear text. Delete it now that its entries are here?`),
          button('Delete the file', `Delete ${exporter}`, () => {
            close()
            api.deleteImportedFile().then(() => snack(`${exporter} deleted`), (e) => snack(String(e)))
          }, 'item danger'),
          button('Keep it', 'Keep the file', close, 'link')]
      : [...(fromFile ? [el('p', { className: 'muted' }, 'The file holds every password in clear text: delete it now, in the app you keep it in.')] : []),
          button('OK', 'OK', close, 'primary')]),
  ])
}
