import { api, type Cloud, type Status } from './api'
import { button, el } from './dom'
import { choose } from './modal'
import { createDatabase } from './new-database'

type Source = 'new' | 'local' | 'folder' | Cloud

const SOURCES: [Source, string, string][] = [
  ['new', 'Create a new database', 'An empty KeePass file where you choose, with a master password and / or a key file.'],
  ['local', 'Open a local file', 'A .kdbx on this PC, a USB drive, or a folder another program syncs. PswManager does not sync it.'],
  ['folder', 'Open from a folder', 'A file on a LAN share such as a NAS: copied to a file on this PC that PswManager keeps in step with it.'],
  ['dropbox', 'Open from Dropbox', 'Sign in; pick a file in Apps/PswManager Sync. It is copied to a file on this PC that stays in step with it.'],
  ['google', 'Open from Google Drive', 'Sign in; pick a file in the PswManager folder of your Drive. PswManager sees only the files it put there.'],
]

const CLOUDS: Record<Cloud, { name: string; where: string }> = {
  dropbox: { name: 'Dropbox', where: 'the Dropbox app folder' },
  google: { name: 'Google Drive', where: 'the PswManager folder of your Google Drive' },
}

export interface ChooseOptions {
  /** A database was chosen: the new status. */
  chosen: (status: Status) => void
  /** Back to the unlock screen; only offered when a database is already chosen. */
  back: () => void
}

/** Signs in to a cloud store in the browser, then asks which file to sync
 *  with. Giving up on the way signs out again; null when the user did. */
async function syncWithCloud(cloud: Cloud, waiting: HTMLElement): Promise<Status | null> {
  const { name, where } = CLOUDS[cloud]
  waiting.replaceChildren(`Finish signing in to ${name} in the browser… `,
    button('Cancel', 'Stop waiting for the browser', () => api.cancelCloud(cloud)))
  waiting.hidden = false
  try {
    const offer = await api.signInToCloud(cloud).finally(() => (waiting.hidden = true))
    if (!offer.files.length) {
      throw new Error(`There is no .kdbx file in ${where}: to put a database there, open it and use Settings → Sync → Upload`)
    }
    const picked = await choose(`Which database in ${where} should this PC open?`, offer.files.map((file) => file.name))
    if (picked === null) {
      await api.cancelCloud(cloud)
      return null
    }
    const file = offer.files[picked]
    // Opened from the store: the user chooses where its file goes on this PC.
    const local = await api.pickNewFile(file.name)
    if (!local) {
      await api.cancelCloud(cloud)
      return null
    }
    return await api.syncWithCloud(cloud, file, local)
  } catch (e) {
    await api.cancelCloud(cloud)
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
  const waiting = el('p', { className: 'muted', hidden: true })
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
      const chosen = source === 'new' ? await createDatabase(container)
        : source in CLOUDS ? await syncWithCloud(source as Cloud, waiting)
        : changedOrNull(await (source === 'local' ? api.pickDatabase() : api.syncWithFolder()))
      // Back from the new-database form: this screen again.
      if (source === 'new' && !chosen) return renderChoose(container, status, options)
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

  const act = (action: () => Promise<Status>) => async () => {
    try {
      const next = await action()
      if (next.database) options.chosen(next)
      else renderChoose(container, next, options) // the list is empty now
    } catch (e) {
      fail(e)
    }
  }
  const current = status.database
  const remove = current
    ? [el('p', { className: 'stop muted' }, `${status.databases.find((d) => d.file === current)?.name ?? current} is in the list. `,
        button('Remove from the list', 'Forget this database here; its file stays where it is', act(() => api.removeDatabase(current))))]
    : []

  container.replaceChildren(
    el('div', { className: 'intro' },
      el('span', { className: 'kicker' }, first ? 'First run' : 'Database'),
      el('h1', {}, first ? 'Choose your database' : 'Add a database'),
      el('p', { className: 'muted' },
        'Each database is a KeePass (KDBX 4) file on this PC; the unlock screen switches between the ones added here.')),
    el('div', { className: 'sources', role: 'radiogroup', ariaLabel: 'Where the database lives' }, ...sources),
    ...remove,
    error,
    waiting,
    footer,
  )
  container.querySelector<HTMLInputElement>('input:checked')?.focus()
}
