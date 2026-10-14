import { afterEach, describe, expect, it, vi } from 'vitest'
import { AppServerProcessManager } from '../../src/main/appServer/processManager'
import { createManager, isProcessAlive, startReady, waitForStatus } from './processManagerTestUtils'

const managers: AppServerProcessManager[] = []

describe('AppServerProcessManager lifecycle', () => {
  afterEach(async () => {
    for (const manager of managers.splice(0)) {
      manager.removeProcessExitHooks()
      await manager.stop()
    }
  })

  it('starts a single process and reaches Ready through initialize', async () => {
    const manager = await startReady('echo')
    managers.push(manager)
    const pid = manager.status.pid
    expect(pid).toBeTypeOf('number')

    const secondStart = await manager.start()
    expect(secondStart.pid).toBe(pid)
    expect(secondStart.state).toBe('Ready')
  })

  it('rejects business RPC before initialize completes', async () => {
    const manager = createManager({ mode: 'noResponse' })
    managers.push(manager)

    const promise = manager.start()
    const initializing = await waitForStatus(manager, 'Initializing')
    expect(initializing.state).toBe('Initializing')

    await expect(manager.request('account/logout')).rejects.toMatchObject({ kind: 'notReady' })
    await expect(promise).resolves.toMatchObject({ state: 'Disconnected' })
    expect(manager.status.lastError).toMatchObject({ kind: 'timeout' })
  })

  it('round-trips typed method names and params', async () => {
    const manager = await startReady('echo')
    managers.push(manager)

    await expect(manager.request('account/logout')).resolves.toBeNull()
    await expect(manager.request('config/mcpServer/reload')).resolves.toBeNull()
  })

  it('fails closed on an invalid initialize response and kills the child', async () => {
    const manager = createManager({ mode: 'badInit' })
    managers.push(manager)

    const startPromise = manager.start()
    await waitForStatus(manager, 'Initializing')
    const pid = manager.status.pid

    await startPromise

    expect(manager.status.state).toBe('Disconnected')
    expect(manager.status.lastError).toMatchObject({ kind: 'invalidRequest' })
    expect(manager.status.pid).toBeNull()
    await vi.waitFor(() => {
      expect(isProcessAlive(pid)).toBe(false)
    })
  })

  it('classifies shutdown admission and reconnects', async () => {
    const manager = await startReady('shutdown')
    managers.push(manager)

    await expect(manager.request('account/logout')).rejects.toMatchObject({
      kind: 'serverShuttingDown',
      code: -32600,
      data: { reason: 'serverShuttingDown' },
    })

    expect((await waitForStatus(manager, 'Reconnecting')).state).toBe('Reconnecting')
    await waitForStatus(manager, 'Ready')
  })

  it('fails pending requests and reconnects when the process exits', async () => {
    const manager = await startReady('slow')
    managers.push(manager)

    const pending = manager.request('account/logout')
    const pid = manager.status.pid
    process.kill(pid as number, 'SIGKILL')

    await expect(pending).rejects.toMatchObject({ kind: 'connectionClosed' })
    await waitForStatus(manager, 'Reconnecting')
    await waitForStatus(manager, 'Ready')
    expect(manager.status.pendingRequests).toBe(0)
  })

  it('stops reconnecting after the bounded attempt count', async () => {
    const manager = createManager({
      mode: 'crash',
      reconnect: { maxAttempts: 1, delayMs: 10 },
    })
    managers.push(manager)

    await manager.start()
    await waitForStatus(manager, 'Reconnecting')
    const finalStatus = await waitForStatus(manager, 'Disconnected')

    expect(finalStatus.lastError).toMatchObject({ kind: 'connectionClosed' })
    expect(finalStatus.reconnectAttempt).toBeNull()
  })
})
