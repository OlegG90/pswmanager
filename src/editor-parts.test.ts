import { describe, expect, it } from 'vitest'
import { EMPTY_ENTRY, fromTemplate } from './editor-parts'

describe('fromTemplate', () => {
  const template = {
    ...EMPTY_ENTRY,
    title: 'Server',
    username: 'admin',
    otp: 'otpauth://totp/x?secret=JBSWY3DPEHPK3PXP',
    tags: ['Favorite', 'work'],
    icon: { kind: 'builtin' as const, id: 12 },
    expires: '2030-01-01T00:00:00Z',
    fields: [
      { name: 'Port', value: '22', protected: false },
      { name: 'TOTP Seed', value: 'JBSWY3DPEHPK3PXP', protected: true },
      { name: 'TOTP Settings', value: '30;6', protected: false },
      { name: 'TimeOtp-Secret-Base32', value: 'JBSWY3DPEHPK3PXP', protected: true },
    ],
  }

  it('takes the fields, icon and tags, not the title, expiry, TOTP or star', () => {
    const made = fromTemplate(template)
    expect(made.title).toBe('')
    expect(made.expires).toBeNull()
    expect(made.otp).toBe('')
    expect(made.username).toBe('admin')
    expect(made.icon).toEqual({ kind: 'builtin', id: 12 })
    expect(made.tags).toEqual(['work'])
    expect(made.fields).toEqual([{ name: 'Port', value: '22', protected: false }])
  })
})
