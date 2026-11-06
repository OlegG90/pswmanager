import { listen } from '@tauri-apps/api/event'
import { busyButton, button, el, enterPresses, errorLine } from '../../../../src/dom'
import { api, type Cloud, type CloudFile, type SignedIn, type Synced } from './api'
import { listScreen } from './list'
import { app, iconButton, screens, sheet, show, snack } from './page'
import { keyFileChooser, phoneFolder } from './phone-files'

// The page's side of sync: signing in to a store, Sync now, pull to sync, the
// status line's sheet, and the key another device changed the database to.

/** What to do when a sign-in ends (event `signed-in`). */
let onSignedIn = (_signedIn: SignedIn) => {}
void listen<SignedIn>('signed-in', (e) => onSignedIn(e.payload))

/** The stores the phone syncs with, and where the app's files are in each. */
export const CLOUDS: Cloud[] = ['dropbox', 'onedrive', 'google']
export const STORES: Record<Cloud, { name: string; folder: string }> = {
  dropbox: { name: 'Dropbox', folder: 'Apps / PswManager Sync' },
  onedrive: { name: 'OneDrive', folder: 'Apps / PswManager' },
  google: { name: 'Google Drive', folder: 'PswManager, at the top of the Drive' },
}

/** Signs in to `cloud` in the browser, then waits for it to come back. A
 *  sign-in started again gives up on the one before. */
export function signIn(cloud: Cloud): Promise<void> {
  onSignedIn({ error: 'A new sign-in was started' })
  return new Promise((done, fail) => {
    onSignedIn = (signedIn) => {
      onSignedIn = () => {}
      if (signedIn.error) fail(signedIn.error)
      else done()
    }
    api.signIn(cloud).catch(fail)
  })
}

/** The databases in `cloud`'s app folder, signing in first when the store asks. */
export async function filesIn(cloud: Cloud): Promise<CloudFile[]> {
  try {
    return await api.cloudFiles(cloud)
  } catch {
    await signIn(cloud)
    return api.cloudFiles(cloud)
  }
}

/** Picks the folder for the visible copy and writes it there. */
async function chooseCopyFolder() {
  try {
    const folder = await phoneFolder('The folder for the copy on this phone')
    if (!folder) return
    app.database = (await api.setCopyFolder(folder)).database
    snack(`The copy is kept in ${folder.name}`)
  } catch (e) {
    snack(String(e))
  }
}

/** The toolbar's Sync now; without a store it looks disabled and, tapped, says
 *  where sync is set up. */
export function syncButton(started: () => void) {
  const notSetUp = 'Sync is not set up: Settings → Sync'
  const on = app.linked()
  const b = iconButton('sync', on ? 'Sync now' : notSetUp, () => {
    if (!on) return snack(notSetUp)
    started()
    void api.syncNow()
  })
  if (!on) b.setAttribute('aria-disabled', 'true')
  return b
}

/** Pulling the list down from its top syncs, with a store linked. */
export function pullToSync(scroll: HTMLElement, started: () => void) {
  let from: number | null = null
  scroll.addEventListener('touchstart', (e) => (from = app.linked() && scroll.scrollTop === 0 ? e.touches[0].clientY : null), { passive: true })
  scroll.addEventListener('touchend', (e) => {
    if (from !== null && e.changedTouches[0].clientY - from > 80) {
      started()
      void api.syncNow()
    }
    from = null
  })
}

/** What a sync did: kept, then the screen shown answers it. */
export function applySync(synced: Synced) {
  app.synced(synced)
  screens.runSynced(synced)
}

/** Signs in to `cloud` again (the store asked), then syncs. */
const signInButton = (cloud: Cloud, close: () => void) =>
  button('Sign in', 'Sign in again', () => (close(), void signIn(cloud).then(() => api.syncNow(), (e) => snack(String(e)))), 'primary')

/** The status line's sheet: what the last sync did, and Sync now with a store linked. */
export function syncSheet() {
  sheet((close) => [
    el('b', {}, 'Sync'),
    el('p', {}, app.syncLine || 'Not synced yet'),
    el('p', { className: 'muted' }, app.database?.syncedWith ? `With ${app.database.syncedWith}` : ''),
    ...(app.copyProblem ? [el('p', { className: 'error' }, app.copyProblem)] : []),
    ...(app.database?.cloud
      ? [
          el('p', { className: 'muted' }, app.database.copyFolder ? `Copy on this phone: ${app.database.copyFolder}` : 'No copy on this phone yet'),
          button(app.database.copyFolder ? 'Move the copy…' : 'Keep a copy on this phone…', 'Choose the folder', () => {
            close()
            void chooseCopyFolder()
          }, 'link'),
        ]
      : []),
    ...(app.signInAgain && app.database?.cloud ? [signInButton(app.database.cloud, close)] : []),
    ...(app.otherKey ? [button('Enter the new key…', 'The master password and / or key file another device changed it to', () => (close(), void otherKeyScreen()), 'primary')] : []),
    ...(app.linked()
      ? [button('Sync now', 'Sync now', () => {
          close()
          void api.syncNow()
        }, 'primary')]
      : []),
  ])
}

/** The key another device changed the database to (#193): the remote file is
 *  synced with it; once it opens, the phone unlocks with it. As on Windows. */
export async function otherKeyScreen() {
  // Which key file the phone uses, as it is now.
  app.database = (await api.status()).database
  const error = errorLine()
  const password = el('input', { type: 'password', autocomplete: 'current-password', placeholder: 'The new master password', className: 'field' })
  error.hideOnInput(password)
  const keyFile = keyFileChooser((e) => error.show(e))
  const notNow = () => {
    password.value = ''
    void api.listing().then(listScreen)
  }
  const use = busyButton('Use this key', 'Sync with the remote file using this key', async () => {
    const status = await api.enterOtherKey(password.value, keyFile.value())
    password.value = ''
    app.database = status.database
    app.otherKey = false
    snack('The new key is in use')
    listScreen(await api.listing())
  }, (e) => error.show(e.replace(/^Error: /, '')), 'primary')
  enterPresses(use, password)
  show([
    el('header', { className: 'bar' }, iconButton('back', 'Back', notNow), el('h1', {}, 'New key')),
    el('p', {}, 'Another device changed the master password or key file. Enter the new one to sync again. Until then this phone works with its own copy, and its changes do not go up.'),
    password,
    el('h2', {}, 'Key file'),
    ...keyFile.nodes,
    error.line,
    use,
    button('Not now', 'Keep working with this phone’s copy; the key can be entered later', notNow, 'link'),
  ], notNow)
  password.focus()
}
