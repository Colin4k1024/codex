import { describe, expect, it } from 'vitest'
import { normalizePlatform } from '../src/shared/platform'

describe('normalizePlatform', () => {
  it('returns supported platforms unchanged', () => {
    expect(normalizePlatform('darwin')).toBe('darwin')
    expect(normalizePlatform('linux')).toBe('linux')
    expect(normalizePlatform('win32')).toBe('win32')
  })

  it('falls back to linux for unsupported platforms', () => {
    expect(normalizePlatform('freebsd')).toBe('linux')
  })
})
