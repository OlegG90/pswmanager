import { describe, expect, it } from 'vitest'
import { EMPTY_ENTRY, fieldNameSuggestions, newEntryData } from './editor-parts'

describe('newEntryData', () => {
  const template = {
    ...EMPTY_ENTRY,
    title: 'Server',
    username: 'admin',
    otp: 'otpauth://totp/x?secret=JBSWY3DPEHPK3PXP',
    tags: ['Favorite', 'work'],
    icon: { kind: 'builtin' as const, id: 12 },
    expires: '2030-01-01T00:00:00Z',
    group: ['Templates'],
    fields: [
      { name: 'Port', value: '22', protected: false },
      { name: 'TOTP Seed', value: 'JBSWY3DPEHPK3PXP', protected: true },
      { name: 'TOTP Settings', value: '30;6', protected: false },
      { name: 'TimeOtp-Secret-Base32', value: 'JBSWY3DPEHPK3PXP', protected: true },
    ],
  }

  it('takes the fields, icon and tags from a template, not the title, expiry, TOTP, star or group', () => {
    const made = newEntryData([], { from: template, defaultUsername: 'me' })
    expect(made.title).toBe('')
    expect(made.expires).toBeNull()
    expect(made.otp).toBe('')
    expect(made.username).toBe('admin')
    expect(made.icon).toEqual({ kind: 'builtin', id: 12 })
    expect(made.tags).toEqual(['work'])
    expect(made.fields).toEqual([{ name: 'Port', value: '22', protected: false }])
    expect(made.group).toEqual([])
  })

  it('starts with the values given, over the default user name (#351)', () => {
    const otp = 'otpauth://totp/Shop:bob?secret=JBSWY3DPEHPK3PXP'
    expect(newEntryData([], { defaultUsername: 'me', prefill: { title: 'Shop', username: 'bob', otp } }))
      .toEqual({ ...EMPTY_ENTRY, title: 'Shop', username: 'bob', otp, group: [] })
    expect(newEntryData([], { defaultUsername: 'me', prefill: { password: 'x7#Kq' } }))
      .toEqual({ ...EMPTY_ENTRY, username: 'me', password: 'x7#Kq', group: [] })
  })

  it('gives only a blank entry the default user name', () => {
    expect(newEntryData(['Work'], { defaultUsername: 'me' })).toEqual({ ...EMPTY_ENTRY, username: 'me', group: ['Work'] })
    expect(newEntryData([], { template: true, defaultUsername: 'me' }).username).toBe('')
  })
})

describe('fieldNameSuggestions', () => {
  it('offers the usual names the entry does not have, and the first further URL', () => {
    expect(fieldNameSuggestions([])).toEqual(['Email', 'PIN', 'Code', 'KP2A_URL_1'])
    expect(fieldNameSuggestions(['email ', 'Port'])).toEqual(['PIN', 'Code', 'KP2A_URL_1'])
  })

  it('numbers the further URL after the highest one there', () => {
    expect(fieldNameSuggestions(['KP2A_URL_1', 'KP2A_URL_3', 'PIN'])).toEqual(['Email', 'Code', 'KP2A_URL_4'])
    expect(fieldNameSuggestions(['KP2A_URL_x', 'kp2a_url_2'])).toEqual(['Email', 'PIN', 'Code', 'KP2A_URL_1'])
  })
})
