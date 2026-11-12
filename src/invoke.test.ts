import { describe, expect, it } from 'vitest'
import { CommandError, isCancelled, thrown } from './invoke'

describe('a failed command', () => {
  it('names the failure its caller acts on', () => {
    const error = thrown({ code: 'cancelled', message: 'Cancelled' })
    expect(error).toBeInstanceOf(CommandError)
    expect(isCancelled(error)).toBe(true)
  })

  it('is shown as its message, with a code or without', () => {
    expect(String(thrown({ code: null, message: 'Unlock with the master password' }))).toBe('Unlock with the master password')
    expect(String(thrown('The database is locked'))).toBe('The database is locked')
  })

  it('is cancelled by its code alone, never by its text', () => {
    expect(isCancelled(thrown('cancelled'))).toBe(false)
    expect(isCancelled(thrown({ code: null, message: 'cancelled' }))).toBe(false)
    expect(isCancelled(new Error('cancelled'))).toBe(false)
  })
})
