import { describe, expect, it } from 'vitest'
import type { Entry, EntryDetail, Listing, VersionDetail } from './api'
import { ALL, tagsOf, TEMPLATES, TRASH } from './search'
import { EMPTY, vaultState } from './vault'

const entry = (title: string, more: Partial<Entry> = {}): Entry => ({
  id: title,
  title,
  username: '',
  url: '',
  host: null,
  link: false,
  group: [],
  tags: [],
  notes: '',
  customIcon: null,
  icon: null,
  hasPassword: true,
  kind: 'entry',
  otp: false,
  passkey: false,
  expires: null,
  ...more,
})

const detail = (e: Entry): EntryDetail => ({ ...e, fields: [], attachments: [], modified: null, versions: 2 })
const version = (e: Entry): VersionDetail => ({ ...detail(e), differs: [] })
const listing = (...entries: Entry[]): Listing => ({ ...EMPTY, entries })

const mail = entry('Mail', { tags: ['work'] })
const bank = entry('Bank', { tags: ['work', 'Favorite'] })
const shop = entry('Shop')
const login = entry('Login', { kind: 'template' })
const old = entry('Old', { kind: 'trash', tags: ['work'] })

/** A vault open on the five entries, the list searched, its first entry shown. */
function opened() {
  const vault = vaultState()
  vault.open(listing(mail, bank, shop, login, old))
  vault.search('')
  vault.select('Mail')
  vault.show('Mail', detail(mail))
  return vault
}

const plain = { ctrl: false, shift: false }
const ctrl = { ctrl: true, shift: false }
const shift = { ctrl: false, shift: true }

describe('the list', () => {
  it('chooses its first entry, and keeps the one selected while it is still shown', () => {
    const vault = vaultState()
    vault.open(listing(mail, bank, shop))
    expect(vault.search('')).toEqual({ one: 'Mail' })
    vault.select('Bank')
    expect(vault.search('ba')).toEqual({ one: 'Bank' })
    expect(vault.search('shop')).toEqual({ one: 'Shop' })
    expect(vault.search('nothing')).toEqual({ one: null })
  })

  it('keeps several chosen while more than one is still shown', () => {
    const vault = opened()
    vault.chooseSeveral({ chosen: ['Mail', 'Bank', 'Shop'], anchor: 'Bank' })
    expect(vault.search('a')).toEqual({ several: { chosen: ['Mail', 'Bank'], anchor: 'Bank' } })
    // One left: it alone, and the several are let go of.
    expect(vault.search('mail')).toEqual({ one: 'Mail' })
    expect(vault.several).toBeNull()
  })

  it('chooses one with a plain click, several with Ctrl and Shift', () => {
    const vault = opened()
    expect(vault.pick('Shop', plain)).toEqual({ one: 'Shop' })
    // Ctrl+click adds to the entry selected.
    expect(vault.pick('Shop', ctrl)).toEqual({ several: { chosen: ['Mail', 'Shop'], anchor: 'Shop' } })
    expect(vault.pick('Shop', shift)).toEqual({ several: { chosen: ['Mail', 'Bank', 'Shop'], anchor: 'Mail' } })
    // Ctrl+click on the only one chosen keeps it.
    expect(vault.pick('Mail', ctrl)).toEqual({ one: 'Mail' })
  })

  it('steps from the entry selected, or from the last of several clicked, within the list', () => {
    const vault = opened()
    expect(vault.step(1)).toBe('Bank')
    expect(vault.step(-1)).toBe('Mail')
    vault.chooseSeveral({ chosen: ['Mail', 'Shop'], anchor: 'Shop' })
    expect(vault.step(1)).toBe('Shop')
    expect(vault.step(-1)).toBe('Bank')
    vault.search('nothing')
    expect(vault.step(1)).toBeNull()
  })

  it('sorts the chosen entries by what can be done to them', () => {
    const vault = opened()
    expect(vault.chosen()).toEqual({ all: [], inUse: [], trashed: [], templates: [] })
    const { all, inUse, trashed, templates } = vault.chosen(['Old', 'Login', 'Bank'])
    expect(all.map((e) => e.id)).toEqual(['Bank', 'Login', 'Old'])
    expect([inUse, trashed, templates].map((l) => l.map((e) => e.id))).toEqual([['Bank'], ['Old'], ['Login']])
  })

  it('opens the list menu on all the chosen entries, or on the one clicked', () => {
    const vault = opened()
    vault.chooseSeveral({ chosen: ['Mail', 'Bank'], anchor: 'Bank' })
    expect(vault.menuTargets('Bank')).toEqual(['Mail', 'Bank'])
    expect(vault.menuTargets('Shop')).toEqual(['Shop'])
  })
})

describe('the sidebar', () => {
  it('goes back to All when no entry in use has the tag shown any more', () => {
    const vault = opened()
    vault.setFilter({ kind: 'tag', tag: 'work' })
    vault.apply(listing(mail, shop))
    expect(vault.filter).toEqual({ kind: 'tag', tag: 'work' })
    // An entry in the trash with it does not keep it.
    vault.apply(listing(shop, old))
    expect(vault.filter).toEqual(ALL)
  })

  it('follows the tag shown when it is renamed', () => {
    const vault = opened()
    vault.setFilter({ kind: 'tag', tag: 'work' })
    vault.renameTag('home', 'house')
    expect(vault.filter).toEqual({ kind: 'tag', tag: 'work' })
    vault.renameTag('work', 'job')
    expect(vault.filter).toEqual({ kind: 'tag', tag: 'job' })
  })

  it('counts the tags of entries in use, and every entry with a tag', () => {
    const vault = opened()
    expect(vault.tags()).toEqual([['work', 2]])
    expect(vault.tagged('work')).toBe(3)
    expect(tagsOf([mail, bank, shop])).toEqual(['work'])
  })

  it('opens a database on All', () => {
    const vault = vaultState()
    vault.setFilter(TRASH)
    vault.open(listing(mail))
    expect(vault.filter).toEqual(ALL)
  })
})

describe('finding an entry', () => {
  it('leaves the list as it is when the entry is in it', () => {
    const vault = opened()
    vault.setFilter({ kind: 'tag', tag: 'work' })
    expect(vault.find('Bank', 'ba')).toBe(true)
    expect(vault.filter).toEqual({ kind: 'tag', tag: 'work' })
  })

  it('shows All, or the templates for a template, and asks for the search to go', () => {
    const vault = opened()
    vault.setFilter({ kind: 'tag', tag: 'work' })
    expect(vault.find('Shop', '')).toBe(false)
    expect(vault.filter).toEqual(ALL)
    expect(vault.find('Login', 'log')).toBe(false)
    expect(vault.filter).toEqual(TEMPLATES)
  })

  it('selects a saved entry, in the listing it was saved into', () => {
    const vault = opened()
    const added = entry('New')
    expect(vault.saved('New', listing(mail, bank, shop, added), 'mail')).toBe(false)
    expect(vault.selectedId).toBe('New')
    expect(vault.search('')).toEqual({ one: 'New' })
  })

  it('masks again what was revealed before a save, in the entry saved or another', () => {
    const vault = opened()
    vault.reveal('Mail', 'Password', 'before the edit')
    vault.saved('Mail', listing(mail, bank), '')
    expect(vault.revealedValue('Password')).toBeUndefined()
    vault.show('Mail', detail(mail))
    vault.reveal('Mail', 'Password', 'of Mail')
    vault.saved('New', listing(mail, entry('New')), '')
    expect(vault.revealedValue('Password')).toBeUndefined()
  })
})

describe('the entry shown', () => {
  it('is fetched when another one is selected, and its revealed values go', () => {
    const vault = opened()
    expect(vault.reveal('Mail', 'Password', 'secret')).toBe(true)
    // Selected again, it stays as it is.
    expect(vault.select('Mail')).toBe(false)
    expect(vault.revealedValue('Password')).toBe('secret')
    expect(vault.select('Bank')).toBe(true)
    expect(vault.revealedValue('Password')).toBeUndefined()
    expect(vault.current).toEqual(detail(mail))
    // An answer for an entry no longer selected is dropped.
    expect(vault.show('Mail', detail(mail))).toBe(false)
    expect(vault.show('Bank', detail(bank))).toBe(true)
    expect(vault.current?.id).toBe('Bank')
  })

  it('empties with no entry selected', () => {
    const vault = opened()
    expect(vault.select(null)).toBe(true)
    expect(vault.current).toBeNull()
  })

  it('reveals a value only in the entry it was asked for', () => {
    const vault = opened()
    expect(vault.reveal('Bank', 'Password', 'secret')).toBe(false)
    expect(vault.revealedValue('Password')).toBeUndefined()
    vault.reveal('Mail', 'Password', 'secret')
    expect(vault.hide('Password')).toBe(true)
    expect(vault.hide('Password')).toBe(false)
  })

  it('goes back from a version to the history, then to the entry', () => {
    const vault = opened()
    const versions = [{ modified: null, changed: ['Password'] }]
    vault.showHistory(versions)
    expect(vault.shownVersion()).toBeNull()
    vault.showVersion(versions, 0, version(mail))
    expect(vault.shownVersion()).toBe(0)
    expect(vault.shownDetail()).toEqual(version(mail))
    vault.reveal('Mail', 'Password', 'older')
    expect(vault.back()).toBe(true)
    expect(vault.view).toEqual({ kind: 'history', versions })
    expect(vault.revealedValue('Password')).toBeUndefined()
    expect(vault.back()).toBe(true)
    expect(vault.view).toEqual({ kind: 'entry' })
    expect(vault.back()).toBe(false)
  })

  it('shows from the top after a change, a version\'s revealed values gone', () => {
    const vault = opened()
    vault.showVersion([], 0, version(mail))
    vault.reveal('Mail', 'Password', 'older')
    vault.apply(listing(mail, bank))
    expect(vault.view).toEqual({ kind: 'entry' })
    expect(vault.current).toBeNull()
    expect(vault.revealedValue('Password')).toBeUndefined()
  })

  it('keeps its revealed values after a change, unless the change touched it', () => {
    const vault = opened()
    vault.reveal('Mail', 'Password', 'secret')
    // Another entry starred, merged or changed on another device.
    vault.apply(listing(mail, bank), ['Bank', 'Shop'])
    expect(vault.revealedValue('Password')).toBe('secret')
    vault.show('Mail', detail(mail))
    // The entry shown changed on another device, or was merged.
    vault.apply(listing(mail, bank), ['Bank', 'Mail'])
    expect(vault.revealedValue('Password')).toBeUndefined()
  })

  it('is forgotten with everything else on lock', () => {
    const vault = opened()
    vault.reveal('Mail', 'Password', 'secret')
    vault.close()
    expect([vault.listing, vault.shown, vault.selectedId, vault.current]).toEqual([EMPTY, [], null, null])
    expect(vault.revealedValue('Password')).toBeUndefined()
  })
})
