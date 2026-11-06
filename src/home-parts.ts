import type { EntryData, GeneratorOptions } from './api'

/** What a password is made for on Home (#351): each need its own kinds of
 *  character and its range of lengths. */
export type Need = 'website' | 'pin' | 'wifi' | 'custom'

export interface NeedPreset {
  need: Need
  label: string
  /** What it makes, under the choice. */
  hint: string
  options: GeneratorOptions
  /** The length input's range: the backend allows 4 to 64. */
  lengths: [number, number]
}

const EVERY_KIND = { upper: true, lower: true, digits: true, symbols: true, excludeLookAlikes: true }

export const NEEDS: NeedPreset[] = [
  { need: 'website', label: 'Website', hint: 'Every kind of character, for a site or an app.',
    options: { length: 20, ...EVERY_KIND }, lengths: [12, 64] },
  { need: 'pin', label: 'PIN', hint: 'Digits only, for a card, a phone or a lock.',
    options: { length: 6, upper: false, lower: false, digits: true, symbols: false, excludeLookAlikes: false }, lengths: [4, 12] },
  // WPA2 takes 8 to 63 characters; with no symbols or look-alikes the key is
  // easy to read off a label and type on a TV or a phone.
  { need: 'wifi', label: 'Wi-Fi', hint: 'Letters and digits without look-alikes, easy to type in by hand.',
    options: { length: 16, upper: true, lower: true, digits: true, symbols: false, excludeLookAlikes: true }, lengths: [8, 63] },
  { need: 'custom', label: 'Custom', hint: 'The kinds of character you choose.',
    options: { length: 20, ...EVERY_KIND }, lengths: [8, 64] },
]

/** A new entry's values from an `otpauth://` URI a QR code held: the title
 *  from its issuer, the user name from the rest of its label
 *  (`otpauth://totp/Issuer:account?secret=…&issuer=Issuer`), the URI as its TOTP. */
export function entryFromOtpUri(uri: string): Partial<EntryData> {
  let label = ''
  let issuer = ''
  try {
    const url = new URL(uri)
    label = decodeURIComponent(url.pathname.replace(/^\/+/, ''))
    issuer = url.searchParams.get('issuer')?.trim() ?? ''
  } catch {
    // Not a URL: the TOTP field says what is wrong with it.
  }
  const colon = label.indexOf(':')
  const prefix = colon < 0 ? '' : label.slice(0, colon).trim()
  const account = (colon < 0 ? label : label.slice(colon + 1)).trim()
  const title = issuer || prefix || account
  // With no account of its own, the entry keeps the database's default user name.
  return { title, ...(account && account !== title ? { username: account } : {}), otp: uri }
}
