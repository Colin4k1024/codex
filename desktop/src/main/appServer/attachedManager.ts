import type { AppServerNotificationEvent } from '../../shared/appServer'
import type { AppServerIpcManager } from './ipc'
import { localError } from './rpcError'

/**
 * Manager that attaches to an existing app-server daemon instead of spawning a child process.
 * Used when CODEX_DESKTOP_DAEMON_SOCKET is set to a valid daemon socket path.
 */
export class AttachedAppServerManager implements AppServerIpcManager {
  private readonly socketPath: string
  private readonly listeners: {
    status: Set<(status: AppServerIpcManager['status']) => void>
    notification: Set<(event: AppServerNotificationEvent) => void>
  } = {
    status: new Set(),
    notification: new Set(),
  }

  private state: AppServerIpcManager['status']['state'] = 'Disconnected'
  private lastError: AppServerIpcManager['status']['lastError'] = null

  constructor(socketPath: string) {
    this.socketPath = socketPath
  }

  get status(): AppServerIpcManager['status'] {
    return {
      state: this.state,
      pid: null,
      pendingRequests: 0,
      queuedMessages: 0,
      reconnectAttempt: null,
      lastError: this.lastError,
    }
  }

  onStatus(listener: (status: AppServerIpcManager['status']) => void): () => void {
    this.listeners.status.add(listener)
    return () => this.listeners.status.delete(listener)
  }

  onNotification(listener: (event: AppServerNotificationEvent) => void): () => void {
    this.listeners.notification.add(listener)
    return () => this.listeners.notification.delete(listener)
  }

  async start(): Promise<AppServerIpcManager['status']> {
    if (this.state !== 'Disconnected') {
      return this.status
    }

    this.state = 'Starting'
    this.emitStatus()

    try {
      const fs = await import('node:fs/promises')
      await fs.access(this.socketPath)
      this.state = 'Ready'
      this.lastError = null
    } catch {
      this.state = 'Disconnected'
      this.lastError = localError(
        'connectionClosed',
        `Daemon socket not found or inaccessible: ${this.socketPath}`,
        { retryable: false },
      )
    }

    this.emitStatus()
    return this.status
  }

  async stop(): Promise<AppServerIpcManager['status']> {
    this.state = 'Disconnected'
    this.emitStatus()
    return this.status
  }

  async request(_method: string, _params?: unknown): Promise<unknown> {
    if (this.state !== 'Ready') {
      throw localError('notReady', `app-server is ${this.state}; business RPC requires Ready state`)
    }
    throw localError('invalidRequest', 'Attached daemon mode does not support direct RPC yet', {
      code: -32600,
    })
  }

  notify(_method: string, _params?: unknown): void {
    if (this.state !== 'Ready') {
      throw localError('notReady', `app-server is ${this.state}; notifications require Ready state`)
    }
    throw localError('invalidRequest', 'Attached daemon mode does not support notifications yet', {
      code: -32600,
    })
  }

  respondToServerRequest(_id: string | number, _result: unknown): Promise<never> {
    return Promise.reject(
      localError('invalidRequest', 'Attached daemon mode does not support server request responses yet', {
        code: -32600,
      }),
    )
  }

  dispose(): void {
    this.state = 'Disconnected'
    this.listeners.status.clear()
    this.listeners.notification.clear()
  }

  private emitStatus(): void {
    const status = this.status
    for (const listener of Array.from(this.listeners.status)) {
      listener(status)
    }
  }
}
