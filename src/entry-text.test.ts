import { describe, expect, it } from 'vitest'
import { formatGroup, formatTags, parseGroup, parseTags } from './entry-text'

describe('entry text', () => {
  it('reads and writes group paths', () => {
    expect(parseGroup(' Work /Mail/ ')).toEqual(['Work', 'Mail'])
    expect(parseGroup('')).toEqual([])
    expect(formatGroup(['Home', 'Money'])).toBe('Home / Money')
  })

  it('reads and writes tags', () => {
    expect(parseTags('a, b;c ,, a')).toEqual(['a', 'b', 'c'])
    expect(formatTags(['a', 'b'])).toBe('a, b')
  })
})
