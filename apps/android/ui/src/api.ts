import { invoke } from '@tauri-apps/api/core'
import type { Entry, EntryDetail } from '../../../../src/api'

export type { Entry, EntryDetail }

export interface Database {
  title: string
  description: string
  /** Where it syncs with, for people. */
  syncedWith: string | null
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
  syncNow: () => invoke<void>('sync_now'),
}
