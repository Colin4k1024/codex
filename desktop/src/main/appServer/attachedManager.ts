import { stat } from 'node:fs/promises'
import { constants as fsConstants } from 'node:fs'
import type { ClientInfo } from '@codex/app-server-protocol'
import WebSocket from 'ws'
import type {
  AppServerNotificationEvent,
  AppServerRequestMethod,
  AppServerRpcError,
  AppServerStatus,
} from '../../shared/appServer'
import type { AppServerIpcManager } from './ipc'
import { JsonRpcClient, type JsonRpcLimits } from './jsonRpcClient'
import { localError } from './rpcError'

export interface AttachedAppServerManagerOptions {
  socketPath: string
  clientInfo?: ClientInfo
  capabilities?: { experimentalApi: boolean }
  limits?: Partial<JsonRpcLimits>
  reconnect?: { maxAttempts?: number; delayMs?: number; backoffFactor?: number }
  logger?: (message: string) => void
}

export const DEFAULT_ATTACHED_RECONNECT_OPTIONS = {
  maxAttempts: 5,
  delayMs: 100,
  backoffFactor: 2,
}

interface InitializeResponseShape {
  userAgent: string
  codexHome: string
  platformFamily: string
  platformOs: string
}

function isInitializeResponse(value: unknown): value is InitializeResponseShape {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { userAgent?: unknown }).userAgent === 'string' &&
    typeof (value as { codexHome?: unknown }).codexHome === 'string' &&
    typeof (value as { platformFamily?: unknown }).platformFamily === 'string' &&
    typeof (value as { platformOs?: unknown }).platformOs === 'string'
  )
}

function majorVersion(version: string): number | null {
  const match = /^(\d+)/.exec(version)
  return match === null ? null : Number(match[1])
}

function parseAppServerVersion(userAgent: string): string | null {
  const separator = userAgent.indexOf('/')
  if (separator < 0) {
    return null
  }
  const version = userAgent.slice(separator + 1).split(/\s+/)[0]
  return version.length > 0 ? version : null
}

function socketError(error: unknown, socketPath: string): AppServerRpcError {
  const code =
    typeof (error as NodeJS.ErrnoException)?.code === 'string'
      ? (error as NodeJS.ErrnoException).code
      : undefined
  const reason =
    code === 'ENOENT'
      ? 'Daemon socket does not exist'
      : code === 'EACCES' || code === 'EPERM'
        ? 'Daemon socket is not accessible'
        : code === 'ECONNREFUSED'
          ? 'Daemon socket refused the connection'
          : 'Failed to attach to the daemon socket'
  return localError('connectionClosed', `${reason}: ${socketPath}`, { retryable: false })
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function isAppServerRpcError(value: unknown): value is AppServerRpcError {
  return (
    isRecord(value) &&
    typeof value.message === 'string' &&
    typeof value.kind === 'string' &&
    typeof value.retryable === 'boolean'
  )
}

/**
 * Attaches to an existing app-server daemon over its local Unix control socket.
 *
 * The manager never spawns or owns an app-server process. It performs the same
 * JSON-RPC initialize handshake as the owned-process manager, but each frame is
 * a WebSocket text frame over the Unix socket. Unexpected closes enter a bounded
 * reconnect loop; every successful reconnect returns to Ready so consumers can
 * reconcile from durable app-server state instead of trusting stale events.
 */
export class AttachedAppServerManager implements AppServerIpcManager {
  private readonly socketPath: string
  private readonly clientInfo: ClientInfo
  private readonly capabilities: { experimentalApi: boolean }
  private readonly limits: JsonRpcLimits
  private readonly reconnectOptions: Required<
    NonNullable<AttachedAppServerManagerOptions['reconnect']>
  >
  private readonly logger: (message: string) => void

  private readonly listeners: {
    status: Set<(status: AppServerStatus) => void>
    notification: Set<(event: AppServerNotificationEvent) => void>
  } = {
    status: new Set(),
    notification: new Set(),
  }

  private state: AppServerStatus['state'] = 'Disconnected'
  // Read through this helper so TypeScript does not narrow `state` across awaits.
  private get currentState(): AppServerStatus['state'] {
    return this.state
  }
  private socket: WebSocket | null = null
  private client: JsonRpcClient | null = null
  private lastError: AppServerRpcError | null = null
  private reconnectAttempt = 0
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null
  private stopping = false
  private nextId = 0
  private serverVersion: string | null = null

  constructor(options: AttachedAppServerManagerOptions) {
    this.socketPath = options.socketPath
    this.clientInfo = options.clientInfo ?? {
      name: 'codex-desktop',
      title: 'Codex Desktop',
      version: '0.1.0',
    }
    this.capabilities = options.capabilities ?? { experimentalApi: false }
    this.limits = {
      maxPendingRequests: options.limits?.maxPendingRequests ?? 128,
      requestTimeoutMs: options.limits?.requestTimeoutMs ?? 30_000,
    }
    this.reconnectOptions = {
      ...DEFAULT_ATTACHED_RECONNECT_OPTIONS,
      ...options.reconnect,
    }
    this.logger = options.logger ?? (() => undefined)
  }

  get status(): AppServerStatus {
    return {
      state: this.state,
      pid: null,
      pendingRequests: this.client?.pendingCount ?? 0,
      queuedMessages: 0,
      reconnectAttempt: this.state === 'Reconnecting' ? this.reconnectAttempt : null,
      lastError: this.lastError,
    }
  }

  /** Version reported by the attached app-server, or null before initialize. */
  get appServerVersion(): string | null {
    return this.serverVersion
  }

  onStatus(listener: (status: AppServerStatus) => void): () => void {
    this.listeners.status.add(listener)
    return () => {
      this.listeners.status.delete(listener)
    }
  }

  onNotification(listener: (event: AppServerNotificationEvent) => void): () => void {
    this.listeners.notification.add(listener)
    return () => {
      this.listeners.notification.delete(listener)
    }
  }

  async start(): Promise<AppServerStatus> {
    if (this.state !== 'Disconnected') {
      return this.status
    }

    this.stopping = false
    this.clearReconnectTimer()
    this.setState('Starting')

    try {
      const socketStats = await stat(this.socketPath)
      if ((socketStats.mode & fsConstants.S_IFMT) !== fsConstants.S_IFSOCK) {
        throw localError('invalidRequest', `Daemon path is not a Unix socket: ${this.socketPath}`, {
          code: -32600,
        })
      }
    } catch (error) {
      this.failConnection(isAppServerRpcError(error) ? error : socketError(error, this.socketPath))
      return this.status
    }

    let socket: WebSocket
    try {
      socket = new WebSocket(`ws+unix://${this.socketPath}`, {
        handshakeTimeout: Math.min(this.limits.requestTimeoutMs, 2_000),
        maxPayload: 8 * 1024 * 1024,
        perMessageDeflate: false,
      })
      await new Promise<void>((resolve, reject) => {
        const onOpen = (): void => {
          socket.off('error', onError)
          socket.off('close', onClose)
          resolve()
        }
        const onError = (error: Error): void => {
          socket.off('open', onOpen)
          socket.off('close', onClose)
          reject(error)
        }
        const onClose = (): void => {
          socket.off('open', onOpen)
          socket.off('error', onError)
          reject(new Error('Daemon socket closed during WebSocket upgrade'))
        }
        socket.once('open', onOpen)
        socket.once('error', onError)
        socket.once('close', onClose)
      })
    } catch (error) {
      this.failConnection(socketError(error, this.socketPath))
      return this.status
    }

    this.socket = socket
    this.setState('Initializing')

    this.client = new JsonRpcClient({
      send: (frame) => socket.send(frame),
      limits: this.limits,
      nextRequestId: () => ++this.nextId,
      onNotification: (method, params) => {
        if (method === 'serverRequest/resolved') {
          const requestId = isRecord(params) ? params.requestId : undefined
          if (typeof requestId === 'string' || typeof requestId === 'number') {
            this.client?.forgetServerRequest(requestId)
          }
        }
        if (this.state === 'Ready') {
          this.emitNotification({ method, params })
        }
      },
      onServerRequest: ({ id, method, params }) => {
        if (this.state === 'Ready') {
          this.emitNotification({ id, method, params })
        }
      },
      onProtocolError: (error) => {
        if (this.state === 'Ready') {
          void this.beginReconnect(error)
        } else if (this.state === 'Initializing') {
          this.failConnection(error)
        }
      },
    })

    socket.on('message', (data) => {
      this.client?.feedLine(data.toString())
    })
    socket.on('close', () => {
      if (this.socket === socket) {
        this.handleSocketClosed()
      }
    })

    try {
      const response = await this.client.request('initialize', {
        clientInfo: this.clientInfo,
        capabilities: this.capabilities,
      })
      if (!isInitializeResponse(response)) {
        throw localError('invalidRequest', 'app-server initialize response was invalid', {
          code: -32600,
        })
      }

      const version = parseAppServerVersion(response.userAgent)
      if (version === null) {
        throw localError('invalidRequest', 'app-server user agent omitted a valid version', {
          code: -32600,
        })
      }
      const expectedMajor = majorVersion(this.clientInfo.version)
      const actualMajor = majorVersion(version)
      if (expectedMajor === null || actualMajor === null || expectedMajor !== actualMajor) {
        throw localError(
          'invalidRequest',
          `app-server major version ${version} is incompatible with desktop major version ${this.clientInfo.version}`,
          { code: -32600 },
        )
      }

      this.serverVersion = version
      this.client.notify('initialized')
      this.logger(`attached to app-server daemon ${version} at ${this.socketPath}`)
      this.reconnectAttempt = 0
      this.setState('Ready')
    } catch (error) {
      const rpcError = error as AppServerRpcError
      if (this.currentState === 'Initializing') {
        this.failConnection(rpcError)
      }
    }

    return this.status
  }

  async request(method: AppServerRequestMethod, params?: unknown): Promise<unknown> {
    if (this.state !== 'Ready') {
      return Promise.reject(
        localError('notReady', `app-server is ${this.state}; business RPC requires Ready state`),
      )
    }
    if (method === 'initialize') {
      return Promise.reject(
        localError('invalidRequest', 'initialize is reserved for the connection lifecycle', {
          code: -32600,
        }),
      )
    }

    try {
      return await this.client?.request(method, params)
    } catch (error) {
      const rpcError = error as AppServerRpcError
      if (rpcError?.kind === 'serverShuttingDown' && this.state === 'Ready') {
        void this.beginReconnect(rpcError)
      }
      throw rpcError
    }
  }

  notify(method: string, params?: unknown): void {
    if (this.state !== 'Ready') {
      throw localError('notReady', `app-server is ${this.state}; notifications require Ready state`)
    }
    if (method === 'initialize' || method === 'initialized') {
      throw localError('invalidRequest', `${method} is reserved for the connection lifecycle`, {
        code: -32600,
      })
    }
    this.client?.notify(method, params)
  }

  respondToServerRequest(id: string | number, result: unknown): Promise<void> {
    if (this.state !== 'Ready') {
      return Promise.reject(
        localError(
          'notReady',
          `app-server is ${this.state}; server request responses require Ready state`,
        ),
      )
    }
    try {
      this.client?.respondToServerRequest(id, result)
      return Promise.resolve()
    } catch (error) {
      return Promise.reject(error)
    }
  }

  async stop(): Promise<AppServerStatus> {
    if (this.state === 'Disconnected') {
      return this.status
    }

    this.setState('ShuttingDown')
    this.stopping = true
    this.clearReconnectTimer()

    const socket = this.socket
    this.failPending(
      localError('connectionClosed', 'app-server is shutting down', { retryable: false }),
    )

    if (socket) {
      await new Promise<void>((resolve) => {
        socket.once('close', resolve)
        socket.close(1000)
      })
    }

    this.setState('Disconnected')
    return this.status
  }

  /** Synchronous teardown for app exit. Safe to call multiple times. */
  dispose(): void {
    this.clearReconnectTimer()
    if (this.state !== 'Disconnected') {
      this.stopping = true
      this.state = 'ShuttingDown'
    }
    this.failPending(
      localError('connectionClosed', 'attached app-server connection was disposed', {
        retryable: false,
      }),
    )
    this.setState('Disconnected')
    this.listeners.status.clear()
    this.listeners.notification.clear()
  }

  private handleSocketClosed(): void {
    if (this.stopping || this.state === 'ShuttingDown' || this.state === 'Disconnected') {
      return
    }

    void this.beginReconnect(
      localError('connectionClosed', 'attached app-server daemon connection closed', {
        retryable: true,
      }),
    )
  }

  private async runReconnectAttempt(): Promise<void> {
    this.state = 'Disconnected'
    this.emitStatus()
    const status = await this.start()
    if (status.state === 'Disconnected' && !this.stopping) {
      await this.beginReconnect(
        status.lastError ??
          localError('connectionClosed', 'daemon reattach attempt failed', { retryable: true }),
      )
    }
  }

  private async beginReconnect(error: AppServerRpcError): Promise<void> {
    if (this.state === 'ShuttingDown' || this.stopping) {
      return
    }

    this.failPending(error)
    this.reconnectAttempt += 1
    if (this.reconnectAttempt > this.reconnectOptions.maxAttempts) {
      const exhausted = localError(
        'connectionClosed',
        `daemon reattach failed after ${this.reconnectOptions.maxAttempts} attempts: ${error.message}`,
      )
      this.reconnectAttempt = 0
      this.state = 'Disconnected'
      this.lastError = exhausted
      this.emitStatus()
      return
    }

    this.setState('Reconnecting')
    const delay =
      this.reconnectOptions.delayMs *
      this.reconnectOptions.backoffFactor ** (this.reconnectAttempt - 1)
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      if (this.state !== 'Reconnecting') {
        return
      }
      void this.runReconnectAttempt()
    }, delay)
  }

  private failConnection(error: AppServerRpcError): void {
    this.failPending(error)
    this.setState('Disconnected')
  }

  private failPending(error: AppServerRpcError): void {
    this.lastError = error
    this.client?.end(error)
    this.client = null

    const socket = this.socket
    this.socket = null
    if (socket) {
      socket.removeAllListeners()
      try {
        socket.terminate()
      } catch (error) {
        this.logger(`failed to close attached daemon socket: ${String(error)}`)
      }
    }
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
  }

  private setState(state: AppServerStatus['state']): void {
    this.state = state
    if (state === 'Disconnected') {
      this.stopping = false
    }
    this.emitStatus()
  }

  private emitStatus(): void {
    const status = this.status
    for (const listener of Array.from(this.listeners.status)) {
      listener(status)
    }
  }

  private emitNotification(event: AppServerNotificationEvent): void {
    for (const listener of Array.from(this.listeners.notification)) {
      listener(event)
    }
  }
}
