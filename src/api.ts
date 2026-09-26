import { invoke } from '@tauri-apps/api/core'

export interface Status {
  database: string | null
  keyFile: string | null
  unlocked: boolean
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

export const PASSWORD = 'Password'
export const USERNAME = 'UserName'
export const URL_FIELD = 'URL'

export const api = {
  status: () => invoke<Status>('status'),
  pickDatabase: () => invoke<Status>('pick_database'),
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
}
