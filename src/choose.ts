import { api, type Status } from './api'
import { button, el } from './dom'
import { choose } from './modal'

type Source = 'local' | 'folder' | 'dropbox'

const SOURCES: [Source, string, string][] = [
  ['local', 'Open a local file', 'A .kdbx on this PC, a USB drive, or a folder another program syncs. PswManager does not sync it.'],
  ['folder', 'Sync with a folder', 'A file on a LAN share such as a NAS. PswManager keeps a working copy and syncs it.'],
  ['dropbox', 'Sync with Dropbox', 'Sign in once; the file lives in Apps/PswManager Sync and opens in Keepass2Android too.'],
]

export interface ChooseOptions {
  /** A database was chosen: the new status. */
  chosen: (status: Status) => void
  /** Back to the unlock screen; only offered when a database is already chosen. */
  back: () => void
}

/** Signs in to Dropbox in the browser, then asks which file to sync with.
 *  Giving up on the way signs out again; null when the user did. */
async function syncWithDropbox(waiting: HTMLElement): Promise<Status | null> {
  waiting.hidden = false
  try {
    const offer = await api.signInToDropbox().finally(() => (waiting.hidden = true))
    const labels = offer.files.map((path) => `Use ${path}`)
    const canUpload = offer.upload !== null && !offer.files.some((p) => p.toLowerCase() === `/${offer.upload}`.toLowerCase())
    if (canUpload) labels.push(`Upload ${offer.upload}`)
    if (!labels.length) throw new Error('The Dropbox app folder has no .kdbx file: open a local file first to upload it')
    const picked = await choose('Which database in the Dropbox app folder should this PC sync with?', labels)
    if (picked === null) {
      await api.cancelDropbox()
      return null
    }
    return await api.syncWithDropbox(picked < offer.files.length ? offer.files[picked] : null)
  } catch (e) {
    await api.cancelDropbox()
    throw e
  }
}

/** True when `next` names another database than `before`: a file dialog
 *  closed with Cancel leaves the status as it was. */
const changed = (before: Status, next: Status) =>
  next.database !== before.database || next.syncedWith !== before.syncedWith

/** Fills `container` with the screen that picks where the database lives. */
export function renderChoose(container: HTMLElement, status: Status, options: ChooseOptions) {
  const first = !status.database
  let source: Source = status.syncKind ?? 'local'

  const error = el('p', { className: 'error', role: 'alert', hidden: true })
  const waiting = el('p', { className: 'muted', hidden: true }, 'Finish signing in to Dropbox in the browser… ',
    button('Cancel', 'Stop waiting for the browser', () => api.cancelDropbox()))
  const fail = (e: unknown) => {
    error.textContent = String(e)
    error.hidden = false
  }

  const sources = SOURCES.map(([value, title, hint]) => {
    const radio = el('input', { type: 'radio', name: 'source', value, checked: value === source })
    radio.addEventListener('change', () => (source = value))
    radio.addEventListener('keydown', (e) => {
      if (e.key !== 'Enter') return
      // Consumed: the unlock screen that follows must not see it as a submit.
      e.preventDefault()
      go()
    })
    return el('label', { className: 'source' }, radio, el('span', { className: 'dot' }),
      el('span', { className: 'text' }, el('span', { className: 'title' }, title), el('span', { className: 'hint' }, hint)))
  })

  const next = button('Continue', 'Continue (Enter)', go, 'primary')
  async function go() {
    if (next.disabled) return
    error.hidden = true
    next.disabled = true
    try {
      const chosen = source === 'dropbox' ? await syncWithDropbox(waiting)
        : changedOrNull(await (source === 'local' ? api.pickDatabase() : api.syncWithFolder()))
      if (chosen?.database) options.chosen(chosen)
    } catch (e) {
      fail(e)
    } finally {
      next.disabled = false
    }
  }
  const changedOrNull = (next: Status) => (changed(status, next) ? next : null)

  const footer = el('div', { className: 'footer' },
    el('span', { className: 'muted' }, 'No database yet? Create one in KeePassXC, or convert a SafeInCloud export with sic2kdbx.'),
    el('div', { className: 'buttons' }, ...(first ? [] : [button('Back', 'Back (Esc)', options.back)]), next))

  const stop = status.syncedWith
    ? [el('p', { className: 'stop muted' }, `Synced with ${status.syncedWith}. `,
        button('Stop syncing', 'Keep using the copy on this PC as a local file', async () => {
          try {
            options.chosen(await api.stopSync())
          } catch (e) {
            fail(e)
          }
        }))]
    : []

  container.replaceChildren(
    el('div', { className: 'intro' },
      el('span', { className: 'kicker' }, first ? 'First run' : 'Database'),
      el('h1', {}, first ? 'Choose your database' : 'Change the database'),
      el('p', { className: 'muted' },
        'PswManager works on one KeePass (KDBX 4) file. Pick where it lives; you can change it later on the unlock screen.')),
    el('div', { className: 'sources', role: 'radiogroup', ariaLabel: 'Where the database lives' }, ...sources),
    ...stop,
    error,
    waiting,
    footer,
  )
  container.querySelector<HTMLInputElement>('input:checked')?.focus()
}
