import type { AppServerNotificationEvent } from '../../shared/appServer'
import type { AppServerIpcManager } from './ipc'
import { localError } from './rpcError'

/**
 * No-op manager used when the app-server integration feature flag is disabled.
 * It preserves the typed IPC surface but never spawns a process or accepts RPC.
 */
export class DisabledAppServerManager implements AppServerIpcManager {
  readonly status = {
    state: 'Disconnected',
    pid: null,
    pendingRequests: 0,
    queuedMessages: 0,
    reconnectAttempt: null,
    lastError: localError('notReady', 'app-server integration is disabled'),
  } as const

  onStatus(): () => void {
    return () => undefined
  }

  onNotification(_listener: (event: AppServerNotificationEvent) => void): () => void {
    return () => undefined
  }

  start(): Promise<never> {
    return Promise.reject(this.status.lastError)
  }

  stop(): Promise<never> {
    return Promise.reject(this.status.lastError)
  }

  request(): Promise<never> {
    return Promise.reject(this.status.lastError)
  }

  notify(): void {
    throw this.status.lastError
  }

  respondToServerRequest(): Promise<never> {
    return Promise.reject(this.status.lastError)
  }
  dispose(): void {
    // Disabled manager never owns a child process.
  }
}
