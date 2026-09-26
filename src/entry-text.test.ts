import { describe, expect, it } from 'vitest'
import { formatGroup, formatTags, keep, parseGroup, parseTags, singleLine, textareaLines } from './entry-text'

describe('entry text', () => {
  it('reads and writes group paths', () => {
    expect(parseGroup(' Work / Mail / ')).toEqual(['Work', 'Mail'])
    expect(parseGroup('TCP/IP / Lab')).toEqual(['TCP/IP', 'Lab'])
    expect(parseGroup('')).toEqual([])
    expect(formatGroup(['Home', 'Money'])).toBe('Home / Money')
  })

  it('keeps originals the editor only reformatted', () => {
    expect(keep('a\r\nb', 'ab', singleLine)).toBe('a\r\nb')
    expect(keep('a\r\nb', 'ax', singleLine)).toBe('ax')
    expect(keep('a\r\nb', 'a\nb', textareaLines)).toBe('a\r\nb')
    expect(keep([' Mail '], ['Mail'], (g) => parseGroup(formatGroup(g)))).toEqual([' Mail '])
  })

  it('reads and writes tags', () => {
    expect(parseTags('a, b;c ,, a')).toEqual(['a', 'b', 'c'])
    expect(formatTags(['a', 'b'])).toBe('a, b')
  })
})
