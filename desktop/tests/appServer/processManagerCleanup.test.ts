import { EventEmitter } from 'node:events'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { SpawnFn } from '../../src/main/appServer/processManager'
import { AppServerProcessManager } from '../../src/main/appServer/processManager'
import { fixturePath, isProcessAlive, startReady } from './processManagerTestUtils'

class StubbornStream extends EventEmitter {
  setEncoding(): void {}
  write(frame: string): boolean {
    void frame
    return true
  }
}

class StubbornChild extends EventEmitter {
  readonly pid = 9876
  exitCode: number | null = null
  signalCode: NodeJS.Signals | null = null
  readonly stdout = new StubbornStream()
  readonly stderr = new StubbornStream()
  readonly stdin = new StubbornStream()

  kill(signal: NodeJS.Signals): boolean {
    if (signal === 'SIGKILL') {
      this.exitCode = 0
      this.emit('exit', 0, null)
    }
    return true
  }
}

const managers: AppServerProcessManager[] = []

describe('AppServerProcessManager cleanup', () => {
  afterEach(async () => {
    for (const manager of managers.splice(0)) {
      manager.removeProcessExitHooks()
      await manager.stop()
    }
  })

  it('kills the child process on stop without leaving an orphan', async () => {
    const manager = await startReady('echo')
    managers.push(manager)
    const pid = manager.status.pid
    expect(isProcessAlive(pid)).toBe(true)

    const status = await manager.stop()

    expect(status.state).toBe('Disconnected')
    expect(status.pid).toBeNull()
    await vi.waitFor(() => {
      expect(isProcessAlive(pid)).toBe(false)
    })
    expect(manager.status.pendingRequests).toBe(0)
  })

  it('fails deterministically when spawning is impossible', async () => {
    const spawnFn = (() => {
      throw new Error('simulated spawn failure')
    }) as unknown as SpawnFn
    const manager = new AppServerProcessManager({
      command: '/definitely/not/a/real/codex-binary',
      spawnFn,
      attachProcessExitHooks: false,
    })
    managers.push(manager)

    const status = await manager.start()

    expect(status.state).toBe('Disconnected')
    expect(status.lastError).toMatchObject({ kind: 'spawn' })
  })

  it('force-kills a handshake-failure child that ignores SIGTERM', async () => {
    const child = new StubbornChild()
    child.stdin.write = (frame: string): boolean => {
      const message = JSON.parse(frame) as { id?: number; method?: string }
      if (message.method === 'initialize') {
        queueMicrotask(() => {
          child.stdout.emit('data', `${JSON.stringify({ id: message.id, result: {} })}\n`)
        })
      }
      return true
    }
    const spawnFn = (() => child) as unknown as SpawnFn
    const manager = new AppServerProcessManager({
      command: 'stubborn-codex',
      spawnFn,
      attachProcessExitHooks: false,
    })
    managers.push(manager)

    const status = await manager.start()
    expect(status.state).toBe('Disconnected')
    expect(status.lastError).toMatchObject({ kind: 'invalidRequest' })
    expect(child.exitCode).toBeNull()

    await new Promise((resolve) => setTimeout(resolve, 2_100))
    expect(child.exitCode).toBe(0)
  })

  it('force-kills a pending child when dispose runs inside the grace period', async () => {
    const child = new StubbornChild()
    child.stdin.write = (frame: string): boolean => {
      const message = JSON.parse(frame) as { id?: number; method?: string }
      if (message.method === 'initialize') {
        queueMicrotask(() => {
          child.stdout.emit('data', `${JSON.stringify({ id: message.id, result: {} })}\n`)
        })
      }
      return true
    }
    const spawnFn = (() => child) as unknown as SpawnFn
    const manager = new AppServerProcessManager({
      command: 'stubborn-codex',
      spawnFn,
      attachProcessExitHooks: false,
    })
    managers.push(manager)

    await manager.start()
    expect(child.exitCode).toBeNull()

    manager.dispose()

    expect(child.exitCode).toBe(0)
  })

  it('kills pending children through the process exit hook', async () => {
    const child = new StubbornChild()
    child.stdin.write = (frame: string): boolean => {
      const message = JSON.parse(frame) as { id?: number; method?: string }
      if (message.method === 'initialize') {
        queueMicrotask(() => {
          child.stdout.emit('data', `${JSON.stringify({ id: message.id, result: {} })}\n`)
        })
      }
      return true
    }
    const spawnFn = (() => child) as unknown as SpawnFn
    const manager = new AppServerProcessManager({
      command: 'stubborn-codex',
      spawnFn,
      attachProcessExitHooks: true,
    })
    managers.push(manager)

    await manager.start()
    expect(child.exitCode).toBeNull()
    ;(process as { emit: (event: string) => boolean }).emit('exit')

    expect(child.exitCode).toBe(0)
  })

  it('kills the child synchronously on dispose (process-exit path)', async () => {
    const manager = new AppServerProcessManager({
      command: process.execPath,
      args: [fixturePath],
      env: { ...process.env, FAKE_MODE: 'echo' },
      limits: { requestTimeoutMs: 750 },
      attachProcessExitHooks: true,
    })
    managers.push(manager)

    await manager.start()
    await vi.waitFor(() => {
      expect(manager.status.state).toBe('Ready')
    })
    const pid = manager.status.pid

    manager.dispose()

    await vi.waitFor(() => {
      expect(isProcessAlive(pid)).toBe(false)
    })
    expect(manager.status.state).toBe('Disconnected')
  })
})
