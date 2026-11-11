import { button, el, enterPresses, errorLine } from '../../../../src/dom'
import { input } from '../../../../src/editor-parts'
import { api, type NewKeyFile, type Picked } from './api'
import { app, sheet, snack } from './page'

// Files and folders on the phone (#207): browsed here with All files access,
// or through Android's pickers.

/** Something on the phone the app opens or writes (#207): with All files
 *  access on, the phone's folders are browsed here (`browse`, which hands back
 *  what was chosen, or null); otherwise a sheet offers Android's `picker`
 *  (which refuses some folders, one named Data, say) or turning the access
 *  on. Null when nothing was chosen. */
export function onPhone<T>(what: string, browse: (done: (it: T | null) => void, picker: () => void) => void, picker: () => Promise<T | null>): Promise<T | null> {
  return new Promise((resolve, reject) => {
    // Browsing reaches the phone's own storage: Android's picker stays for an SD card, say.
    const toPicker = () => picker().then(resolve, reject)
    void api.allFilesAccess().then((granted) => {
      if (granted) return browse(resolve, toPicker)
      let handedOn = false
      sheet((close) => [
        el('b', {}, what),
        el('p', { className: 'muted' }, 'Android’s pickers refuse some folders (one named Data, say). To reach those, turn on access to all files for PswManager, then choose again: it opens only what you choose.'),
        button('Android’s picker', 'Choose with Android’s picker', () => {
          handedOn = true
          close()
          picker().then(resolve, reject)
        }, 'item'),
        button('Allow access to all files…', 'Android’s setting for PswManager; come back and choose again', () => {
          close()
          void api.askAllFilesAccess().catch((e) => snack(String(e)))
        }, 'item'),
      ], () => handedOn || resolve(null))
    }, reject)
  })
}

/** A folder on the phone, for a new database or the visible copy. */
export const phoneFolder = (what: string) =>
  onPhone<Picked>(what, (done, picker) => browsePhone({ folder: api.folderAt, done, picker }), api.pickFolder)

/** A key file to choose (`make`: a new one to make) (#207). */
export function chooseKeyFile(make: boolean, done: (keyFile: Picked) => void, fail: (message: string) => void) {
  onPhone<Picked>(make ? 'Where the new key file goes' : 'Where the key file is',
    (finish, picker) => browsePhone({
      ...(make ? { make: api.createKeyFileAt, makeName: NEW_KEY_FILE } : { pick: api.keyFileAt }),
      done: finish,
      picker,
    }),
    () => new Promise((picked) => (make ? newKeyFileInFolder : keyFileFromFolder)(picked, fail)),
  ).then((keyFile) => keyFile && done(keyFile), (e) => fail(String(e)))
}

/** The name a new key file is offered under. */
const NEW_KEY_FILE = 'PswManager.keyx'

/** The phone's files, folder by folder, in a sheet (All files access on).
 *  Rust lists key files first (`is_key_file`); files `first` likes, if
 *  given, go before the rest. `pick` takes the file tapped; `make` makes
 *  one, named here, in the folder shown; `folder` takes the folder shown. `done` gets what they gave, or null when the sheet closed without;
 *  a failure is said in the sheet. */
export function browsePhone<T>(how: {
  first?: (name: string) => boolean
  pick?: (folder: string, name: string) => Promise<T>
  make?: (folder: string, name: string) => Promise<T>
  makeName?: string
  folder?: (path: string) => Promise<T>
  done: (it: T | null) => void
  /** Android's picker instead (an SD card, another volume). */
  picker?: () => void
}) {
  let handedOn = false
  sheet((close) => {
    const where = el('b', {})
    const items = el('div', { className: 'browser' })
    const error = errorLine()
    const name = input(how.makeName ?? '', { className: 'field', ariaLabel: 'Name' })
    error.hideOnInput(name)
    // What was chosen goes before the sheet closes, which would say null;
    // a second tap meanwhile does nothing.
    let busy = false
    const finish = (chosen: () => Promise<T>) => {
      if (busy) return
      busy = true
      void chosen().then((it) => (how.done(it), close()), (e) => error.show(String(e))).finally(() => (busy = false))
    }
    let shown = ''
    const actions = [
      ...(how.make ? [name, button('Make it here', 'Make it in this folder', () => finish(() => how.make!(shown, name.value)), 'primary')] : []),
      ...(how.folder ? [button('Use this folder', 'Use the folder shown', () => finish(() => how.folder!(shown)), 'primary')] : []),
    ]
    const open = (path: string | null) => void api.browse(path).then((folder) => {
      shown = folder.path
      where.textContent = folder.path.replace(/^\/storage\/emulated\/0/, 'Phone')
      error.hide()
      const first = how.first
      const files = first ? [...folder.files].sort((a, b) => Number(first(b)) - Number(first(a))) : folder.files
      items.replaceChildren(
        ...(folder.up ? [button('↑ Up', 'The folder above', () => open(folder.up), 'item')] : []),
        ...folder.folders.map((f) => button(`${f}/`, `Open ${f}`, () => open(`${folder.path}/${f}`), 'item')),
        ...(how.pick ? files.map((f) => button(f, `Use ${f}`, () => finish(() => how.pick!(folder.path, f)), 'item')) : []),
      )
    }, (e) => error.show(String(e)))
    open(null)
    const elsewhere = how.picker
      ? [button('Android’s picker…', 'Somewhere else: an SD card, another volume', () => ((handedOn = true), close(), how.picker!()), 'link')]
      : []
    return [where, items, error.line, ...actions, ...elsewhere, button('Cancel', 'Cancel', close, 'link')]
  }, () => handedOn || how.done(null))
}

/** A key file chosen through a folder (#207): Android's file picker lists
 *  nothing in some folders, its folder picker offers them. The folder is
 *  picked, then one of its files here. */
function keyFileFromFolder(picked: (keyFile: Picked) => void, fail: (message: string) => void) {
  void api.pickKeyFolder().then((found) => {
    if (!found) return
    sheet((close) => [
      el('b', {}, `Key file in ${found.folder.name}`),
      ...(found.files.length
        ? found.files.map((name) => button(name, `Use ${name}`, () => {
          close()
          void api.keyFileIn(found.folder.uri, name).then(picked, (e) => fail(String(e)))
        }, 'item'))
        : [el('p', { className: 'muted' }, 'There are no files in this folder.')]),
      button('Cancel', 'Cancel', close, 'link'),
    ])
  }, (e) => fail(String(e)))
}

/** A new key file made in a folder the user picks, as [keyFileFromFolder]
 *  chooses one: its name offered, never over a file already there. */
function newKeyFileInFolder(made: (keyFile: Picked) => void, fail: (message: string) => void) {
  void api.pickKeyFolder().then((found) => {
    if (!found) return
    sheet((close) => {
      const name = input(NEW_KEY_FILE, { className: 'field', ariaLabel: 'Name' })
      const create = button('Make it', 'Make the key file in this folder', () => {
        close()
        void api.createKeyFileIn(found.folder.uri, name.value).then(made, (e) => fail(String(e)))
      }, 'primary')
      enterPresses(create, name)
      return [el('b', {}, `New key file in ${found.folder.name}`), name, create, button('Cancel', 'Cancel', close, 'link')]
    })
  }, (e) => fail(String(e)))
}

/** The key file a key is to have: the one the database has, another one
 *  chosen or made in a folder, or none. */
export function keyFileChooser(fail: (message: string) => void, offerNew = false) {
  // As the phone keeps it now (the status read when the screen opened).
  const now = app.database?.keyFile ?? null
  let chosen: NewKeyFile = { kind: 'keep' }
  const name = () => (chosen.kind === 'keep' ? now : chosen.kind === 'picked' ? chosen.name : null)
  const shown = el('p', {})
  const none = button('Remove the key file', 'Only the master password opens it then', () => ((chosen = { kind: 'none' }), refresh()), 'wide')
  const refresh = () => {
    shown.textContent = name() ? `Key file: ${name()}` : 'No key file'
    none.hidden = !name()
  }
  const take = (picked: Picked) => {
    chosen = { kind: 'picked', ...picked }
    refresh()
  }
  const pick = button('Choose a key file…', 'A folder, then the key file in it', () => chooseKeyFile(false, take, fail), 'wide')
  refresh()
  // As Windows' New…: a key file the app makes, kept apart from the database.
  const make = button('New key file…', 'A folder, then a new key file made in it; keep it apart from the database', () => chooseKeyFile(true, take, fail), 'wide')
  return { nodes: [shown, pick, ...(offerNew ? [make] : []), none], value: () => chosen, name }
}
