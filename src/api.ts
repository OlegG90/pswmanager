import { invoke } from '@tauri-apps/api/core'

export interface Status {
  database: string | null
  /** Where the database is synced to; `database` is then its working copy. */
  syncedWith: string | null
  syncKind: 'folder' | Cloud | null
  keyFile: string | null
  unlocked: boolean
  /** Something to tell the user, such as a hotkey that could not be registered. */
  notice: string | null
}

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
}

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
  syncWithFolder: () => invoke<Status>('sync_with_folder'),
  stopSync: () => invoke<Status>('stop_sync'),
  signInToCloud: (cloud: Cloud) => invoke<CloudFiles>('sign_in_to_cloud', { cloud }),
  /** Syncs with `file` there; `null` uploads the local database first. */
  syncWithCloud: (cloud: Cloud, file: CloudFile | null) => invoke<Status>('sync_with_cloud', { cloud, file }),
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
