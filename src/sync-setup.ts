import { api, modulesOn, type Cloud, type SyncTarget } from './api'
import { button } from './dom'
import { choose } from './modal'

// Which stores the sync setup on this PC offers, and what each is for the
// commands: labels and hints come from the core (`Module`), so the frontend
// keeps no copy of them (#412). `document` is Android's, not offered here.
const STORE_CLOUDS: Record<string, Cloud | null> = {
  folder: null,
  dropbox: 'dropbox',
  google: 'google',
  onedrive: 'onedrive',
}
const CLOUD_NAMES: Record<Cloud, string> = { dropbox: 'Dropbox', google: 'Google Drive', onedrive: 'OneDrive' }

/**
 * Sets up sync for the open database: `upload` puts it into a store as a new
 * file, `link` pairs it with a file already there (asking what to do when
 * the two differ). `waiting` shows while the browser sign-in runs. Resolves
 * to true when sync was set up, false when the user gave up on the way.
 */
export async function setUpSync(mode: 'upload' | 'link', waiting: HTMLElement): Promise<boolean> {
  // The stores the sync setup offers, from the registry: a store turned off in
  // Settings is not offered (#412).
  const stores = (await modulesOn('stores'))
    .filter((m) => Object.hasOwn(STORE_CLOUDS, m.id))
    .map((m) => ({ label: m.label, cloud: STORE_CLOUDS[m.id] }))
  const where = await choose(mode === 'upload' ? 'Upload this database to…' : 'Link this database to a file in…', stores.map((s) => s.label))
  if (where === null) return false
  const cloud = stores[where].cloud
  if (!cloud) {
    if (mode === 'upload') return !!(await api.uploadToFolder()).syncedWith
    const path = await api.pickRemoteFile()
    return path !== null && linkWith({ kind: 'folder', path })
  }

  const name = CLOUD_NAMES[cloud]
  waiting.replaceChildren(`Finish signing in to ${name} in the browser… `,
    button('Cancel', 'Stop waiting for the browser', () => api.cancelCloud(cloud)))
  waiting.hidden = false
  try {
    const offer = await api.signInToCloud(cloud).finally(() => (waiting.hidden = true))
    if (mode === 'upload') {
      await api.syncWithCloud(cloud, null, null)
      return true
    }
    if (!offer.files.length) throw new Error(`There is no .kdbx file for PswManager in ${name} yet: upload this database instead`)
    const picked = await choose(`Which file in ${name}?`, offer.files.map((f) => f.name))
    if (picked === null) {
      await api.cancelCloud(cloud)
      return false
    }
    const linked = await linkWith({ kind: 'cloud', cloud, file: offer.files[picked] })
    if (!linked) await api.cancelCloud(cloud)
    return linked
  } catch (e) {
    await api.cancelCloud(cloud)
    throw e
  }
}

/** Links to `target`; when the two files differ, the user says what happens. */
async function linkWith(target: SyncTarget): Promise<boolean> {
  if (await api.linkDatabase(target, null)) return true
  const choice = await choose('This database and the remote file differ. What should happen?', [
    'Merge both (as KeePass does; needs the database unlocked)',
    'Use the remote file (this one is kept as .kdbx.bak)',
    'Keep this file (the remote one is kept as .remote.bak)',
  ])
  if (choice === null) return false
  return api.linkDatabase(target, (['merge', 'useRemote', 'keepLocal'] as const)[choice])
}
