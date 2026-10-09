import { invoke } from '@tauri-apps/api/core'
import type { DatabaseSetting, DatabaseSettings, Encryption, Entry, EntryData, EntryDetail, FileChange, GeneratorOptions, MergePreview, Similar, StagedFile, Strength, Version, VersionDetail } from '../../../../src/api'

export type { Entry, EntryData, EntryDetail, MergePreview, Similar, Version, VersionDetail }

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

/** Something an import could not bring over. */
export interface Skipped {
  /** The item's title in the other app. */
  title: string
  why: string
}

/** What an import from another app brought. */
export interface Imported {
  exporter: string
  /** The group the entries were put in. */
  group: string
  added: number
  /** What could not be brought over, and why. */
  skipped: Skipped[]
  listing: Listing
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
  /** The remote file opens with a key this phone does not know (another device changed it). */
  otherKey: boolean
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
  /** Sideways swipes on the list open the drawer and the settings. */
  swipes: boolean
  /** Days between asking for the master password when biometric unlock is on. */
  passwordEveryDays: number
  version: string
}

/** The key file a database is to have after a key change. */
export type NewKeyFile = { kind: 'keep' } | { kind: 'none' } | { kind: 'picked'; uri: string; name: string }

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
  /** The project's GitHub page, in the browser (Settings → About). */
  openRepository: () => invoke<void>('open_repository'),
  openAttachment: (id: string, name: string, version: number | null = null) => invoke<void>('open_attachment', { id, name, version }),
  /** False when the user cancelled Android's save picker. */
  saveAttachment: (id: string, name: string, version: number | null = null) => invoke<boolean>('save_attachment', { id, name, version }),
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
  /** Imports from another password manager on this phone; null when the user went back. */
  importFromApp: () => invoke<Imported | null>('import_from_app'),
  /** Offers the database for export to other password managers (#153), named `name` there. */
  registerExport: (name: string) => invoke<void>('register_export', { name }),
  cloudFiles: (cloud: Cloud) => invoke<CloudFile[]>('cloud_files', { cloud }),
  openCloudFile: (cloud: Cloud, file: CloudFile, folder: Picked) =>
    invoke<{ status: Status; copyProblem: string | null }>('open_cloud_file', { cloud, file, folder }),
  copyNameTaken: (folder: string, name: string) => invoke<boolean>('copy_name_taken', { folder, name }),
  pickFolder: () => invoke<Picked | null>('pick_folder'),
  setCopyFolder: (folder: Picked) => invoke<Status>('set_copy_folder', { folder }),
  syncIfPending: () => invoke<void>('sync_if_pending'),
  /** The database is unlocked with this key file from now on. */
  useKeyFile: (keyFile: Picked) => invoke<Status>('use_key_file', { keyFile }),
  /** A folder for a key file (Android's folder picker) and its files; null when cancelled (#207). */
  pickKeyFolder: () => invoke<{ folder: Picked; files: string[] } | null>('pick_key_folder'),
  keyFileIn: (folder: string, name: string) => invoke<Picked>('key_file_in', { folder, name }),
  /** All files access: on, the app browses the phone's files for a key file. */
  allFilesAccess: () => invoke<boolean>('all_files_access'),
  /** Android's page that turns All files access on. */
  askAllFilesAccess: () => invoke<void>('ask_all_files_access'),
  /** A folder of the phone's storage (null: its top), with All files access on. */
  browse: (path: string | null) => invoke<{ path: string; up: string | null; folders: string[]; files: string[] }>('browse', { path }),
  keyFileAt: (folder: string, name: string) => invoke<Picked>('key_file_at', { folder, name }),
  /** A browsed file, staged to attach. */
  attachFileAt: (folder: string, name: string) => invoke<StagedFile>('attach_file_at', { folder, name }),
  /** Saves an attachment as `fileName` in a browsed folder, never over a file there. */
  saveAttachmentAt: (id: string, name: string, version: number | null, folder: string, fileName: string) =>
    invoke<void>('save_attachment_at', { id, name, version, folder, fileName }),
  /** A browsed folder, kept as a picked one is. */
  folderAt: (path: string) => invoke<Picked>('folder_at', { path }),
  /** The database `name` in a browsed folder, synced with it there by its path. */
  openDatabaseAt: (folder: string, name: string) => invoke<Status>('open_database_at', { folder, name }),
  createKeyFileAt: (folder: string, name: string) => invoke<Picked>('create_key_file_at', { folder, name }),
  /** Makes a new key file in a picked folder, never over a file already there. */
  createKeyFileIn: (folder: string, name: string) => invoke<Picked>('create_key_file_in', { folder, name }),
  clearKeyFile: () => invoke<Status>('clear_key_file'),
  icon: (host: string) => invoke<string | null>('icon', { host }),
  editEntry: (id: string) => invoke<EntryData>('edit_entry', { id }),
  saveEntry: (id: string | null, base: EntryData | null, data: EntryData, files: FileChange[]) => invoke<Saved>('save_entry', { id, base, data, files }),
  pickFileToAttach: () => invoke<StagedFile | null>('pick_file_to_attach'),
  releaseFiles: (files: number[]) => invoke<void>('release_files', { files }),
  /** The settings kept in the open database's file (#193). */
  databaseSettings: () => invoke<DatabaseSettings>('database_settings'),
  setDatabaseSetting: (setting: DatabaseSetting, value: string) => invoke<DatabaseSettings>('set_database_setting', { setting, value }),
  /** How many old versions these history limits would remove. */
  historyLimitsPreview: (maxItems: number, maxSize: number) => invoke<number>('history_limits_preview', { maxItems, maxSize }),
  setHistoryLimits: (maxItems: number, maxSize: number) => invoke<DatabaseSettings>('set_history_limits', { maxItems, maxSize }),
  /** Milliseconds an unlock takes on this phone with this encryption. */
  encryptionUnlockTime: (encryption: Encryption) => invoke<number>('encryption_unlock_time', { encryption }),
  setEncryption: (encryption: Encryption) => invoke<DatabaseSettings>('set_encryption', { encryption }),
  /** After `current` proves right: an empty `password` means none. */
  changeMasterKey: (current: string, password: string, keyFile: NewKeyFile) => invoke<Status>('change_master_key', { current, password, keyFile }),
  /** The key another device changed the database to; the remote file is synced with it. */
  enterOtherKey: (password: string, keyFile: NewKeyFile) => invoke<Status>('enter_other_key', { password, keyFile }),
  /** Moves entries to the recycle bin. */
  deleteEntries: (ids: string[]) => invoke<Listing>('delete_entries', { ids }),
  /** The entries that are for the same site. */
  similarEntries: () => invoke<Similar[]>('similar_entries'),
  /** What `mergeEntries` would change in `keep`; nothing changes. */
  mergePreview: (keep: string, others: string[]) => invoke<MergePreview>('merge_preview', { keep, others }),
  /** Adds what `others` have to `keep` and moves them to the recycle bin. */
  mergeEntries: (keep: string, others: string[]) => invoke<Listing>('merge_entries', { keep, others }),
  /** Puts an entry from the recycle bin back where it was. */
  restoreEntry: (id: string) => invoke<Listing>('restore_entry', { id }),
  /** Removes an entry in the recycle bin for good. */
  deleteForGood: (id: string) => invoke<Listing>('delete_for_good', { id }),
  /** Removes everything in the recycle bin for good. */
  emptyTrash: () => invoke<Listing>('empty_trash'),
  /** Gives entries a tag or takes it off (the star is the tag Favorite). */
  setTag: (ids: string[], tag: string, on: boolean) => invoke<Listing>('set_tag', { ids, tag, on }),
  generatePassword: (options: GeneratorOptions) => invoke<string>('generate_password', { options }),
  passwordStrength: (password: string) => invoke<Strength>('password_strength', { password }),
}
