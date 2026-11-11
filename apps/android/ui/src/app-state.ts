import type { Entry, Settings, Status, Synced } from './api'
import { ALL, sameFilter, search, TEMPLATES, type Filter } from '../../../../src/search'

/** Where the list was when an entry was opened from it, for that group or tag
 *  and search: going back finds it there again (#228). */
interface ListPlace {
  top: number
  id: string
  filter: Filter
  query: string
}

/** How soon after coming back the unlock screen still counts as come back to (ms). */
const RETURN_PROMPT = 3000
/** How often use is reported to the backend, at most (ms), as on Windows. */
const REPORT_EVERY = 5000

/** Coming back to the app after this long syncs (ms). */
const AGAIN_AFTER = 60 * 1000

/**
 * What the phone's screens share: the database, the settings, the list's
 * group or tag and search, what the last sync said, and the moments the
 * locking and the syncs on their own go by. It holds the rules on that state
 * and nothing of the page: the screens draw what it says. `clock` gives the
 * time (ms), for the tests.
 */
export function appState(clock = () => Date.now()) {
  const state = {
    /** The database on this phone: what the screens say about it. */
    database: null as Status['database'],
    /** The settings in effect. */
    settings: null as Settings | null,
    /** The database is unlocked: syncs run on their own. */
    unlocked: false,
    /** The list's group or tag, and its search. */
    filter: ALL as Filter,
    query: '',
    /** What the last sync did, as the status line says it. */
    syncLine: '',
    /** The last sync asked to sign in to the store again. */
    signInAgain: false,
    /** Why the visible copy was not written at the last sync, if it was not. */
    copyProblem: null as string | null,
    /** The remote file is on a key another device changed it to. */
    otherKey: false,

    /** Whether the new-key screen opened for the key change the last sync found. */
    keyAsked: false,
    /** When the last sync ended, for syncing again on coming back. */
    lastSynced: 0,
    /** When the app last came back from another app. */
    returnedAt: 0,
    /** When the app went to the background, and when a touch was last reported. */
    hiddenAt: clock(),
    lastReported: 0,
    listPlace: null as ListPlace | null,

    /** Whether the database syncs with a cloud store: only then is there a sync
     *  to start by hand (#225, #226). A local file still syncs at the moments of
     *  *Synchronisation* in the spec. */
    linked: () => !!state.database?.cloud,

    /** Swipes are on unless the setting turned them off. */
    swipesOn: () => state.settings?.swipes !== false,

    /** What a sync did (event `synced`). */
    synced(synced: Synced) {
      state.syncLine = synced.text
      state.signInAgain = synced.signIn
      state.copyProblem = synced.copyProblem
      state.otherKey = synced.otherKey
      if (!state.otherKey) state.keyAsked = false
      state.lastSynced = clock()
    },

    /** Whether the new-key screen opens by itself now: once, when a sync found
     *  that another device changed the key (as Windows' dialog does); after
     *  *Not now*, the toolbar's key and the sync sheet still offer it. */
    askForKey(): boolean {
      if (!state.otherKey || state.keyAsked || !state.unlocked) return false
      state.keyAsked = true
      return true
    },

    /** The database locked: nothing of it stays on the page. `here` when the
     *  page locked it (not the backend, in the background): the next unlock's
     *  sync then says again whether another device changed the key. */
    locked(here: boolean) {
      state.unlocked = false
      state.query = ''
      state.listPlace = null
      if (!here) return
      state.otherKey = false
      state.keyAsked = false
    },

    /** An entry opened from the list scrolled to `top`. */
    leaveList(top: number, id: string) {
      state.listPlace = { top, id, filter: state.filter, query: state.query }
    },

    /** Where the list was when an entry was opened from it, once, and only
     *  while it shows the same group or tag and search. */
    takeListPlace(): ListPlace | null {
      const place = state.listPlace
      state.listPlace = null
      return place && sameFilter(place.filter, state.filter) && place.query === state.query ? place : null
    },

    /** Back on the list a saved entry shows: among All (a template among the
     *  templates), without the search, when the chosen group or the search
     *  leaves it out. */
    showSaved(id: string, entries: Entry[], template: boolean) {
      if (search(entries, state.query, state.filter).some((e) => e.id === id)) return
      state.filter = template ? TEMPLATES : ALL
      state.query = ''
    },

    /** The app went to the background. */
    wentAway() {
      if (state.unlocked) state.hiddenAt = clock()
    },

    /** The app came back to the front: true when it must lock now, as the
     *  backend's timer may not have run on time (Android asleep). */
    cameBack(): boolean {
      state.returnedAt = clock()
      const after = state.settings?.lockInBackground
      return state.unlocked && after != null && clock() - state.hiddenAt > after * 1000
    },

    /** The unlock screen shown now counts as come back to: its prompt opens by itself. */
    justReturned: () => clock() - state.returnedAt < RETURN_PROMPT,

    /** A touch, a key or a scroll: whether to report it to the backend, which
     *  times the idle lock (#378); at most every few seconds, while unlocked. */
    touched(): boolean {
      if (!state.unlocked || clock() - state.lastReported < REPORT_EVERY) return false
      state.lastReported = clock()
      return true
    },

    /** Whether a sync is due while in front: the last one is `age` (ms) old or older. */
    syncDue: (age: number) => state.unlocked && age > 0 && clock() - state.lastSynced >= age,

    /** The sync on coming back is due. */
    returnSyncDue: () => state.syncDue(AGAIN_AFTER),

    /** The sync the setting asks for while in front is due. */
    everySyncDue: () => state.syncDue((state.settings?.syncEveryMinutes ?? 5) * 60 * 1000),
  }
  return state
}
