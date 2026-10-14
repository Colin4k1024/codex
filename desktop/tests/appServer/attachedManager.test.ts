import { describe, expect, it, vi } from 'vitest'
import { AttachedAppServerManager } from '../../src/main/appServer/attachedManager'

describe('AttachedAppServerManager', () => {
  it('starts in Disconnected state', () => {
    const manager = new AttachedAppServerManager('/tmp/test-daemon.sock')
    expect(manager.status.state).toBe('Disconnected')
  })

  it('transitions to Ready when socket exists', async () => {
    const fs = await import('node:fs/promises')
    const socketPath = '/tmp/test-daemon-attach.sock'
    await fs.writeFile(socketPath, '')

    const manager = new AttachedAppServerManager(socketPath)
    const status = await manager.start()

    expect(status.state).toBe('Ready')
    expect(manager.status.lastError).toBeNull()

    await fs.unlink(socketPath)
  })

  it('fails with deterministic error when socket is missing', async () => {
    const manager = new AttachedAppServerManager('/tmp/nonexistent-daemon.sock')
    const status = await manager.start()

    expect(status.state).toBe('Disconnected')
    expect(status.lastError).toMatchObject({
      kind: 'connectionClosed',
      message: expect.stringContaining('Daemon socket not found'),
    })
  })

  it('rejects RPC calls when not ready', async () => {
    const manager = new AttachedAppServerManager('/tmp/test-daemon.sock')
    await expect(manager.request('account/logout')).rejects.toMatchObject({ kind: 'notReady' })
  })

  it('rejects notifications when not ready', () => {
    const manager = new AttachedAppServerManager('/tmp/test-daemon.sock')
    expect(() => manager.notify('test/notify')).toThrowError(/not ready|Disconnected/)
  })

  it('rejects server request responses when not ready', async () => {
    const manager = new AttachedAppServerManager('/tmp/test-daemon.sock')
    await expect(manager.respondToServerRequest('req-1', {})).rejects.toMatchObject({
      kind: 'invalidRequest',
    })
  })

  it('cleans up listeners on dispose', () => {
    const manager = new AttachedAppServerManager('/tmp/test-daemon.sock')
    const statusListener = vi.fn()
    const notificationListener = vi.fn()

    manager.onStatus(statusListener)
    manager.onNotification(notificationListener)

    manager.dispose()

    expect(manager.status.state).toBe('Disconnected')
  })
})
