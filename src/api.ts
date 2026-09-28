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
  name: string
  syncKind: SyncKind | null
}

/** A remote file a database can link to. */
export type SyncTarget = { kind: 'folder'; path: string } | { kind: 'cloud'; cloud: Cloud; file: CloudFile }

/** What to do when a database is linked to a remote file that differs. */
export type LinkChoice = 'merge' | 'useRemote' | 'keepLocal'

/** A cloud store one signs in to. */
export type Cloud = 'dropbox' | 'google'

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
}

/** After attaching a file. */
export interface Attached {
  /** The name the file got: a name the entry already uses gets a number. */
  name: string
  listing: Listing
}

export interface Listing {
  entries: Entry[]
  customIcons: Record<string, string>
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
  createDatabase: (file: string, password: string, keyFile: string | null) =>
    invoke<Status>('create_database', { file, password, keyFile }),
  selectDatabase: (file: string) => invoke<Status>('select_database', { file }),
  /** Takes a database off the list; its file stays. */
  removeDatabase: (file: string) => invoke<Status>('remove_database', { file }),
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
  pickKeyFile: () => invoke<Status>('pick_key_file'),
  clearKeyFile: () => invoke<Status>('clear_key_file'),
  unlock: (password: string) => invoke<Listing>('unlock', { password }),
  lock: () => invoke<void>('lock'),
  listing: () => invoke<Listing>('listing'),
  entry: (id: string) => invoke<EntryDetail>('entry', { id }),
  reveal: (id: string, field: string) => invoke<string>('reveal', { id, field }),
  /** Resolves to the seconds until the clipboard is cleared. */
  copy: (id: string, field: string) => invoke<number>('copy_field', { id, field }),
  openUrl: (id: string) => invoke<void>('open_url', { id }),
  icon: (host: string) => invoke<string | null>('icon', { host }),
  /** Tells the backend the window is in use, which postpones the auto-lock. */
  touch: () => invoke<void>('touch'),
  hideWindow: () => invoke<void>('hide_window'),
  editEntry: (id: string) => invoke<EntryData>('edit_entry', { id }),
  /** Creates an entry when `id` is null. */
  /** Creates an entry when `id` is null. `base` is the entry as the editor
   *  opened it: only what changed against it is saved. */
  saveEntry: (id: string | null, base: EntryData | null, data: EntryData) =>
    invoke<Saved>('save_entry', { id, base, data }),
  deleteEntry: (id: string) => invoke<Listing>('delete_entry', { id }),
  /** Asks where to save the file and writes it there; false when cancelled. */
  saveAttachment: (id: string, name: string) => invoke<boolean>('save_attachment', { id, name }),
  /** Opens the file in the app Windows uses for its type, from a read-only
   *  copy deleted when the database locks. */
  openAttachment: (id: string, name: string) => invoke<void>('open_attachment', { id, name }),
  /** Asks for a file and attaches it to the entry; null when cancelled. */
  attachFile: (id: string) => invoke<Attached | null>('attach_file', { id }),
  /** Asks for a file and makes its content the entry's file `name`; its
   *  history keeps the old content. Null when cancelled. */
  replaceAttachment: (id: string, name: string) => invoke<Listing | null>('replace_attachment', { id, name }),
  /** Renames the entry's file; its history keeps the old name. */
  renameAttachment: (id: string, from: string, to: string) => invoke<Attached>('rename_attachment', { id, from, to }),
  /** Removes the file from the entry; its history keeps it. */
  removeAttachment: (id: string, name: string) => invoke<Listing>('remove_attachment', { id, name }),
  groupPaths: () => invoke<string[][]>('group_paths'),
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
