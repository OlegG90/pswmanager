import { invoke } from '@tauri-apps/api/core'

export interface Status {
  /** The file of the database the unlock screen opens. */
  database: string | null
  /** Where it is synced to, if anywhere. */
  syncedWith: string | null
  syncKind: SyncKind | null
  keyFile: string | null
  /** Every database in the list, the current one among them. */
  databases: DatabaseInfo[]
  unlocked: boolean
  /** Something to tell the user, such as a hotkey that could not be registered. */
  notice: string | null
}

export type SyncKind = 'folder' | Cloud

/** A database in the list. */
export interface DatabaseInfo {
  file: string
  /** Its name as last unlocked, or the file name. */
  name: string
  fileName: string
  /** As last unlocked; empty when it has none. */
  description: string
  syncKind: SyncKind | null
}

/** Settings kept in the database file itself; empty when it has none. */
export interface DatabaseSettings {
  name: string
  description: string
  defaultUsername: string
  /** Old versions kept per entry; -1 for no limit. */
  historyMaxItems: number
  /** Bytes of old versions kept per entry; -1 for no limit. */
  historyMaxSize: number
  encryption: Encryption
}

/** `other`: one the app keeps as it is but does not offer (Twofish, say). */
export type Cipher = 'aes256' | 'chaCha20' | 'other'
export type Kdf = 'argon2id' | 'argon2d' | 'aes' | 'other'

/** The file's cipher and key derivation. */
export interface Encryption {
  cipher: Cipher
  kdf: Kdf
  /** Argon2's iterations, or AES-KDF's rounds. */
  iterations: number
  /** Argon2 only: bytes. */
  memory: number
  /** Argon2 only: threads. */
  parallelism: number
}

/** The settings of the database file set one at a time, as text. */
export type DatabaseSetting = 'name' | 'description' | 'defaultUsername'

/** A remote file a database can link to. */
export type SyncTarget = { kind: 'folder'; path: string } | { kind: 'cloud'; cloud: Cloud; file: CloudFile }

/** What to do when a database is linked to a remote file that differs. */
export type LinkChoice = 'merge' | 'useRemote' | 'keepLocal'

/** A cloud store one signs in to. */
export type Cloud = 'dropbox' | 'google' | 'onedrive'

/** A database file in a cloud store: its id there (a path for Dropbox) and name. */
export interface CloudFile {
  id: string
  name: string
}

/** After signing in to a cloud store: what it offers. */
export interface CloudFiles {
  files: CloudFile[]
  /** The local database's name, when it can be uploaded instead. */
  upload: string | null
}

/** What the window shows about syncing with a remote store. */
export interface SyncStatus {
  /** False for a local file: nothing to show. */
  remote: boolean
  busy: boolean
  text: string
  /** The last sync did not finish: offline or an error. */
  problem: boolean
}

/** The current database's backup; times are seconds since 1970. */
export interface BackupInfo {
  folder: string | null
  /** 0: never. */
  everyDays: number
  last: number | null
  next: number | null
  /** Why the last try failed, if it did. */
  error: string | null
  /** The intervals offered, in days; 0 is never. */
  intervals: number[]
}

/** Which copies of the unlocked database open with a key this device does not know yet. */
export interface KeyNeeded {
  /** The file on this PC, replaced by another program. */
  local: boolean
  /** The remote file, at the last sync. */
  remote: boolean
}

export interface Entry {
  id: string
  title: string
  username: string
  url: string
  host: string | null
  group: string[]
  tags: string[]
  notes: string
  customIcon: string | null
  /** The KeePass standard icon chosen for it (not the default key). */
  icon: number | null
  hasPassword: boolean
  kind: EntryKind
  /** It has a TOTP secret. */
  otp: boolean
  /** It has a passkey KeePassXC stored. */
  passkey: boolean
  /** When it expires (RFC 3339), if it does. */
  expires: string | null
}

/** In use, a template, or in the recycle bin. */
export type EntryKind = 'entry' | 'template' | 'trash'

export interface Field {
  name: string
  /** Missing for a protected field: ask for it with reveal(). */
  value: string | null
  protected: boolean
}

/** A file attached to an entry; its content stays in the backend. */
export interface Attachment {
  name: string
  /** In bytes. */
  size: number
}

export interface EntryDetail extends Entry {
  fields: Field[]
  attachments: Attachment[]
  /** When the entry was last changed (RFC 3339), if the file says. */
  modified: string | null
  /** How many older versions its history keeps. */
  versions: number
}

/** An older version of an entry, as its history lists it. */
export interface Version {
  /** When it was saved (RFC 3339). */
  modified: string | null
  /** What changed from it to the next newer version: names, never values. */
  changed: string[]
}

/** An older version shown read only, with how it differs from the entry now. */
export interface VersionDetail extends EntryDetail {
  differs: Difference[]
}

/** A field whose value in a version is not the entry's current one. */
export interface Difference {
  name: string
  /** The current value, when it is not a secret and the entry has the field. */
  current: string | null
  /** The current value is a secret: only that it differs is said. */
  protected: boolean
}

/** A file picked in the editor, held by the backend until the entry is saved. */
export interface StagedFile {
  /** What a FileChange names it by. */
  content: number
  name: string
  size: number
}

/** A change the editor makes to the entry's files, saved with the entry. */
export type FileChange =
  /** A name the entry already uses gets a number (`scan (2).pdf`). */
  | { kind: 'add'; name: string; content: number }
  /** Renamed to `to`, given new content (a staged file), or both. */
  | { kind: 'change'; name: string; to: string; content: number | null }
  | { kind: 'remove'; name: string }

export interface Listing {
  entries: Entry[]
  customIcons: Record<string, string>
  database: DatabaseSettings
}

/** An entry's icon as the editor chooses it. */
export type IconChoice =
  | { kind: 'auto' }
  | { kind: 'builtin'; id: number }
  /** An image kept in the database, base64. */
  | { kind: 'custom'; data: string }

/** An entry with every value, as the editor shows and sends it. */
export interface EntryData {
  title: string
  username: string
  password: string
  url: string
  notes: string
  /** TOTP secret or otpauth:// URI; empty for none. */
  otp: string
  tags: string[]
  group: string[]
  fields: FieldData[]
  icon: IconChoice
  /** When it expires (RFC 3339); null when it does not. */
  expires: string | null
}

/** An additional field with its value, for the editor. */
export interface FieldData {
  name: string
  value: string
  protected: boolean
}

/** Sent when the database file changed on disk and was read again. */
export interface DiskChange {
  listing: Listing
  /** Entries that differ from what was shown. */
  changed: string[]
}

export interface Saved {
  id: string
  listing: Listing
  /** Fields another device also changed; this edit replaced them. */
  conflicts: string[]
}

export interface TotpCode {
  code: string
  /** Seconds the code stays valid. */
  remaining: number
  period: number
}

export interface GeneratorOptions {
  length: number
  upper: boolean
  lower: boolean
  digits: boolean
  symbols: boolean
  excludeLookAlikes: boolean
}

export interface Strength {
  /** 0 (guessed at once) to 4 (very hard). */
  score: number
  crackTime: string
}

/** The settings as they are in effect. Minutes of 0 mean never. */
/** Light or dark, or as Windows is set. */
export type Theme = 'system' | 'light' | 'dark'

export interface Settings {
  lockAfterMinutes: number
  lockOnSessionLock: boolean
  lockWhenHidden: boolean
  /** Seconds. */
  clearClipboard: number
  syncEveryMinutes: number
  downloadIcons: boolean
  theme: Theme
  /** The show / hide hotkey, e.g. `Ctrl+Alt+P`. */
  hotkey: string
  startWithWindows: boolean
}

/** An entry the password health check lists, and why. */
export interface Finding {
  id: string
  title: string
  detail: string
}

/** Each entry is in one list at most: reused, else weak, else old. */
export interface Health {
  reused: Finding[]
  weak: Finding[]
  old: Finding[]
}

export type SettingName = keyof Settings

export const PASSWORD = 'Password'
export const USERNAME = 'UserName'
export const URL_FIELD = 'URL'
export const OTP = 'otp'

export const api = {
  status: () => invoke<Status>('status'),
  pickDatabase: () => invoke<Status>('pick_database'),
  /** Where a new file goes on this PC (a save dialog); null when cancelled. */
  pickNewFile: (name: string, fresh = false) => invoke<string | null>('pick_new_file', { name, fresh }),
  pickKeyFilePath: () => invoke<string | null>('pick_key_file_path'),
  /** Milliseconds an unlock takes on this PC with `encryption` (as slow as one). */
  encryptionUnlockTime: (encryption: Encryption) => invoke<number>('encryption_unlock_time', { encryption }),
  /** Saved and synced like an edit. */
  setEncryption: (encryption: Encryption) => invoke<DatabaseSettings>('set_encryption', { encryption }),
  /** The key of a copy another device changed the key of; it is read with it. */
  enterOtherKey: (password: string, keyFile: string | null) => invoke<Status>('enter_other_key', { password, keyFile }),
  /** Makes a new key file where the user chooses; its path, null when cancelled. */
  createKeyFile: () => invoke<string | null>('create_key_file'),
  /** After `current` proves right: an empty `password` means none, `keyFile` is the one from now on. */
  changeMasterKey: (current: string, password: string, keyFile: string | null) =>
    invoke<Status>('change_master_key', { current, password, keyFile }),
  createDatabase: (file: string, password: string, keyFile: string | null) =>
    invoke<Status>('create_database', { file, password, keyFile }),
  selectDatabase: (file: string) => invoke<Status>('select_database', { file }),
  /** Takes a database off the list; its file stays. */
  removeDatabase: (file: string) => invoke<Status>('remove_database', { file }),
  /** Renames the current database's file (and its backups) in its folder. */
  renameDatabaseFile: (name: string) => invoke<Status>('rename_database_file', { name }),
  syncWithFolder: () => invoke<Status>('sync_with_folder'),
  stopSync: () => invoke<Status>('stop_sync'),
  /** Links the open database to an existing remote file; false (nothing changed) when they differ and no choice was given. */
  linkDatabase: (target: SyncTarget, choice: LinkChoice | null) => invoke<boolean>('link_database', { target, choice }),
  uploadToFolder: () => invoke<Status>('upload_to_folder'),
  /** A database file in a folder to link to; null when cancelled. */
  pickRemoteFile: () => invoke<string | null>('pick_remote_file'),
  signInToCloud: (cloud: Cloud) => invoke<CloudFiles>('sign_in_to_cloud', { cloud }),
  /** Opens `file` there into `local` on this PC; with no file, uploads the current local database and syncs it. */
  syncWithCloud: (cloud: Cloud, file: CloudFile | null, local: string | null) =>
    invoke<Status>('sync_with_cloud', { cloud, file, local }),
  /** Gives up on a cloud store: stops waiting for the browser and signs out, unless already synced with it. */
  cancelCloud: (cloud: Cloud) => invoke<void>('cancel_cloud', { cloud }),
  syncNow: () => invoke<void>('sync_now'),
  syncStatus: () => invoke<SyncStatus>('sync_status'),
  keyNeeded: () => invoke<KeyNeeded>('key_needed'),
  backup: () => invoke<BackupInfo | null>('backup'),
  setBackupEvery: (days: number) => invoke<BackupInfo | null>('set_backup_every', { days }),
  pickBackupFolder: () => invoke<BackupInfo | null>('pick_backup_folder'),
  backupNow: () => invoke<BackupInfo | null>('backup_now'),
  showBackupFolder: () => invoke<void>('show_backup_folder'),
  pickKeyFile: () => invoke<Status>('pick_key_file'),
  clearKeyFile: () => invoke<Status>('clear_key_file'),
  unlock: (password: string) => invoke<Listing>('unlock', { password }),
  lock: () => invoke<void>('lock'),
  listing: () => invoke<Listing>('listing'),
  databaseSettings: () => invoke<DatabaseSettings>('database_settings'),
  /** Saved in the database file and synced like an edit. */
  setDatabaseSetting: (setting: DatabaseSetting, value: string) => invoke<DatabaseSettings>('set_database_setting', { setting, value }),
  /** How many old versions these history limits would remove. */
  historyLimitsPreview: (maxItems: number, maxSize: number) => invoke<number>('history_limits_preview', { maxItems, maxSize }),
  /** Every entry's history is trimmed to them; saved and synced like an edit. */
  setHistoryLimits: (maxItems: number, maxSize: number) => invoke<DatabaseSettings>('set_history_limits', { maxItems, maxSize }),
  entry: (id: string) => invoke<EntryDetail>('entry', { id }),
  /** With `version`, the value in that older version (0 is the newest). */
  reveal: (id: string, field: string, version: number | null = null) => invoke<string>('reveal', { id, field, version }),
  /** Resolves to the seconds until the clipboard is cleared. */
  copy: (id: string, field: string, version: number | null = null) => invoke<number>('copy_field', { id, field, version }),
  /** The entry's older versions, newest first. */
  entryHistory: (id: string) => invoke<Version[]>('entry_history', { id }),
  entryVersion: (id: string, index: number) => invoke<VersionDetail>('entry_version', { id, index }),
  /** Makes an older version the entry's current content; the replaced one goes to history. */
  restoreVersion: (id: string, index: number, saved: string | null) => invoke<Listing>('restore_version', { id, index, saved }),
  openUrl: (id: string) => invoke<void>('open_url', { id }),
  icon: (host: string) => invoke<string | null>('icon', { host }),
  /** Tells the backend the window is in use, which postpones the auto-lock. */
  touch: () => invoke<void>('touch'),
  hideWindow: () => invoke<void>('hide_window'),
  editEntry: (id: string) => invoke<EntryData>('edit_entry', { id }),
  /** Creates an entry when `id` is null. */
  /** Creates an entry when `id` is null. `base` is the entry as the editor
   *  opened it: only what changed against it is saved. */
  saveEntry: (id: string | null, base: EntryData | null, data: EntryData, template = false, files: FileChange[] = []) =>
    invoke<Saved>('save_entry', { id, base, data, template, files }),
  /** Moves the entries to the recycle bin, as one change. */
  deleteEntries: (ids: string[]) => invoke<Listing>('delete_entries', { ids }),
  /** Puts entries from the trash back where they were. */
  restoreEntries: (ids: string[]) => invoke<Listing>('restore_entries', { ids }),
  /** Removes entries in the trash for good. */
  deleteForGood: (ids: string[]) => invoke<Listing>('delete_for_good', { ids }),
  /** Removes everything in the trash for good. */
  emptyTrash: () => invoke<Listing>('empty_trash'),
  /** Gives the entries the tag or takes it off; the star is the tag Favorite. */
  setTag: (ids: string[], tag: string, on: boolean) => invoke<Listing>('set_tag', { ids, tag, on }),
  /** Renames a tag in every entry; to a name another tag has, the two become one. */
  renameTag: (from: string, to: string) => invoke<Listing>('rename_tag', { from, to }),
  /** Takes a tag off every entry. */
  removeTag: (tag: string) => invoke<Listing>('remove_tag', { tag }),
  /** Asks where to save the file and writes it there; false when cancelled. */
  saveAttachment: (id: string, name: string, version: number | null = null) => invoke<boolean>('save_attachment', { id, name, version }),
  /** Opens the file in the app Windows uses for its type, from a read-only
   *  copy deleted when the database locks. */
  openAttachment: (id: string, name: string, version: number | null = null) => invoke<void>('open_attachment', { id, name, version }),
  /** Asks for a file to attach in the editor; null when cancelled. */
  pickFileToAttach: () => invoke<StagedFile | null>('pick_file_to_attach'),
  /** The editor let go of files it had picked. */
  releaseFiles: (files: number[]) => invoke<void>('release_files', { files }),
  /** An image file for an entry's icon, base64; null when cancelled. */
  pickIconImage: () => invoke<string | null>('pick_icon_image'),
  totp: (id: string) => invoke<TotpCode | null>('totp', { id }),
  copyTotp: (id: string) => invoke<number>('copy_totp', { id }),
  generatePassword: (options: GeneratorOptions) => invoke<string>('generate_password', { options }),
  passwordStrength: (password: string) => invoke<Strength>('password_strength', { password }),
  passwordHealth: () => invoke<Health>('password_health'),
  settings: () => invoke<Settings>('settings'),
  /** Applies at once; resolves to every setting as it now is. */
  setSetting: (name: SettingName, value: number | boolean | string) => invoke<Settings>('set_setting', { name, value }),
}
