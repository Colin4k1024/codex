import { EventEmitter } from 'node:events'
import { describe, expect, it, vi } from 'vitest'
import type { ChildProcessWithoutNullStreams } from 'node:child_process'
import { AppServerProcessManager } from '../../src/main/appServer/processManager'

class FakeStream extends EventEmitter {
  setEncoding(): void {}
  write(frame: string): boolean {
    void frame
    return true
  }
}

class FakeChild extends EventEmitter {
  readonly pid = 4321
  exitCode: number | null = null
  signalCode: NodeJS.Signals | null = null
  readonly stdout = new FakeStream()
  readonly stderr = new FakeStream()
  readonly stdin = new FakeStream()

  kill(): boolean {
    return true
  }
}

function createFakeManager() {
  const child = new FakeChild()
  const manager = new AppServerProcessManager({
    command: 'fake-codex',
    args: [],
    spawnFn: () => child as unknown as ChildProcessWithoutNullStreams,
    reconnect: { maxAttempts: 1, delayMs: 1, backoffFactor: 1 },
    attachProcessExitHooks: false,
  })

  child.stdin.write = (frame: string): boolean => {
    const message = JSON.parse(frame) as { id?: number; method?: string }
    if (message.method === 'initialize') {
      queueMicrotask(() => {
        child.stdout.emit(
          'data',
          `${JSON.stringify({
            id: message.id,
            result: {
              userAgent: 'fake-app-server',
              codexHome: '/fake/codex-home',
              platformFamily: 'unix',
              platformOs: 'macos',
            },
          })}\n`,
        )
      })
    }
    return true
  }

  return { child, manager }
}

describe('AppServerProcessManager stream disconnection', () => {
  it('reconnects when stdin closes while the process remains alive', async () => {
    const { child, manager } = createFakeManager()

    await manager.start()
    await vi.waitFor(() => {
      expect(manager.status.state).toBe('Ready')
    })

    child.stdin.emit('close')

    await vi.waitFor(() => {
      expect(manager.status.state).toBe('Reconnecting')
    })
    expect(manager.status.lastError).toMatchObject({ kind: 'connectionClosed' })
  })

  it('reconnects when stdout closes while the process remains alive', async () => {
    const { child, manager } = createFakeManager()

    await manager.start()
    await vi.waitFor(() => {
      expect(manager.status.state).toBe('Ready')
    })

    child.stdout.emit('close')

    await vi.waitFor(() => {
      expect(manager.status.state).toBe('Reconnecting')
    })
    expect(manager.status.lastError).toMatchObject({ kind: 'connectionClosed' })
  })
})
