import { invoke } from '@tauri-apps/api/core'
import type { Entry, EntryData, EntryDetail, FileChange, GeneratorOptions, StagedFile, Strength } from '../../../../src/api'

export type { Entry, EntryData, EntryDetail }

export interface Database {
  title: string
  description: string
  /** Where it syncs with, for people. */
  syncedWith: string | null
  /** It syncs with a cloud store (and so may have a visible copy). */
  cloud: boolean
  /** The folder its visible copy is in, if one was chosen. */
  copyFolder: string | null
  /** The key file it is unlocked with, by name, if it has one. */
  keyFile: string | null
}

export interface Status {
  database: Database | null
  unlocked: boolean
}

export interface Listing {
  entries: Entry[]
  /** The database's own icons, as `data:` URLs. */
  customIcons: Record<string, string>
  database: { defaultUsername: string }
}

export interface Saved {
  id: string
  listing: Listing
  /** Fields another device also changed; this edit replaced them. */
  conflicts: string[]
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

/** The settings screen's values (names as in crates/core/src/settings.rs). */
export interface Settings {
  /** Seconds in the background before locking; null for never. */
  lockInBackground: number | null
  lockOnScreenOff: boolean
  /** Minutes in front without a touch; 0 for never. */
  lockAfterMinutes: number
  clearClipboard: number
  /** Minutes between checks of the remote file while in front; 0 for never. */
  syncEveryMinutes: number
  downloadIcons: boolean
  theme: 'system' | 'light' | 'dark'
  version: string
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
  openAttachment: (id: string, name: string) => invoke<void>('open_attachment', { id, name }),
  lockLater: () => invoke<void>('lock_later'),
  screenOff: () => invoke<boolean>('screen_off'),
  settings: () => invoke<Settings>('settings'),
  setSetting: (name: string, value: unknown) => invoke<Settings>('set_setting', { name, value }),
  stayUnlocked: () => invoke<void>('stay_unlocked'),
  syncNow: () => invoke<void>('sync_now'),
  lastSync: () => invoke<Synced | null>('last_sync'),
  signInToDropbox: () => invoke<void>('sign_in_to_dropbox'),
  dropboxFiles: () => invoke<CloudFile[]>('dropbox_files'),
  openDropboxFile: (file: CloudFile, folder: Picked) => invoke<{ status: Status; copyProblem: string | null }>('open_dropbox_file', { file, folder }),
  copyNameTaken: (folder: string, name: string) => invoke<boolean>('copy_name_taken', { folder, name }),
  pickFolder: () => invoke<Picked | null>('pick_folder'),
  setCopyFolder: (folder: Picked) => invoke<Status>('set_copy_folder', { folder }),
  syncIfPending: () => invoke<void>('sync_if_pending'),
  pickKeyFile: () => invoke<Status>('pick_key_file'),
  clearKeyFile: () => invoke<Status>('clear_key_file'),
  icon: (host: string) => invoke<string | null>('icon', { host }),
  editEntry: (id: string) => invoke<EntryData>('edit_entry', { id }),
  saveEntry: (id: string | null, base: EntryData | null, data: EntryData, files: FileChange[]) => invoke<Saved>('save_entry', { id, base, data, files }),
  pickFileToAttach: () => invoke<StagedFile | null>('pick_file_to_attach'),
  releaseFiles: (files: number[]) => invoke<void>('release_files', { files }),
  deleteEntry: (id: string) => invoke<Listing>('delete_entry', { id }),
  setFavorite: (id: string, on: boolean) => invoke<Listing>('set_favorite', { id, on }),
  generatePassword: (options: GeneratorOptions) => invoke<string>('generate_password', { options }),
  passwordStrength: (password: string) => invoke<Strength>('password_strength', { password }),
}
