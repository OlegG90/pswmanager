import { invoke } from '@tauri-apps/api/core'
import type { Entry, EntryDetail } from '../../../../src/api'

export type { Entry, EntryDetail }

export interface Database {
  title: string
  description: string
  /** Where it syncs with, for people. */
  syncedWith: string | null
  /** It syncs with a cloud store (and so may have a visible copy). */
  cloud: boolean
  /** The folder its visible copy is in, if one was chosen. */
  copyFolder: string | null
}

export interface Status {
  database: Database | null
  unlocked: boolean
}

export interface Listing {
  entries: Entry[]
  /** The database's own icons, as `data:` URLs. */
  customIcons: Record<string, string>
}

export interface Code {
  code: string
  /** Seconds left. */
  remaining: number
  period: number
}

/** What the last sync did (event `synced`). */
export interface Synced {
  text: string
  problem: boolean
  /** The entries changed: read the list again. */
  changed: boolean
  /** The store wants the user to sign in again. */
  signIn: boolean
  /** The visible copy could not be written, and why. */
  copyProblem: string | null
}

/** A folder or file the user picked. */
export interface Picked {
  uri: string
  name: string
}

/** A database in the app's Dropbox folder. */
export interface CloudFile {
  id: string
  name: string
}

/** How a sign-in ended (event `signed-in`). */
export interface SignedIn {
  error: string | null
}

export const api = {
  status: () => invoke<Status>('status'),
  openLocalFile: () => invoke<Status | null>('open_local_file'),
  forgetDatabase: () => invoke<Status>('forget_database'),
  unlock: (password: string) => invoke<Listing>('unlock', { password }),
  lock: () => invoke<void>('lock'),
  listing: () => invoke<Listing>('listing'),
  entry: (id: string) => invoke<EntryDetail>('entry', { id }),
  reveal: (id: string, field: string) => invoke<string>('reveal', { id, field }),
  copyField: (id: string, field: string) => invoke<number>('copy_field', { id, field }),
  totp: (id: string) => invoke<Code | null>('totp', { id }),
  copyTotp: (id: string) => invoke<number>('copy_totp', { id }),
  openUrl: (id: string) => invoke<void>('open_url', { id }),
  syncNow: () => invoke<void>('sync_now'),
  lastSync: () => invoke<Synced | null>('last_sync'),
  signInToDropbox: () => invoke<void>('sign_in_to_dropbox'),
  dropboxFiles: () => invoke<CloudFile[]>('dropbox_files'),
  openDropboxFile: (file: CloudFile, folder: Picked) => invoke<Status>('open_dropbox_file', { file, folder }),
  copyNameTaken: (folder: string, name: string) => invoke<boolean>('copy_name_taken', { folder, name }),
  pickFolder: () => invoke<Picked | null>('pick_folder'),
  setCopyFolder: (folder: Picked) => invoke<Status>('set_copy_folder', { folder }),
  syncIfPending: () => invoke<void>('sync_if_pending'),
}
