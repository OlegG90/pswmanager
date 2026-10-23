import type { GauthProgress } from './api'
import { el } from './dom'

/** What both apps' Import from Google Authenticator says (docs/spec.md,
 *  *Import from Google Authenticator*, #281); each draws it in its own way. */
export function gauthAbout(how: string): Node[] {
  return [
    el('p', {}, 'In Google Authenticator: ', el('b', {}, 'Transfer accounts → Export accounts'), ', choose the accounts, and it shows one or more QR codes.'),
    el('p', {}, how),
    el('p', {}, 'Each account is added as a new entry in a group of their own. The codes are kept in memory only, until they are imported or you leave.'),
  ]
}

/** How far the reading is: which of the export's codes are read, and the accounts in them. */
export function progressText({ codes, read, accounts }: GauthProgress): string {
  const missing = Array.from({ length: codes }, (_, i) => i + 1).filter((n) => !read.includes(n))
  const held = `${accounts} ${accounts === 1 ? 'account' : 'accounts'}`
  if (!missing.length) return codes === 1 ? `The code is read: ${held}.` : `All ${codes} codes are read: ${held}.`
  return `Read ${read.length} of ${codes} codes (${held}); still to read: ${missing.join(', ')}.`
}

/** The button that imports: all of it, or what is read so far. */
export const importLabel = ({ codes, read }: GauthProgress) => (read.length === codes ? 'Import' : 'Import what is read')
