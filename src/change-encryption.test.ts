import { describe, expect, it } from 'vitest'
import { describeEncryption } from './change-encryption'

describe('describeEncryption', () => {
  it('names the cipher and the key derivation with its parameters', () => {
    expect(describeEncryption({ cipher: 'aes256', kdf: 'argon2id', iterations: 10, memory: 64 * 2 ** 20, parallelism: 2 }))
      .toBe('AES-256 · Argon2id, 64 MB, 10 iterations, 2 threads')
    expect(describeEncryption({ cipher: 'chaCha20', kdf: 'aes', iterations: 100000, memory: 0, parallelism: 0 }))
      .toBe('ChaCha20 · AES-KDF, 100,000 rounds')
    expect(describeEncryption({ cipher: 'other', kdf: 'argon2d', iterations: 2, memory: 2 ** 20, parallelism: 1 }))
      .toBe('Twofish (kept) · Argon2d, 1 MB, 2 iterations, 1 thread')
  })
})
