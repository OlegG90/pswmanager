import type { Entry } from './api'

/** The sidebar's fixed groups: an entry is in one because of what it is. */
export type Group = 'all' | 'favorites' | 'expired' | '2fa' | 'passkey' | 'templates' | 'trash'

export type Filter = { kind: 'group'; group: Group } | { kind: 'tag'; tag: string }

export const ALL: Filter = { kind: 'group', group: 'all' }

export const GROUPS: { group: Group; label: string }[] = [
  { group: 'all', label: 'All' },
  { group: 'favorites', label: 'Favorites' },
  { group: 'expired', label: 'Expired' },
  { group: '2fa', label: '2FA' },
  { group: 'passkey', label: 'Passkey' },
  { group: 'templates', label: 'Templates' },
  { group: 'trash', label: 'Trash' },
]

/** The star is this tag, as `sic2kdbx` writes SafeInCloud's; it is not listed among the tags. */
export const FAVORITE = 'Favorite'

/** Entries expiring within this many days are listed under Expired too, as soon. */
const SOON_DAYS = 14

/** Whether the entry has expired, expires soon, or neither. */
export function expiry(entry: Entry, now: number): 'expired' | 'soon' | null {
  if (!entry.expires) return null
  const at = Date.parse(entry.expires)
  if (at <= now) return 'expired'
  return at <= now + SOON_DAYS * 24 * 3600 * 1000 ? 'soon' : null
}

function inFilter(entry: Entry, filter: Filter, now: number): boolean {
  if (filter.kind === 'tag') return entry.kind === 'entry' && entry.tags.includes(filter.tag)
  switch (filter.group) {
    case 'templates':
      return entry.kind === 'template'
    case 'trash':
      return entry.kind === 'trash'
  }
  if (entry.kind !== 'entry') return false
  switch (filter.group) {
    case 'all':
      return true
    case 'favorites':
      return entry.tags.includes(FAVORITE)
    case 'expired':
      return expiry(entry, now) !== null
    case '2fa':
      return entry.otp
    case 'passkey':
      return entry.passkey
  }
}

/** Entries matching every word of the query (title, user name, URL, tags and
 *  notes; never passwords), within the group or tag. */
export function search(entries: Entry[], query: string, filter: Filter = ALL, now = Date.now()): Entry[] {
  const words = query.toLocaleLowerCase().split(/\s+/).filter(Boolean)
  return entries.filter((e) => {
    if (!inFilter(e, filter, now)) return false
    if (words.length === 0) return true
    const text = [e.title, e.username, e.url, ...e.tags, e.notes].join('\n').toLocaleLowerCase()
    return words.every((w) => text.includes(w))
  })
}

/** Every tag of the entries in use with how many have it, sorted; not the star. */
export function tagCounts(entries: Entry[]): [string, number][] {
  const counts = new Map<string, number>()
  for (const e of entries) {
    if (e.kind !== 'entry') continue
    for (const tag of e.tags) if (tag !== FAVORITE) counts.set(tag, (counts.get(tag) ?? 0) + 1)
  }
  return [...counts].sort(([a], [b]) => a.localeCompare(b))
}

export const sameFilter = (a: Filter, b: Filter) =>
  a.kind === 'group' ? b.kind === 'group' && a.group === b.group : b.kind === 'tag' && a.tag === b.tag
