import { invoke } from '@tauri-apps/api/core'

export interface Status {
  database: string | null
  /** Where the database is synced to; `database` is then its working copy. */
  syncedWith: string | null
  keyFile: string | null
  unlocked: boolean
  /** Something to tell the user, such as a hotkey that could not be registered. */
  notice: string | null
}

/** After signing in to Dropbox: what the app folder offers. */
export interface DropboxFiles {
  /** Databases in the app folder, as paths there. */
  files: string[]
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
  hasPassword: boolean
}

export interface Field {
  name: string
  /** Missing for a protected field: ask for it with reveal(). */
  value: string | null
  protected: boolean
}

export interface EntryDetail extends Entry {
  fields: Field[]
}

export interface Listing {
  entries: Entry[]
  customIcons: Record<string, string>
}

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
export interface Settings {
  lockAfterMinutes: number
  lockOnSessionLock: boolean
  lockWhenHidden: boolean
  /** Seconds. */
  clearClipboard: number
  syncEveryMinutes: number
  downloadIcons: boolean
  /** Shown only: set in the state file. */
  hotkey: string
  startWithWindows: boolean
}

export type SettingName = Exclude<keyof Settings, 'hotkey'>

export const PASSWORD = 'Password'
export const USERNAME = 'UserName'
export const URL_FIELD = 'URL'
export const OTP = 'otp'

export const api = {
  status: () => invoke<Status>('status'),
  pickDatabase: () => invoke<Status>('pick_database'),
  syncWithFolder: () => invoke<Status>('sync_with_folder'),
  stopSync: () => invoke<Status>('stop_sync'),
  signInToDropbox: () => invoke<DropboxFiles>('sign_in_to_dropbox'),
  /** Syncs with `path` in the app folder; `null` uploads the local database first. */
  syncWithDropbox: (path: string | null) => invoke<Status>('sync_with_dropbox', { path }),
  /** Gives up on Dropbox: stops waiting for the browser and signs out, unless already synced with it. */
  cancelDropbox: () => invoke<void>('cancel_dropbox'),
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
  groupPaths: () => invoke<string[][]>('group_paths'),
  totp: (id: string) => invoke<TotpCode | null>('totp', { id }),
  copyTotp: (id: string) => invoke<number>('copy_totp', { id }),
  generatePassword: (options: GeneratorOptions) => invoke<string>('generate_password', { options }),
  passwordStrength: (password: string) => invoke<Strength>('password_strength', { password }),
  settings: () => invoke<Settings>('settings'),
  /** Applies at once; resolves to every setting as it now is. */
  setSetting: (name: SettingName, value: number | boolean) => invoke<Settings>('set_setting', { name, value }),
}
