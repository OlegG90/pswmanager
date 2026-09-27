import { describe, expect, it } from 'vitest'
import { formatGroup, formatSize, formatTags, keep, parseGroup, parseTags, singleLine, textareaLines } from './entry-text'

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

  it('writes file sizes', () => {
    expect(formatSize(0)).toBe('0 B')
    expect(formatSize(1023)).toBe('1023 B')
    expect(formatSize(1024)).toBe('1 KB')
    expect(formatSize(2560)).toBe('2.5 KB')
    expect(formatSize(14_500)).toBe('14 KB')
    expect(formatSize(2.4 * 1024 * 1024)).toBe('2.4 MB')
    expect(formatSize(20 * 1024 * 1024)).toBe('20 MB')
  })
})
