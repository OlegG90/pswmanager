import { busyButton, button, el, enterPresses, errorLine } from '../../../../src/dom'
import { showHide } from '../../../../src/editor-parts'
import { api, type Listing, type Status } from './api'
import { chooseScreen } from './first-run'
import { listScreen } from './list'
import { app, confirmSheet, iconButton, screens, show } from './page'
import { chooseKeyFile } from './phone-files'
import { applySync } from './sync'

// The unlock screen, and locking.

/** The list after an unlock; the sync may have finished before it was there to hear it. */
export async function unlockedWith(opened: Listing) {
  app.unlocked = true
  listScreen(opened)
  // Offered for export to other password managers (#153); a phone without
  // Android's transfer (no Google Play services) simply does not list it.
  if (app.database) void api.registerExport(app.database.title).catch(() => {})
  const last = await api.lastSync()
  if (last) applySync(last)
}

export function unlockScreen() {
  const current = app.database!
  const error = errorLine()
  const password = el('input', { type: 'password', autocomplete: 'off', placeholder: 'Master password', className: 'field' })
  // Biometric unlock, when its key is sealed: the fingerprint button, or Unlock
  // with no password typed, once (again, it unlocks with the key file alone).
  let prompted = false
  const unlock = busyButton('Unlock', 'Unlock the database', async () => {
    if (!password.value && !fingerprint.hidden && !prompted) return withBiometric()
    app.syncLine = 'Syncing…'
    const opened = await api.unlock(password.value)
    password.value = ''
    await unlockedWith(opened)
  }, error.show, 'primary')
  const withBiometric = async () => {
    prompted = true
    try {
      app.syncLine = 'Syncing…'
      await unlockedWith(await api.unlockWithBiometric())
    } catch (e) {
      if (String(e) !== 'cancelled') error.show(String(e))
      password.focus()
    }
  }
  const fingerprint = iconButton('fingerprint', 'Unlock with your fingerprint or face', () => void withBiometric(), 'icon fingerprint')
  fingerprint.hidden = true
  enterPresses(unlock, password)
  error.hideOnInput(password)
  const forget = () =>
    confirmSheet(`Forget “${current.title}”? Its file stays where it is.`, 'Forget it', async () => {
      try {
        app.database = (await api.forgetDatabase()).database
        chooseScreen()
      } catch (e) {
        error.show(String(e))
      }
    })
  const showAgain = (status: Status) => {
    app.database = status.database
    unlockScreen()
  }
  const keyLine = current.keyFile
    ? el('p', { className: 'muted' }, `Key file: ${current.keyFile} `, button('Remove', 'Unlock without a key file', () => void api.clearKeyFile().then(showAgain, error.show), 'link'))
    : button('Use a key file…', 'For a database set up with one: a .keyx or .key file kept apart from it', () =>
      chooseKeyFile(false, (picked) => void api.useKeyFile(picked).then(showAgain, error.show), error.show), 'link')
  show([
    el('h1', {}, current.title),
    el('p', { className: 'muted' }, current.description),
    el('p', { className: 'muted' }, current.syncedWith ? `Syncs with ${current.syncedWith}` : ''),
    el('div', { className: 'together unlock-row' }, password, showHide(password), fingerprint),
    keyLine,
    unlock,
    error.line,
    button('Use another database…', 'Forget this one (its file stays where it is)', forget, 'link'),
  ])
  password.focus()
  // Coming back from another app to this screen opens the prompt by itself
  // (the lock may have happened while away, just before the screen was shown).
  const prompt = (ready: boolean) => {
    if (ready && fingerprint.isConnected && app.justReturned()) {
      // Shown before the app is fully back, Android would cancel it.
      window.setTimeout(() => fingerprint.isConnected && void withBiometric(), 400)
    }
  }
  void api.biometricReady().then((ready) => {
    fingerprint.hidden = !ready
    prompt(ready)
    screens.onReturn(() => prompt(!fingerprint.hidden))
  }, () => {})
}

export async function lock() {
  await api.lock()
  afterLock()
}

/** The page locked the database: nothing of it stays on the page. */
export function afterLock() {
  app.locked(true)
  unlockScreen()
}
