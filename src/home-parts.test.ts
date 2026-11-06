import { describe, expect, it } from 'vitest'
import { entryFromOtpUri, NEEDS } from './home-parts'

describe('a new entry from a QR code', () => {
  it('takes the title from the issuer and the user name from the label', () => {
    const uri = 'otpauth://totp/Example%20Co:alice%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=Example%20Co'
    expect(entryFromOtpUri(uri)).toEqual({ title: 'Example Co', username: 'alice@example.com', otp: uri })
  })

  it('falls back on the label’s issuer part, and keeps the default user name without an account', () => {
    expect(entryFromOtpUri('otpauth://totp/Shop:bob?secret=JBSWY3DPEHPK3PXP')).toMatchObject({ title: 'Shop', username: 'bob' })
    expect(entryFromOtpUri('otpauth://totp/Shop?secret=JBSWY3DPEHPK3PXP')).not.toHaveProperty('username')
  })

  it('keeps what is not a URI as the TOTP, with no title', () => {
    expect(entryFromOtpUri('not a uri')).toEqual({ title: '', otp: 'not a uri' })
  })
})

describe('the needs a password is made for', () => {
  it('start at a length within their range, which the backend allows', () => {
    for (const { options, lengths: [min, max] } of NEEDS) {
      expect(options.length).toBeGreaterThanOrEqual(min)
      expect(options.length).toBeLessThanOrEqual(max)
      expect(min).toBeGreaterThanOrEqual(4)
      expect(max).toBeLessThanOrEqual(64)
    }
  })
})
