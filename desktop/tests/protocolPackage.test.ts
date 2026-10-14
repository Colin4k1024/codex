import { describe, expect, it } from 'vitest'
import { PROTOCOL_FILE_COUNT, PROTOCOL_REVISION } from '@codex/app-server-protocol'

describe('@codex/app-server-protocol', () => {
  it('exposes the generated protocol revision', () => {
    expect(PROTOCOL_REVISION).toMatch(/^[a-f0-9]{64}$/)
    expect(PROTOCOL_FILE_COUNT).toBeGreaterThan(700)
  })
})
