import { busyButton, button, el, errorLine } from '../../../../src/dom'
import { input, showHide, strengthMeter } from '../../../../src/editor-parts'
import { masterPasswordProblem, masterRulesTip } from '../../../../src/master-password'
import { api, type Cloud, type CloudFile, type Picked, type Status } from './api'
import { app, iconButton, screen, show, snack } from './page'
import { browsePhone, onPhone, phoneFolder } from './phone-files'
import { CLOUDS, filesIn, signIn, STORES } from './sync'
import { unlockScreen } from './unlock'

// ------------------------------------------------------------ first run

export function chooseScreen() {
  const error = errorLine()
  const opened = (status: Status | null) => {
    if (!status) return
    app.database = status.database
    unlockScreen()
  }
  // Android's file picker, or the phone's folders browsed here (#207).
  const local = busyButton('Open a local file', 'A .kdbx on this phone or an SD card', async () => opened(await onPhone('Where the database is',
    (done, picker) => browsePhone({ first: (n) => /\.kdbx$/i.test(n), pick: api.openDatabaseAt, done, picker }),
    api.openLocalFile)), error.show, 'card')
  show([
    el('p', { className: 'muted' }, 'First run'),
    el('h1', {}, 'Choose your database'),
    el('p', {}, 'PswManager opens one KeePass (KDBX 4) file. The same file opens in PswManager on Windows and in KeePassXC.'),
    ...CLOUDS.map((cloud) => el('button', { type: 'button', className: 'card', onclick: () => void cloudScreen(cloud) }, `Sync with ${STORES[cloud].name}`,
      el('small', {}, `Sign in once. The file lives in ${STORES[cloud].folder}; a working copy stays on this phone.`))),
    local,
    el('button', { type: 'button', className: 'card', onclick: () => newDatabaseScreen() }, 'Create a new database',
      el('small', {}, 'With a master password; it goes to a cloud store or a folder on this phone.')),
    el('p', { className: 'muted' }, 'A local file is picked with Android’s file picker; it can be anywhere the picker reaches, also a folder another app syncs.'),
    error.line,
  ])
}

/** A new database: its name and master password, then where it lives (a
 *  cloud store, with a copy in a folder on this phone, or a folder). */
function newDatabaseScreen() {
  const error = errorLine()
  const name = input('Passwords', { className: 'field', placeholder: 'Name' })
  const password = el('input', { type: 'password', autocomplete: 'new-password', placeholder: 'Master password', className: 'field' })
  const again = el('input', { type: 'password', autocomplete: 'new-password', placeholder: 'The master password again', className: 'field' })
  const strength = strengthMeter(password, api.passwordStrength)
  error.hideOnInput(name)
  error.hideOnInput(password)
  error.hideOnInput(again)
  const ready = () => {
    if (!name.value.trim()) throw new Error('Give the database a name')
    if (!password.value) throw new Error('Choose a master password')
    const weak = masterPasswordProblem(password.value)
    if (weak) throw new Error(weak)
    if (password.value !== again.value) throw new Error('The two passwords differ')
  }
  const create = async (place: Parameters<typeof api.createDatabase>[2]) => {
    const made = await api.createDatabase(name.value, password.value, place)
    password.value = again.value = ''
    app.database = made.status.database
    unlockScreen()
    if (made.copyProblem) snack(`${made.copyProblem}. Choose the folder again in the sync sheet.`)
  }
  const fail = (e: string) => error.show(e.replace(/^Error: /, ''))
  const inCloud = (cloud: Cloud) =>
    busyButton(STORES[cloud].name, `Put it into ${STORES[cloud].folder} in ${STORES[cloud].name}`, async () => {
      ready()
      // Signed in (the store is asked for its files), then the copy's folder.
      await filesIn(cloud)
      snack('Choose the folder for the copy on this phone')
      const picked = await phoneFolder('The folder for the copy on this phone')
      if (picked) await create({ kind: 'cloud', cloud, folder: picked })
    }, fail, 'card')
  const onPhone = busyButton('On this phone', 'A local file in a folder you choose', async () => {
    ready()
    const picked = await phoneFolder('The folder for the database')
    if (picked) await create({ kind: 'folder', folder: picked })
  }, fail, 'card')
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Back', chooseScreen), el('h1', {}, 'New database')),
    name,
    el('div', { className: 'together' }, password, showHide(password)),
    again,
    strength.element,
    masterRulesTip(password),
    el('h2', {}, 'Where it lives'),
    ...CLOUDS.map(inCloud),
    onPhone,
    el('p', { className: 'muted' }, 'A file of the same name already there is never replaced. A key file can be added later in PswManager for Windows.'),
    error.line,
  ], chooseScreen)
  name.focus()
}

/** First run with a cloud store: sign in, then pick the file in the app's folder. */
async function cloudScreen(cloud: Cloud) {
  const { name } = STORES[cloud]
  const error = errorLine()
  const back = iconButton('back', 'Back', chooseScreen)
  const waiting = el('p', {}, `Waiting for ${name}…`)
  show([el('header', { className: 'bar' }, back, el('h1', {}, name)), el('p', { className: 'muted' }, 'Sign-in opens in the browser. Nothing is uploaded.'), waiting, error.line], chooseScreen)
  try {
    await signIn(cloud)
    filesScreen(cloud, await api.cloudFiles(cloud))
  } catch (e) {
    waiting.remove()
    error.show(String(e))
    screen.append(busyButton(`Sign in to ${name}`, 'Sign in again', () => cloudScreen(cloud), error.show, 'primary'))
  }
}

function filesScreen(cloud: Cloud, files: CloudFile[]) {
  const { name, folder } = STORES[cloud]
  const error = errorLine()
  const pick = (file: CloudFile) => button(file.name, `Use ${file.name}`, () => folderScreen(cloud, files, file), 'card')
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Back', chooseScreen), el('h1', {}, 'Pick the file')),
    el('p', { className: 'muted' }, folder),
    ...(files.length ? files.map(pick) : [el('p', {}, 'There is no .kdbx in the app’s folder yet. Put one there from PswManager on Windows (Settings → Sync → Upload) and come back.')]),
    el('p', { className: 'muted' }, `PswManager sees only its app folder in ${name}. PswManager for Windows opens the same file there.`),
    error.line,
  ], chooseScreen)
}

/** First run with a cloud store, step 3: where the visible copy goes on this phone. */
function folderScreen(cloud: Cloud, files: CloudFile[], file: CloudFile) {
  const error = errorLine()
  let folder: Picked | null = null
  const chosen = el('p', {}, 'No folder chosen yet.')
  const taken = el('p', { className: 'muted' })
  const go = busyButton('Download and continue', 'Download the database', async () => {
    const opened = await api.openCloudFile(cloud, file, folder!)
    app.database = opened.status.database
    unlockScreen()
    if (opened.copyProblem) snack(`${opened.copyProblem}. Choose the folder again in the sync sheet.`)
  }, error.show, 'primary')
  go.disabled = true
  const choose = busyButton('Choose a folder…', 'The folder for the copy on this phone', async () => {
    const picked = await phoneFolder('The folder for the copy on this phone')
    if (!picked) return
    folder = picked
    chosen.textContent = `Folder: ${picked.name}`
    taken.textContent = (await api.copyNameTaken(picked.uri, file.name)) ? `A ${file.name} is already in this folder. It will be kept as ${file.name}.bak.` : ''
    go.disabled = false
  }, error.show)
  const back = () => filesScreen(cloud, files)
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Back', back), el('h1', {}, 'Where to keep it')),
    el('p', {}, `${file.name} syncs with ${STORES[cloud].name}. A copy of it is kept in a folder on this phone, where you can see it and back it up; it is updated after every change.`),
    choose,
    chosen,
    taken,
    go,
    error.line,
  ], back)
}
