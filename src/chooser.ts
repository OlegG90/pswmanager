import { listen } from '@tauri-apps/api/event'
import { api, type AutoTypeAsk, type Entry, type Status } from './api'
import { typeChoice, typeTarget } from './auto-type'
import { button, el, errorLine } from './dom'

// Auto-Type's chooser (#307): a window of its own over the window the hotkey
// was pressed in. It unlocks the database if it must, then asks which entry
// to type there; the main window is left as it is.

const root = document.getElementById('chooser')!

/** What the chooser shows; `idle` once it has gone, `busy` while unlocking. */
let state: 'idle' | 'busy' | 'unlock' | 'choose' | 'message' = 'idle'

/** The chooser's look: what it is about, what it asks, and its buttons with Cancel last. */
function draw(message: string, body: Node[], buttons: Node[], focus?: HTMLElement) {
  const cancelButton = button('Cancel', 'Cancel (Esc)', cancel)
  root.replaceChildren(
    el('div', { className: 'content' }, el('p', {}, message), ...body),
    el('div', { className: 'buttons' }, ...buttons, cancelButton),
  )
  ;(focus ?? cancelButton).focus()
}

/** Nothing waits to be typed into any more, and the chooser goes. */
function cancel() {
  state = 'idle'
  root.replaceChildren()
  api.cancelAutoType().catch(() => {})
}

/** Shows what the window the hotkey was pressed in needs now. */
async function refresh() {
  try {
    const status = await api.status()
    if (!status.unlocked) return showUnlock(status)
    const ask = await api.autoTypeWaiting()
    if (!ask) return cancel()
    showChoice(ask, (await api.listing()).entries)
  } catch (e) {
    showMessage(String(e))
  }
}

function showMessage(message: string) {
  state = 'message'
  draw(message, [], [])
}

function showChoice(ask: AutoTypeAsk, entries: Entry[]) {
  state = 'choose'
  const { body, buttons, focus } = typeChoice(ask, entries, (choice) => {
    // The chooser steps aside to type, and comes back when typing failed.
    state = 'idle'
    api.autoType(choice.id, choice.typed).catch((e) => showMessage(String(e)))
  })
  draw(typeTarget(ask), body, buttons, focus)
}

/** The master password and key file, for the database the main window's
 *  unlock screen opens; the key file is the one chosen for it there, or here. */
async function showUnlock(status: Status) {
  if (!status.database) return showMessage('Choose a database in PswManager first, then press the Auto-Type hotkey again')
  state = 'unlock'
  const current = status.databases.find((d) => d.file === status.database)
  const password = el('input', { id: 'password', type: 'password', ariaLabel: 'Master password' })
  const error = errorLine()
  error.hideOnInput(password)
  const hello = button('Windows Hello', 'Unlock with Windows Hello: face, fingerprint or PIN', () => unlockWith(api.unlockWithHello))
  hello.hidden = true
  const keyFile = el('span', { className: 'path muted' })
  const showKeyFile = (path: string | null) => {
    keyFile.textContent = keyFile.title = path ?? 'No key file'
  }
  showKeyFile(status.keyFile)
  const pickKeyFile = button('Key file…', 'A KeePass key file (.keyx or .key), needed only if this database was set up with one', async () => {
    try {
      showKeyFile((await api.pickKeyFile()).keyFile)
    } catch (e) {
      error.show(String(e))
    }
  })
  const form = el('form', { className: 'unlock', autocomplete: 'off' },
    el('label', { className: 'field' }, el('span', {}, 'Master password'), password),
    el('div', { className: 'file-row' }, keyFile, pickKeyFile),
    error.line,
  )
  form.addEventListener('submit', (e) => {
    e.preventDefault()
    const typed = password.value
    password.value = ''
    void unlockWith(() => api.unlock(typed))
  })
  // Enter in the field submits; the Unlock button sits with the others, outside the form.
  const unlock = button('Unlock', 'Unlock (Enter)', () => form.requestSubmit(), 'primary')
  draw(`Unlock ${current?.name ?? 'the database'} to type into the window`, [form], [unlock, hello], password)

  /** Unlocks, one way at a time, then asks for the window; a failure says
   *  why, closing Windows Hello's prompt nothing. */
  async function unlockWith(open: () => Promise<unknown>) {
    if (state !== 'unlock') return
    state = 'busy'
    try {
      await open()
    } catch (e) {
      state = 'unlock'
      if (String(e) !== 'cancelled') error.show(String(e))
      return password.focus()
    }
    await refresh()
  }

  // Windows Hello asks at once, as the main window's unlock screen does.
  if (await api.helloReady().catch(() => false)) {
    hello.hidden = false
    if (state === 'unlock' && !password.value) void unlockWith(api.unlockWithHello)
  }
}

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    e.preventDefault()
    cancel()
  }
})

// The hotkey was pressed again, with the chooser made before. While it
// unlocks, the window that waits is asked for once unlocked.
listen('auto-type-waiting', () => {
  if (state !== 'unlock' && state !== 'busy') void refresh()
})
// Unlocked in the main window meanwhile.
listen('unlocked', () => {
  if (state === 'unlock') void refresh()
})
// A lock takes the window that waited: there is nothing left to choose.
listen('locked', () => {
  if (state === 'choose' || state === 'message') cancel()
})

void refresh()
