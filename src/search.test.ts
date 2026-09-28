import { describe, expect, it } from 'vitest'
import type { Entry } from './api'
import { expiry, search, tagCounts, type Filter } from './search'

const entry = (title: string, more: Partial<Entry> = {}): Entry => ({
  id: title,
  title,
  username: '',
  url: '',
  host: null,
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

const NOW = Date.parse('2030-01-01T00:00:00Z')

const entries = [
  entry('Mail', { username: 'me@example.com', tags: ['Favorite', 'work'], otp: true }),
  entry('Bank', { url: 'bank.example.org', notes: 'card PIN is elsewhere', tags: ['work'], expires: '2029-12-31T00:00:00Z' }),
  entry('Router', { passkey: true, expires: '2030-01-10T00:00:00Z' }),
  entry('Shop', { expires: '2030-03-01T00:00:00Z', tags: ['home'] }),
  entry('Card', { kind: 'template', tags: ['work'] }),
  entry('Old', { kind: 'trash', tags: ['Favorite', 'gone'], otp: true }),
]
const titles = (list: Entry[]) => list.map((e) => e.title)
const group = (g: Extract<Filter, { kind: 'group' }>['group']): Filter => ({ kind: 'group', group: g })

describe('search', () => {
  it('matches every word anywhere except secrets, ignoring case', () => {
    expect(titles(search(entries, '', undefined, NOW))).toEqual(['Mail', 'Bank', 'Router', 'Shop'])
    expect(titles(search(entries, 'EXAMPLE'))).toEqual(['Mail', 'Bank'])
    expect(titles(search(entries, 'bank pin'))).toEqual(['Bank'])
    expect(titles(search(entries, 'favorite'))).toEqual(['Mail'])
    expect(titles(search(entries, 'nothing'))).toEqual([])
  })

  it('shows each group, templates and the trash only in their own', () => {
    const shown = (g: Parameters<typeof group>[0]) => titles(search(entries, '', group(g), NOW))
    expect(shown('favorites')).toEqual(['Mail'])
    expect(shown('expired')).toEqual(['Bank', 'Router'])
    expect(shown('2fa')).toEqual(['Mail'])
    expect(shown('passkey')).toEqual(['Router'])
    expect(shown('templates')).toEqual(['Card'])
    expect(shown('trash')).toEqual(['Old'])
    expect(titles(search(entries, 'old', group('trash')))).toEqual(['Old'])
  })

  it('narrows to a tag among the entries in use', () => {
    expect(titles(search(entries, '', { kind: 'tag', tag: 'work' }))).toEqual(['Mail', 'Bank'])
    expect(titles(search(entries, 'bank', { kind: 'tag', tag: 'work' }))).toEqual(['Bank'])
  })

  it('counts the tags in use, without the star', () => {
    expect(tagCounts(entries)).toEqual([['home', 1], ['work', 2]])
  })

  it('tells expired from expiring within 14 days', () => {
    expect(entries.map((e) => expiry(e, NOW))).toEqual([null, 'expired', 'soon', null, null, null])
  })
})
