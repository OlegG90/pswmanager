import { describe, expect, it } from 'vitest'
import { masterPasswordProblem, unmetRules } from './master-password'

describe('the rules for a new master password', () => {
  it('asks for length, both cases and a symbol', () => {
    expect(unmetRules('')).toHaveLength(3)
    expect(unmetRules('abcdef')).toEqual(['Upper- and lower-case letters', 'A character that is neither a letter nor a digit'])
    expect(unmetRules('Ab1!')).toEqual(['At least 6 characters'])
    expect(unmetRules('Abcde!')).toEqual([])
  })

  it('takes letters of any alphabet, and a space as a symbol', () => {
    expect(unmetRules('Пароль ключ')).toEqual([])
    expect(unmetRules('ПАРОЛЬ1')).toContain('Upper- and lower-case letters')
    expect(unmetRules('Abcdef1')).toEqual(['A character that is neither a letter nor a digit'])
  })

  it('says what is missing, but leaves an empty password to the key file', () => {
    expect(masterPasswordProblem('')).toBeNull()
    expect(masterPasswordProblem('Abcde!')).toBeNull()
    expect(masterPasswordProblem('abc')).toBe('The master password needs: at least 6 characters; upper- and lower-case letters; ' +
      'a character that is neither a letter nor a digit')
  })
})
