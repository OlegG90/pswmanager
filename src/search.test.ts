import { describe, expect, it } from 'vitest'
import type { Entry } from './api'
import { filterChoices, search } from './search'

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
  ...more,
})

const entries = [
  entry('Mail', { username: 'me@example.com', group: ['Work'], tags: ['Favorite'] }),
  entry('Bank', { url: 'bank.example.org', group: ['Home', 'Money'], notes: 'card PIN is elsewhere' }),
  entry('Router', { group: ['Home'] }),
]
const titles = (list: Entry[]) => list.map((e) => e.title)

describe('search', () => {
  it('matches every word anywhere except secrets, ignoring case', () => {
    expect(titles(search(entries, ''))).toEqual(['Mail', 'Bank', 'Router'])
    expect(titles(search(entries, 'EXAMPLE'))).toEqual(['Mail', 'Bank'])
    expect(titles(search(entries, 'bank pin'))).toEqual(['Bank'])
    expect(titles(search(entries, 'home'))).toEqual([])
    expect(titles(search(entries, 'favorite'))).toEqual(['Mail'])
    expect(titles(search(entries, 'nothing'))).toEqual([])
  })

  it('narrows to a group with its subgroups, or to a tag', () => {
    expect(titles(search(entries, '', { kind: 'group', path: 'Home' }))).toEqual(['Bank', 'Router'])
    expect(titles(search(entries, '', { kind: 'group', path: 'Home / Money' }))).toEqual(['Bank'])
    expect(titles(search(entries, '', { kind: 'group', path: 'Hom' }))).toEqual([])
    expect(titles(search(entries, 'router', { kind: 'tag', tag: 'Favorite' }))).toEqual([])
  })

  it('offers every group with its parents, and every tag', () => {
    expect(filterChoices(entries)).toEqual({ groups: ['Home', 'Home / Money', 'Work'], tags: ['Favorite'] })
  })
})
