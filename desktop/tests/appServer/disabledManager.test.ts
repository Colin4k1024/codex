import { describe, expect, it } from 'vitest'
import { DisabledAppServerManager } from '../../src/main/appServer/disabledManager'

describe('DisabledAppServerManager', () => {
  it('reports a deterministic disabled status', () => {
    const manager = new DisabledAppServerManager()

    expect(manager.status).toEqual({
      state: 'Disconnected',
      pid: null,
      pendingRequests: 0,
      queuedMessages: 0,
      reconnectAttempt: null,
      lastError: {
        code: null,
        message: 'app-server integration is disabled',
        kind: 'notReady',
        retryable: false,
      },
    })
  })

  it('rejects lifecycle and RPC operations without spawning a process', async () => {
    const manager = new DisabledAppServerManager()

    await expect(manager.start()).rejects.toMatchObject({ kind: 'notReady' })
    await expect(manager.stop()).rejects.toMatchObject({ kind: 'notReady' })
    await expect(manager.request()).rejects.toMatchObject({ kind: 'notReady' })
    expect(() => manager.notify()).toThrowError('app-server integration is disabled')
    expect(manager.status.pid).toBeNull()
  })
})
