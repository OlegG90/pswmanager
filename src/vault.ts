import type { DatabaseSettings, Entry, EntryDetail, Listing, Version, VersionDetail } from './api'
import { ALL, FAVORITE, sameFilter, search, tagCounts, TEMPLATES, type Filter } from './search'
import { clicked, type Choice } from './selection'

/** The listing while no database is unlocked. */
export const EMPTY: Listing = {
  entries: [],
  customIcons: {},
  database: { name: '', description: '', defaultUsername: '', historyMaxItems: -1, historyMaxSize: -1,
    encryption: { cipher: 'aes256', kdf: 'argon2id', iterations: 0, memory: 0, parallelism: 0 } }, // all but the user name: unused here, Settings fetches them
  inImportGroups: 0,
}

/** What the right column shows of the current entry: itself, its history, or
 *  one of its older versions (`index` in the history, 0 the newest). */
export type View =
  | { kind: 'entry' }
  | { kind: 'history'; versions: Version[] }
  | { kind: 'version'; versions: Version[]; index: number; at: VersionDetail }

const ENTRY_VIEW: View = { kind: 'entry' }

/** What the list chooses: one entry (or none), or several. */
export type Pick = { one: string | null } | { several: Choice }

/** Chosen entries, by what can be done to them. */
export interface Chosen {
  all: Entry[]
  /** Starred, tagged, auto-typed, merged. */
  inUse: Entry[]
  /** Restored or deleted for good. */
  trashed: Entry[]
  /** Moved to the recycle bin, as entries in use are. */
  templates: Entry[]
}

/** An entry in use: starred, tagged, given files. Templates are edited and
 *  deleted too (see [isTemplate]); the trash's are only restored or deleted. */
export function editable<T extends Entry>(entry: T | null | undefined): entry is T & { kind: 'entry' } {
  return entry?.kind === 'entry'
}

export function isTemplate<T extends Entry>(entry: T | null | undefined): entry is T & { kind: 'template' } {
  return entry?.kind === 'template'
}

export function inTrash<T extends Entry>(entry: T | null | undefined): entry is T & { kind: 'trash' } {
  return entry?.kind === 'trash'
}

export function count(n: number): string {
  return `${n} ${n === 1 ? 'entry' : 'entries'}`
}

/** The tags of `entries`, without the star, in order. */
export function tagsOf(entries: Entry[]): string[] {
  return [...new Set(entries.flatMap((e) => e.tags.filter((t) => t !== FAVORITE)))].sort((a, b) => a.localeCompare(b))
}

/**
 * The records pane of an unlocked database: its listing, what the sidebar
 * filters it to, the entries the list shows, the one selected or the several
 * chosen, the entry shown with its view, and the values revealed in it.
 * It holds the rules on that state and nothing of the page: the window draws
 * what it says and fetches what it asks for.
 */
export function vaultState() {
  let listing = EMPTY
  /** What the sidebar shows: a group or a tag. */
  let filter: Filter = ALL
  /** The entries the list shows, in its order. */
  let shown: Entry[] = []
  let selectedId: string | null = null
  /** Several entries chosen with Ctrl / Shift+click; null while one (or none) is. */
  let several: Choice | null = null
  /** The entry shown, as last fetched; null while it is to be fetched (again). */
  let current: EntryDetail | null = null
  let view: View = ENTRY_VIEW
  /** Values revealed in the entry shown, by field name. */
  const revealed = new Map<string, string>()

  const isShown = (id: string | null) => shown.some((e) => e.id === id)
  const tags = () => tagCounts(listing.entries)

  /** A tag no entry in use has any more goes back to All. */
  const setListing = (next: Listing) => {
    listing = next
    const chosen = filter
    if (chosen.kind === 'tag' && !tags().some(([tag]) => tag === chosen.tag)) filter = ALL
  }

  /** The listing the backend sent after a change. The entry shown is fetched
   *  again and shown from the top: a version's revealed values go. */
  const apply = (next: Listing) => {
    setListing(next)
    if (view.kind !== 'entry') revealed.clear()
    view = ENTRY_VIEW
    current = null
  }

  /** Makes entry `id` show in the list: among All (a template among the
   *  templates) when the sidebar's choice or `query` leaves it out of
   *  `entries`. False when the search must be cleared too. */
  const find = (id: string, query: string, entries = listing.entries): boolean => {
    if (search(entries, query, filter).some((e) => e.id === id)) return true
    filter = entries.some((e) => e.id === id && isTemplate(e)) ? TEMPLATES : ALL
    return false
  }

  return {
    get listing() {
      return listing
    },
    get filter() {
      return filter
    },
    get shown() {
      return shown
    },
    get selectedId() {
      return selectedId
    },
    get several() {
      return several
    },
    get current() {
      return current
    },
    get view() {
      return view
    },

    entry: (id: string): Entry | undefined => listing.entries.find((e) => e.id === id),

    /** Every tag of the entries in use with how many have it, sorted; not the star. */
    tags,

    /** How many entries have the tag: templates and the trash's too, as renaming
     *  and removing a tag change every entry (a template would bring it back). */
    tagged: (tag: string): number => listing.entries.filter((e) => e.tags.includes(tag)).length,

    // ---------------------------------------------------------- the listing

    /** A database unlocked: its listing, all of it in the sidebar. */
    open(next: Listing) {
      filter = ALL
      setListing(next)
    },

    /** Locked: everything shown is forgotten. */
    close() {
      listing = EMPTY
      shown = []
      selectedId = null
      several = null
      current = null
      view = ENTRY_VIEW
      revealed.clear()
    },

    apply,

    /** Entry `id` was saved into `next`: it is selected, and shows in the list
     *  (see [find]). False when the search must be cleared too. The values
     *  revealed before go: they were another entry's, or are older. */
    saved(id: string, next: Listing, query: string): boolean {
      const kept = find(id, query, next.entries)
      selectedId = id
      revealed.clear()
      apply(next)
      return kept
    },

    /** The database's settings changed (the default user name a new entry
     *  gets); the entry shown is fetched again, as its history may be trimmed. */
    setDatabase(database: DatabaseSettings) {
      listing = { ...listing, database }
      current = null
    },

    // ---------------------------------------------------------- the list

    setFilter(next: Filter) {
      filter = next
    },

    /** The tag shown was renamed: the sidebar follows it. */
    renameTag(from: string, to: string) {
      if (sameFilter(filter, { kind: 'tag', tag: from })) filter = { kind: 'tag', tag: to }
    },

    /** Re-runs the search, and says what the list chooses now: what was chosen,
     *  as far as it is still shown, else the first entry. */
    search(query: string): Pick {
      shown = search(listing.entries, query, filter)
      if (several) {
        const still = several.chosen.filter(isShown)
        if (still.length > 1) return { several: { ...several, chosen: still } }
        several = null
        if (still.length) return { one: still[0] }
      }
      return { one: isShown(selectedId) ? selectedId : (shown[0]?.id ?? null) }
    },

    find,

    /** What a click on entry `id` chooses: it alone, or with Ctrl / Shift, several. */
    pick(id: string, keys: { ctrl: boolean; shift: boolean }): Pick {
      if (!keys.ctrl && !keys.shift) return { one: id }
      const now = several ?? (selectedId ? { chosen: [selectedId], anchor: selectedId } : null)
      const next = clicked(shown.map((e) => e.id), now, id, keys)
      return next.chosen.length > 1 ? { several: next } : { one: next.chosen[0] }
    },

    /** The entry `by` steps away in the list (from several chosen, from the
     *  last one clicked); null when the list is empty. */
    step(by: number): string | null {
      if (shown.length === 0) return null
      const i = shown.findIndex((e) => e.id === (several?.anchor ?? selectedId))
      return shown[Math.min(shown.length - 1, Math.max(0, i + by))].id
    },

    /** Selects entry `id` (or none). True when the entry shown changes: it is
     *  to be fetched, or with none the column empties. Another entry's
     *  revealed values go. */
    select(id: string | null): boolean {
      const same = id === selectedId
      if (!same) revealed.clear()
      selectedId = id
      several = null
      if (same && current?.id === id) return false
      if (!id) current = null
      return true
    },

    /** Entry `id` arrived from the backend; false when another is selected meanwhile. */
    show(id: string, entry: EntryDetail): boolean {
      if (selectedId !== id) return false
      current = entry
      view = ENTRY_VIEW
      return true
    },

    /** The entry shown is fetched again the next time it is selected. */
    refetch() {
      current = null
    },

    chooseSeveral(choice: Choice) {
      several = choice
      selectedId = null
      current = null
      revealed.clear()
    },

    /** The several chosen were changed away (deleted, restored): the list
     *  chooses its first entry again. */
    dropSeveral() {
      several = null
    },

    /** What the list's menu for entry `id` acts on: all the chosen entries when
     *  it is one of them, else it alone. */
    menuTargets: (id: string): string[] => (several?.chosen.includes(id) ? several.chosen : [id]),

    /** The entries `ids` names, by what can be done to them; by default the several chosen. */
    chosen(ids = several?.chosen ?? []): Chosen {
      const all = listing.entries.filter((e) => ids.includes(e.id))
      return { all, inUse: all.filter(editable), trashed: all.filter(inTrash), templates: all.filter(isTemplate) }
    },

    // ---------------------------------------------------------- the entry shown

    revealedValue: (field: string): string | undefined => revealed.get(field),

    /** Reveals `value` of `field` in entry `id`, unless another is shown meanwhile. */
    reveal(id: string, field: string, value: string): boolean {
      if (current?.id !== id) return false
      revealed.set(field, value)
      return true
    },

    /** Masks `field` again; false when it was not revealed. */
    hide: (field: string): boolean => revealed.delete(field),

    /** The older version shown, for fetching its values and files. */
    shownVersion: (): number | null => (view.kind === 'version' ? view.index : null),

    /** What the entry view shows now: the entry, or the older version open. */
    shownDetail: (): EntryDetail | null => (view.kind === 'version' ? view.at : current),

    showEntry() {
      view = ENTRY_VIEW
    },

    showHistory(versions: Version[]) {
      view = { kind: 'history', versions }
      revealed.clear()
    },

    showVersion(versions: Version[], index: number, at: VersionDetail) {
      view = { kind: 'version', versions, index, at }
      revealed.clear()
    },

    /** One step back: from a version to the history, from the history to the
     *  entry. False when the entry itself is shown. */
    back(): boolean {
      if (view.kind === 'entry') return false
      view = view.kind === 'version' ? { kind: 'history', versions: view.versions } : ENTRY_VIEW
      revealed.clear()
      return true
    },
  }
}

export type Vault = ReturnType<typeof vaultState>
