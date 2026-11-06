import { describe, expect, it } from 'vitest'
import type { Listing, Saved } from './api'
import { CHANGED_ELSEWHERE, editorSession, type EditorRules } from './editor-session'

/** A promise the test settles when it wants to. */
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((yes, no) => ((resolve = yes), (reject = no)))
  return { promise, resolve, reject }
}

const SAVED: Saved = { id: 'Mail', listing: {} as Listing, conflicts: [] }

/** Questions answered by `answers`, in turn; what was asked is kept. */
function questions(...answers: boolean[]) {
  const asked: string[] = []
  let asking = false
  return {
    asked,
    setAsking: (on: boolean) => (asking = on),
    ask: async (question: string) => (asked.push(question), answers.shift() ?? false),
    isAsking: () => asking,
  }
}

/** Rules that note what the session did with them. */
function rules(more: Partial<EditorRules> = {}) {
  const done: string[] = []
  const r: EditorRules = {
    id: 'Mail',
    dirty: () => false,
    store: async () => SAVED,
    saved: () => done.push('saved'),
    closed: () => done.push('closed'),
    focus: () => done.push('focus'),
    fail: (message) => done.push(`fail: ${message}`),
    ...more,
  }
  return { done, rules: r }
}

describe('loading an editor', () => {
  it('counts as editing while it loads, and gives what it loaded', async () => {
    const session = editorSession(questions())
    const load = deferred<string>()
    const loading = session.load(() => load.promise)
    expect(session.isEditing()).toBe(true)
    load.resolve('data')
    expect(await loading).toBe('data')
  })

  it('gives nothing when the vault locked meanwhile', async () => {
    const session = editorSession(questions())
    const load = deferred<string>()
    const loading = session.load(() => load.promise)
    session.clear()
    load.resolve('data')
    expect(await loading).toBeNull()
    expect(session.isEditing()).toBe(false)
  })

  it('leaves no editor when loading fails', async () => {
    const session = editorSession(questions())
    await expect(session.load(() => Promise.reject(new Error('gone')))).rejects.toThrow('gone')
    expect(session.isEditing()).toBe(false)
  })
})

describe('saving', () => {
  it('closes the editor and hands over what was saved', async () => {
    const session = editorSession(questions())
    const { done, rules: r } = rules()
    session.open(r)
    await session.save()
    expect(done).toEqual(['saved'])
    expect(session.isEditing()).toBe(false)
  })

  it('saves once while a save runs', async () => {
    const session = editorSession(questions())
    const store = deferred<Saved>()
    let calls = 0
    const { done, rules: r } = rules({ store: () => (calls++, store.promise) })
    const { save } = session.open(r)
    const first = save()
    await save()
    store.resolve(SAVED)
    await first
    expect([calls, done]).toEqual([1, ['saved']])
  })

  it('does nothing more when the vault locked while saving', async () => {
    const session = editorSession(questions())
    const store = deferred<Saved>()
    const { done, rules: r } = rules({ store: () => store.promise })
    const saving = session.open(r).save()
    session.clear()
    store.resolve(SAVED)
    await saving
    expect(done).toEqual([])
  })

  it('says why a save failed and stays open', async () => {
    const session = editorSession(questions())
    const { done, rules: r } = rules({ store: () => Promise.reject('disk full') })
    await session.open(r).save()
    expect(done).toEqual(['fail: disk full'])
    expect(session.isEditing()).toBe(true)
  })

  it('says nothing of a failed save when the vault locked meanwhile', async () => {
    const session = editorSession(questions())
    const store = deferred<Saved>()
    const { done, rules: r } = rules({ store: () => store.promise })
    const saving = session.open(r).save()
    session.clear()
    store.reject('disk full')
    await saving
    expect(done).toEqual([])
  })
})

describe('cancelling', () => {
  it('closes at once when nothing changed', async () => {
    const q = questions()
    const session = editorSession(q)
    const { done, rules: r } = rules()
    await session.open(r).close()
    expect([q.asked, done]).toEqual([[], ['closed']])
    expect(session.isEditing()).toBe(false)
  })

  it('asks first when something changed, and keeps editing on No', async () => {
    const q = questions(false, true)
    const session = editorSession(q)
    const { done, rules: r } = rules({ dirty: () => true })
    const { close } = session.open(r)
    await close()
    expect([q.asked.length, done]).toEqual([1, []])
    await close()
    expect(done).toEqual(['closed'])
  })

  it('does nothing when the editor went away while asking', async () => {
    const session = editorSession({ ask: async () => (session.clear(), true), isAsking: () => false })
    const { done, rules: r } = rules({ dirty: () => true })
    await session.open(r).close()
    expect(done).toEqual([])
  })
})

describe('leaving for something else', () => {
  it('is free when there is no editor', async () => {
    expect(await editorSession(questions()).leave()).toBe(true)
  })

  it('closes without saving on Yes, and goes back to the form on No', async () => {
    const q = questions(true, false)
    const session = editorSession(q)
    const yes = rules()
    session.open(yes.rules)
    expect(await session.leave()).toBe(true)
    expect(yes.done).toEqual(['closed'])
    const no = rules()
    session.open(no.rules)
    expect(await session.leave()).toBe(false)
    expect(no.done).toEqual(['focus'])
    expect(session.isEditing()).toBe(true)
  })

  it('does not ask over another question, nor act on an answer given too late', async () => {
    const q = questions(true)
    const session = editorSession(q)
    const { done, rules: r } = rules()
    session.open(r)
    q.setAsking(true)
    expect(await session.leave()).toBe(false)
    expect(q.asked).toEqual([])
    const late = editorSession({ ask: async () => (late.clear(), true), isAsking: () => false })
    late.open(r)
    expect(await late.leave()).toBe(false)
    expect(done).toEqual([])
  })
})

describe('changes from another device', () => {
  it('are told only to the editor of that entry', () => {
    const session = editorSession(questions())
    const { done, rules: r } = rules()
    session.changedElsewhere(['Mail'])
    session.open(r)
    session.changedElsewhere(['Bank'])
    expect(done).toEqual([])
    session.changedElsewhere(['Bank', 'Mail'])
    expect(done).toEqual([`fail: ${CHANGED_ELSEWHERE}`])
  })

  it('are not told to the editor of a new entry', () => {
    const session = editorSession(questions())
    const { done, rules: r } = rules({ id: null })
    session.open(r)
    session.changedElsewhere(['Mail'])
    expect(done).toEqual([])
  })
})
