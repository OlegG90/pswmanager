import { invoke } from '@tauri-apps/api/core'
import type { Entry, EntryData, EntryDetail, FileChange, GeneratorOptions, StagedFile, Strength, Version, VersionDetail } from '../../../../src/api'

export type { Entry, EntryData, EntryDetail, Version, VersionDetail }

export interface Database {
  title: string
  description: string
  /** Where it syncs with, for people. */
  syncedWith: string | null
  /** The cloud store it syncs with, if one (and so it may have a visible copy). */
  cloud: Cloud | null
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

/** A cloud store the phone syncs with. */
export type Cloud = 'dropbox' | 'onedrive' | 'google'

/** A database in the app's folder in a cloud store. */
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
  /** Unlock with a fingerprint or face (offered on the unlock screen). */
  biometricUnlock: boolean
  /** Days between asking for the master password when biometric unlock is on. */
  passwordEveryDays: number
  version: string
}

export const api = {
  status: () => invoke<Status>('status'),
  openLocalFile: () => invoke<Status | null>('open_local_file'),
  forgetDatabase: () => invoke<Status>('forget_database'),
  /** Stops syncing with the cloud store (with `signOut`, *Disconnect*); the visible copy becomes the file. */
  stopSyncing: (signOut: boolean) => invoke<Status>('stop_syncing', { signOut }),
  /** Syncs this database with `file` in the store (merged), or uploads it there as a new file (null). */
  syncWithCloud: (cloud: Cloud, file: CloudFile | null) => invoke<Status>('sync_with_cloud', { cloud, file }),
  /** With biometric unlock on and no sealed key (or the password due), the key is then sealed for it. */
  unlock: (password: string) => invoke<Listing>('unlock', { password }),
  /** Rejects with `cancelled` when the user chose the master password. */
  unlockWithBiometric: () => invoke<Listing>('unlock_with_biometric'),
  /** The unlock screen shows the fingerprint button. */
  biometricReady: () => invoke<boolean>('biometric_ready'),
  forgetBiometric: () => invoke<void>('forget_biometric'),
  lock: () => invoke<void>('lock'),
  listing: () => invoke<Listing>('listing'),
  entry: (id: string) => invoke<EntryDetail>('entry', { id }),
  /** A field's value; with `version`, an older version's (so too for copyField and openAttachment). */
  reveal: (id: string, field: string, version: number | null = null) => invoke<string>('reveal', { id, field, version }),
  copyField: (id: string, field: string, version: number | null = null) => invoke<number>('copy_field', { id, field, version }),
  /** The entry's older versions, newest first. */
  entryHistory: (id: string) => invoke<Version[]>('entry_history', { id }),
  entryVersion: (id: string, index: number) => invoke<VersionDetail>('entry_version', { id, index }),
  totp: (id: string) => invoke<Code | null>('totp', { id }),
  copyTotp: (id: string) => invoke<number>('copy_totp', { id }),
  openUrl: (id: string) => invoke<void>('open_url', { id }),
  openAttachment: (id: string, name: string, version: number | null = null) => invoke<void>('open_attachment', { id, name, version }),
  lockLater: () => invoke<void>('lock_later'),
  screenOff: () => invoke<boolean>('screen_off'),
  settings: () => invoke<Settings>('settings'),
  setSetting: (name: string, value: unknown) => invoke<Settings>('set_setting', { name, value }),
  stayUnlocked: () => invoke<void>('stay_unlocked'),
  syncNow: () => invoke<void>('sync_now'),
  lastSync: () => invoke<Synced | null>('last_sync'),
  signIn: (cloud: Cloud) => invoke<void>('sign_in', { cloud }),
  /** Makes a new database and puts it in a store (with its visible copy in `folder`) or a folder. */
  createDatabase: (name: string, password: string, place: { kind: 'cloud'; cloud: Cloud; folder: Picked } | { kind: 'folder'; folder: Picked }) =>
    invoke<{ status: Status; copyProblem: string | null }>('create_database', { name, password, place }),
  cloudFiles: (cloud: Cloud) => invoke<CloudFile[]>('cloud_files', { cloud }),
  openCloudFile: (cloud: Cloud, file: CloudFile, folder: Picked) =>
    invoke<{ status: Status; copyProblem: string | null }>('open_cloud_file', { cloud, file, folder }),
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
