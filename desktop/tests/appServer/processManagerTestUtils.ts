import path from 'node:path'
import { fileURLToPath } from 'node:url'
import type { AppServerStatus } from '../../src/shared/appServer'
import { AppServerProcessManager } from '../../src/main/appServer/processManager'

export const fixturePath = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  '../fixtures/fakeAppServer.mjs',
)

export function createManager(options: {
  mode: string
  reconnect?: { maxAttempts?: number; delayMs?: number; backoffFactor?: number }
  limits?: { requestTimeoutMs?: number; maxPendingRequests?: number; maxQueuedMessages?: number }
}): AppServerProcessManager {
  return new AppServerProcessManager({
    command: process.execPath,
    args: [fixturePath],
    env: { ...process.env, FAKE_MODE: options.mode },
    limits: { requestTimeoutMs: 750, ...options.limits },
    reconnect: { maxAttempts: 2, delayMs: 30, backoffFactor: 1, ...options.reconnect },
    attachProcessExitHooks: false,
  })
}

export function waitForStatus(
  manager: AppServerProcessManager,
  state: AppServerStatus['state'],
  timeoutMs = 5_000,
): Promise<AppServerStatus> {
  return new Promise((resolve, reject) => {
    if (manager.status.state === state) {
      resolve(manager.status)
      return
    }
    const timer = setTimeout(() => {
      manager.off('status', onStatus)
      reject(new Error(`timed out waiting for ${state}; state was ${manager.status.state}`))
    }, timeoutMs)
    const onStatus = (status: AppServerStatus): void => {
      if (status.state === state) {
        clearTimeout(timer)
        manager.off('status', onStatus)
        resolve(status)
      }
    }
    manager.on('status', onStatus)
  })
}

export function isProcessAlive(pid: number | null): boolean {
  if (pid === null) {
    return false
  }
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

export async function startReady(mode: string): Promise<AppServerProcessManager> {
  const manager = createManager({ mode })
  await manager.start()
  await waitForStatus(manager, 'Ready')
  return manager
}
