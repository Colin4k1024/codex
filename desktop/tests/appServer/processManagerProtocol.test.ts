import { afterEach, describe, expect, it, vi } from 'vitest'
import { AppServerProcessManager } from '../../src/main/appServer/processManager'
import { createManager, startReady, waitForStatus } from './processManagerTestUtils'

const managers: AppServerProcessManager[] = []

async function trackedStartReady(mode: string): Promise<AppServerProcessManager> {
  const manager = await startReady(mode)
  managers.push(manager)
  return manager
}

describe('AppServerProcessManager protocol handling', () => {
  afterEach(async () => {
    for (const manager of managers.splice(0)) {
      manager.removeProcessExitHooks()
      await manager.stop()
    }
  })

  it('classifies unknown method responses', async () => {
    const manager = await trackedStartReady('unknown')

    await expect(manager.request('account/logout')).rejects.toMatchObject({
      kind: 'methodNotFound',
      code: -32601,
    })
  })

  it('classifies overloaded responses as retryable', async () => {
    const manager = await trackedStartReady('overloaded')

    await expect(manager.request('account/logout')).rejects.toMatchObject({
      kind: 'overloaded',
      code: -32001,
      retryable: true,
    })
  })

  it('reconnects after malformed server frames', async () => {
    const manager = await trackedStartReady('malformed')

    await waitForStatus(manager, 'Reconnecting')
    expect(manager.status.lastError).toMatchObject({ kind: 'parse', code: -32700 })

    await waitForStatus(manager, 'Ready')
  })

  it('reconnects on oversized frames', async () => {
    const manager = await trackedStartReady('huge')

    await waitForStatus(manager, 'Reconnecting')

    expect(manager.status.lastError).toMatchObject({ kind: 'parse' })
  })

  it('delivers server requests and responds with the original request id', async () => {
    const manager = await trackedStartReady('approvals')
    const events: unknown[] = []
    manager.onNotification((event) => {
      events.push(event)
    })

    await vi.waitFor(() => {
      expect(events).toContainEqual({
        id: 'approval-1',
        method: 'item/commandExecution/requestApproval',
        params: expect.objectContaining({ command: 'echo approval-integration' }),
      })
    })

    await manager.respondToServerRequest('approval-1', { decision: 'accept' })

    await vi.waitFor(() => {
      expect(events).toContainEqual({
        method: 'serverRequest/resolved',
        params: { threadId: 'integration-thread-1', requestId: 'approval-1' },
      })
    })
  })

  it('delivers server notifications in wire order', async () => {
    const manager = createManager({ mode: 'notify' })
    managers.push(manager)
    const events: unknown[] = []
    manager.onNotification((event) => {
      events.push(event)
    })

    await manager.start()
    await waitForStatus(manager, 'Ready')
    await vi.waitFor(() => {
      const ordered = events.filter(
        (event) => (event as { method: string }).method === 'test/ordered',
      )
      expect(
        ordered.map((event) => (event as { params: { sequence: number } }).params.sequence),
      ).toEqual([0, 1, 2])
    })
  })
})
