import { describe, expect, it } from 'vitest'
import { dateOf, formatSize, formatTags, keep, parseTags, singleLine, startOfDay, textareaLines } from './entry-text'

describe('entry text', () => {
  it('keeps originals the editor only reformatted', () => {
    expect(keep('a\r\nb', 'ab', singleLine)).toBe('a\r\nb')
    expect(keep('a\r\nb', 'ax', singleLine)).toBe('ax')
    expect(keep('a\r\nb', 'a\nb', textareaLines)).toBe('a\r\nb')
    expect(keep(['a ', 'b'], ['a', 'b'], (t) => parseTags(formatTags(t)))).toEqual(['a ', 'b'])
  })

  it('reads and writes tags', () => {
    expect(parseTags('a, b;c ,, a')).toEqual(['a', 'b', 'c'])
    expect(formatTags(['a', 'b'])).toBe('a, b')
  })

  it('reads and writes expiry days on this PC', () => {
    expect(dateOf(startOfDay('2030-01-02'))).toBe('2030-01-02')
    expect(new Date(startOfDay('2030-01-02')).getHours()).toBe(0)
    expect(dateOf(new Date(2030, 11, 31, 23, 59).toISOString())).toBe('2030-12-31')
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
