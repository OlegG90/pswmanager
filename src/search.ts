import type { Entry } from './api'

export type Filter = { kind: 'all' } | { kind: 'group'; path: string } | { kind: 'tag'; tag: string }

export const GROUP_SEPARATOR = ' / '

export function groupPath(entry: Entry): string {
  return entry.group.join(GROUP_SEPARATOR)
}

function inFilter(entry: Entry, filter: Filter): boolean {
  switch (filter.kind) {
    case 'all':
      return true
    case 'group': {
      const path = groupPath(entry)
      return path === filter.path || path.startsWith(filter.path + GROUP_SEPARATOR)
    }
    case 'tag':
      return entry.tags.includes(filter.tag)
  }
}

/** Entries matching every word of the query (title, user name, URL, tags and
 *  notes; never passwords), within the filter. */
export function search(entries: Entry[], query: string, filter: Filter = { kind: 'all' }): Entry[] {
  const words = query.toLocaleLowerCase().split(/\s+/).filter(Boolean)
  return entries.filter((e) => {
    if (!inFilter(e, filter)) return false
    if (words.length === 0) return true
    const text = [e.title, e.username, e.url, ...e.tags, e.notes].join('\n').toLocaleLowerCase()
    return words.every((w) => text.includes(w))
  })
}

/** Every group (with its parents) and tag in use, sorted, for the filter menu. */
export function filterChoices(entries: Entry[]): { groups: string[]; tags: string[] } {
  const groups = new Set<string>()
  const tags = new Set<string>()
  for (const e of entries) {
    e.group.forEach((_, i) => groups.add(e.group.slice(0, i + 1).join(GROUP_SEPARATOR)))
    e.tags.forEach((t) => tags.add(t))
  }
  const sorted = (s: Set<string>) => [...s].sort((a, b) => a.localeCompare(b))
  return { groups: sorted(groups), tags: sorted(tags) }
}
