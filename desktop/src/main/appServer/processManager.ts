import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { EventEmitter } from 'node:events'
import type { ClientInfo, InitializeResponse } from '@codex/app-server-protocol'
import type {
  AppServerNotificationEvent,
  AppServerRequestMethod,
  AppServerRpcError,
  AppServerStatus,
} from '../../shared/appServer'
import { JsonRpcClient, type JsonRpcLimits } from './jsonRpcClient'
import { OutboundQueue } from './outboundQueue'
import { localError } from './rpcError'

export type SpawnFn = (
  command: string,
  args: string[],
  options: { stdio: ['pipe', 'pipe', 'pipe']; env?: NodeJS.ProcessEnv },
) => ChildProcessWithoutNullStreams

export interface AppServerManagerOptions {
  command?: string
  args?: string[]
  env?: NodeJS.ProcessEnv
  spawnFn?: SpawnFn
  limits?: Partial<JsonRpcLimits & { maxQueuedMessages: number }>
  reconnect?: { maxAttempts?: number; delayMs?: number; backoffFactor?: number }
  clientInfo?: ClientInfo
  capabilities?: { experimentalApi: boolean }
  logger?: (message: string) => void
  attachProcessExitHooks?: boolean
}

export const DEFAULT_APP_SERVER_COMMAND = 'codex'
export const DEFAULT_APP_SERVER_ARGS = ['app-server', '--listen', 'stdio://']
export const DEFAULT_MAX_QUEUED_MESSAGES = 256
export const DEFAULT_RECONNECT_OPTIONS = {
  maxAttempts: 5,
  delayMs: 100,
  backoffFactor: 2,
}
const KILL_GRACE_MS = 2_000
const MAX_FRAME_BYTES = 8 * 1024 * 1024
const MAX_UNTERMINATED_BUFFER_BYTES = 16 * 1024 * 1024

function isInitializeResponse(value: unknown): value is InitializeResponse {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { userAgent?: unknown }).userAgent === 'string' &&
    typeof (value as { codexHome?: unknown }).codexHome === 'string'
  )
}

function hasExited(child: ChildProcessWithoutNullStreams): boolean {
  return child.exitCode !== null || child.signalCode !== null
}

/**
 * Owns the single app-server child process and its JSON-RPC client.
 *
 * State machine:
 *   Disconnected -> Starting -> Initializing -> Ready
 *   Starting/Initializing/Ready -> Reconnecting -> Starting -> ...
 *   any -> ShuttingDown -> Disconnected
 *
 * Invariants:
 * - only one child process may exist (start is idempotent),
 * - business RPC/notifications are rejected unless state is Ready,
 * - unexpected exit fails every pending request and schedules a bounded reconnect,
 * - main process exit kills the child so no orphan app-server survives.
 */
export class AppServerProcessManager extends EventEmitter {
  private readonly command: string
  private readonly args: string[]
  private readonly env: NodeJS.ProcessEnv | undefined
  private readonly spawnFn: SpawnFn
  private readonly limits: JsonRpcLimits & { maxQueuedMessages: number }
  private readonly reconnectOptions: Required<NonNullable<AppServerManagerOptions['reconnect']>>
  private readonly clientInfo: ClientInfo
  private readonly capabilities: { experimentalApi: boolean }
  private readonly logger: (message: string) => void

  private state: AppServerStatus['state'] = 'Disconnected'
  // Read through this helper so TypeScript does not narrow `state` across
  // awaited operations even though setState mutates it internally.
  private get currentState(): AppServerStatus['state'] {
    return this.state
  }
  private child: ChildProcessWithoutNullStreams | null = null
  private client: JsonRpcClient | null = null
  private outbound: OutboundQueue | null = null
  private lastError: AppServerRpcError | null = null
  private reconnectAttempt = 0
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null
  private forceKillTimer: ReturnType<typeof setTimeout> | null = null
  private pendingForceKillChild: ChildProcessWithoutNullStreams | null = null
  private stopping = false
  private nextId = 0
  private exitHook: (() => void) | null = null
  private stdoutBuffer = ''

  constructor(options: AppServerManagerOptions = {}) {
    super()
    this.command = options.command ?? DEFAULT_APP_SERVER_COMMAND
    this.args = options.args ?? DEFAULT_APP_SERVER_ARGS
    this.env = options.env
    this.spawnFn =
      options.spawnFn ??
      ((command: string, args: string[], spawnOptions: Parameters<SpawnFn>[2]) =>
        spawn(command, args, spawnOptions) as ChildProcessWithoutNullStreams)
    this.limits = {
      maxPendingRequests: options.limits?.maxPendingRequests ?? 128,
      requestTimeoutMs: options.limits?.requestTimeoutMs ?? 30_000,
      maxQueuedMessages: options.limits?.maxQueuedMessages ?? DEFAULT_MAX_QUEUED_MESSAGES,
    }
    this.reconnectOptions = { ...DEFAULT_RECONNECT_OPTIONS, ...options.reconnect }
    this.clientInfo = options.clientInfo ?? {
      name: 'codex-desktop',
      title: 'Codex Desktop',
      version: '0.1.0',
    }
    this.capabilities = options.capabilities ?? { experimentalApi: false }
    this.logger = options.logger ?? (() => undefined)

    if (options.attachProcessExitHooks !== false) {
      this.exitHook = () => {
        // Synchronous last-resort cleanup during process.exit.
        this.killOwnedProcesses()
      }
      process.once('exit', this.exitHook)
    }
  }

  get status(): AppServerStatus {
    return {
      state: this.state,
      pid: this.child?.pid ?? null,
      pendingRequests: this.client?.pendingCount ?? 0,
      queuedMessages: this.outbound?.length ?? 0,
      reconnectAttempt: this.state === 'Reconnecting' ? this.reconnectAttempt : null,
      lastError: this.lastError,
    }
  }

  onStatus(listener: (status: AppServerStatus) => void): () => void {
    this.on('status', listener)
    return () => {
      this.off('status', listener)
    }
  }

  onNotification(listener: (event: AppServerNotificationEvent) => void): () => void {
    this.on('notification', listener)
    return () => {
      this.off('notification', listener)
    }
  }

  async start(): Promise<AppServerStatus> {
    if (this.state !== 'Disconnected') {
      // Idempotent start: a second start never spawns a second process.
      return this.status
    }

    this.stopping = false
    this.clearReconnectTimer()
    this.setState('Starting')

    let child: ChildProcessWithoutNullStreams
    try {
      child = this.spawnFn(this.command, this.args, {
        stdio: ['pipe', 'pipe', 'pipe'],
        env: this.env,
      })
    } catch (error) {
      this.failConnection(localError('spawn', `failed to spawn app-server: ${String(error)}`))
      return this.status
    }

    this.child = child
    this.logger(`app-server spawned with pid ${String(child.pid)}`)
    this.wireChild(child)

    // The process can exit synchronously during startup (ENOENT emits both
    // error and exit asynchronously, but a fast exit only emits exit).
    this.setState('Initializing')

    if (child.pid === undefined) {
      this.failConnection(
        localError('spawn', 'app-server process failed to start: no process id was assigned'),
      )
      return this.status
    }

    this.outbound = new OutboundQueue({
      stream: child.stdin,
      maxQueuedMessages: this.limits.maxQueuedMessages,
      onOverflow: (error) => {
        this.logger('app-server outbound queue overflow; reconnecting')
        void this.beginReconnect(error)
      },
    })

    this.client = new JsonRpcClient({
      send: (frame) => this.outbound?.push(frame),
      limits: this.limits,
      nextRequestId: () => ++this.nextId,
      onNotification: (method, params) => {
        if (this.state === 'Ready') {
          this.emit('notification', { method, params } satisfies AppServerNotificationEvent)
        }
      },
      onProtocolError: (error) => {
        void this.beginReconnect(error)
      },
    })

    try {
      const result = await this.client.request('initialize', {
        clientInfo: this.clientInfo,
        capabilities: this.capabilities,
      })
      if (!isInitializeResponse(result)) {
        throw localError('invalidRequest', 'app-server initialize response was invalid')
      }
      this.client.notify('initialized')
      this.reconnectAttempt = 0
      this.setState('Ready')
    } catch (error) {
      const rpcError = error as AppServerRpcError
      if (this.currentState === 'Initializing') {
        // Handshake failure ends in deterministic Disconnected state; no
        // user configuration or approval side effects are performed.
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
      return await this.requestInternal(method, params)
    } catch (error) {
      const rpcError = error as AppServerRpcError
      if (rpcError?.kind === 'serverShuttingDown' && this.state === 'Ready') {
        // The server is draining; move to Reconnecting instead of reporting
        // a generic invalid request.
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
    this.requestNotify(method, params)
  }

  async stop(): Promise<AppServerStatus> {
    if (this.state === 'Disconnected') {
      return this.status
    }

    this.setState('ShuttingDown')
    this.stopping = true
    this.clearReconnectTimer()
    this.failPending(
      localError('connectionClosed', 'app-server is shutting down', { retryable: false }),
    )

    const child = this.child
    if (!child || hasExited(child)) {
      this.teardownChild()
      this.setState('Disconnected')
      return this.status
    }

    await new Promise<void>((resolve) => {
      const onExit = (): void => {
        child.removeListener('exit', onExit)
        this.clearForceKillTimer()
        resolve()
      }
      child.once('exit', onExit)
      this.forceKill('SIGTERM')
    })

    return this.status
  }

  /** Synchronous teardown for process/app exit. Safe to call multiple times. */
  dispose(): void {
    this.removeProcessExitHooks()
    this.clearReconnectTimer()

    if (this.state !== 'Disconnected') {
      this.stopping = true
      this.setState('ShuttingDown')
    }
    this.clearForceKillTimer()
    this.killOwnedProcesses()
  }

  private killOwnedProcesses(): void {
    const pendingForceKillChild = this.pendingForceKillChild
    this.pendingForceKillChild = null
    if (pendingForceKillChild && !hasExited(pendingForceKillChild)) {
      try {
        pendingForceKillChild.kill('SIGKILL')
      } catch (error) {
        this.logger(`failed to force-kill pending app-server: ${String(error)}`)
      }
    }
    this.killChild('SIGTERM')
    this.killChild('SIGKILL')
  }

  removeProcessExitHooks(): void {
    if (this.exitHook) {
      process.removeListener('exit', this.exitHook)
      this.exitHook = null
    }
  }

  private handleStreamClosed(
    child: ChildProcessWithoutNullStreams,
    streamName: 'stdin' | 'stdout',
  ): void {
    if (this.child !== child || this.stopping || this.state === 'ShuttingDown') {
      return
    }
    void this.beginReconnect(
      localError('connectionClosed', `app-server ${streamName} stream closed`, {
        retryable: true,
      }),
    )
  }

  private requestInternal(method: string, params?: unknown): Promise<unknown> {
    const client = this.client
    if (!client) {
      return Promise.reject(localError('connectionClosed', 'app-server connection is closed'))
    }
    return client.request(method, params)
  }

  private requestNotify(method: string, params?: unknown): void {
    this.client?.notify(method, params)
  }

  private wireChild(child: ChildProcessWithoutNullStreams): void {
    child.stdout.setEncoding('utf8')
    child.stdout.on('data', (chunk: string) => {
      this.feedStdout(chunk)
    })
    child.stdout.on('error', (error: Error) => {
      this.logger(`app-server stdout error: ${error.message}`)
    })
    child.stdout.once('close', () => {
      this.handleStreamClosed(child, 'stdout')
    })
    child.stdout.once('end', () => {
      this.handleStreamClosed(child, 'stdout')
    })
    child.stderr.setEncoding('utf8')
    child.stderr.on('data', (chunk: string) => {
      this.logger(`app-server stderr: ${chunk.trimEnd()}`)
    })
    child.stderr.on('error', (error: Error) => {
      this.logger(`app-server stderr error: ${error.message}`)
    })
    child.once('error', (error: Error) => {
      if (this.child !== child) {
        return
      }
      this.failConnection(localError('spawn', `app-server process error: ${error.message}`))
    })
    child.once('exit', (code, signal) => {
      this.handleChildExit(child, code, signal)
    })
    child.stdin.once('error', (error: Error) => {
      this.logger(`app-server stdin error: ${error.message}`)
    })
    child.stdin.once('close', () => {
      this.handleStreamClosed(child, 'stdin')
    })
    child.stdin.once('end', () => {
      this.handleStreamClosed(child, 'stdin')
    })
  }

  private feedStdout(chunk: string): void {
    this.stdoutBuffer += chunk
    let newlineIndex = this.stdoutBuffer.indexOf('\n')
    while (newlineIndex !== -1) {
      const line = this.stdoutBuffer.slice(0, newlineIndex)
      this.stdoutBuffer = this.stdoutBuffer.slice(newlineIndex + 1)
      const byteLength = Buffer.byteLength(line, 'utf8')
      if (byteLength > MAX_FRAME_BYTES) {
        this.logger('app-server sent an oversized frame; closing connection')
        void this.beginReconnect(
          localError('parse', `app-server frame exceeded 8 MiB (${byteLength} bytes)`, {
            code: -32700,
          }),
        )
        return
      }
      this.client?.feedLine(line)
      newlineIndex = this.stdoutBuffer.indexOf('\n')
    }
    if (Buffer.byteLength(this.stdoutBuffer, 'utf8') > MAX_UNTERMINATED_BUFFER_BYTES) {
      void this.beginReconnect(
        localError('parse', 'app-server buffered more than 16 MiB without a newline', {
          code: -32700,
        }),
      )
    }
  }

  private handleChildExit(
    child: ChildProcessWithoutNullStreams,
    code: number | null,
    signal: NodeJS.Signals | null,
  ): void {
    if (this.child !== child) {
      return
    }

    const exitError = localError(
      'connectionClosed',
      `app-server exited unexpectedly (code=${String(code)}, signal=${String(signal)})`,
      { retryable: true },
    )

    if (this.stopping || this.state === 'ShuttingDown') {
      this.failPending(exitError)
      this.teardownChild()
      this.setState('Disconnected')
      return
    }

    void this.beginReconnect(exitError)
  }

  private async beginReconnect(error: AppServerRpcError): Promise<void> {
    if (this.state === 'ShuttingDown' || this.stopping || this.state === 'Disconnected') {
      return
    }

    // Drop the broken connection and its pending requests first.
    this.failPending(error)
    this.killChild('SIGTERM')
    this.scheduleForceKill()
    this.child = null
    this.client = null
    this.outbound = null

    this.reconnectAttempt += 1
    if (this.reconnectAttempt > this.reconnectOptions.maxAttempts) {
      const exhausted = localError(
        'connectionClosed',
        `app-server reconnect failed after ${this.reconnectOptions.maxAttempts} attempts: ${error.message}`,
      )
      this.reconnectAttempt = 0
      this.setState('Disconnected')
      this.lastError = exhausted
      this.emit('status', this.status)
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
      this.setState('Disconnected')
      void this.start()
    }, delay)
  }

  private failConnection(error: AppServerRpcError): void {
    this.failPending(error)

    const child = this.child
    if (child && !hasExited(child)) {
      try {
        child.kill('SIGTERM')
      } catch (killError) {
        this.logger(`failed to terminate app-server: ${String(killError)}`)
      }
    }

    this.teardownChild()

    if (child && !hasExited(child)) {
      this.pendingForceKillChild = child
      this.clearForceKillTimer()
      this.forceKillTimer = setTimeout(() => {
        this.forceKillTimer = null
        this.pendingForceKillChild = null
        if (!hasExited(child)) {
          try {
            child.kill('SIGKILL')
          } catch (forceKillError) {
            this.logger(`failed to force-kill app-server: ${String(forceKillError)}`)
          }
        }
      }, KILL_GRACE_MS)
      this.forceKillTimer.unref?.()
    }

    this.setState('Disconnected')
  }

  private failPending(error: AppServerRpcError): void {
    this.lastError = error
    this.client?.end(error)
    this.outbound?.close()
    this.outbound = null
    if (this.child) {
      this.child.stdout.removeAllListeners('data')
      this.child.stderr.removeAllListeners('data')
    }
    this.stdoutBuffer = ''
  }

  private teardownChild(): void {
    this.clearForceKillTimer()
    this.child = null
    this.client = null
    this.outbound = null
    this.stdoutBuffer = ''
  }

  private forceKill(signal: NodeJS.Signals): void {
    this.killChild(signal)
    this.scheduleForceKill()
  }

  private scheduleForceKill(): void {
    const child = this.child
    if (!child || hasExited(child)) {
      return
    }
    this.clearForceKillTimer()
    this.forceKillTimer = setTimeout(() => {
      this.forceKillTimer = null
      if (!hasExited(child)) {
        try {
          child.kill('SIGKILL')
        } catch (error) {
          this.logger(`failed to force-kill app-server: ${String(error)}`)
        }
      }
    }, KILL_GRACE_MS)
    this.forceKillTimer.unref?.()
  }

  private killChild(signal: NodeJS.Signals): void {
    const child = this.child
    if (child && !hasExited(child)) {
      try {
        child.kill(signal)
      } catch (error) {
        this.logger(`failed to kill app-server with ${signal}: ${String(error)}`)
      }
    }
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
  }

  private clearForceKillTimer(): void {
    if (this.forceKillTimer) {
      clearTimeout(this.forceKillTimer)
      this.forceKillTimer = null
    }
  }

  private setState(state: AppServerStatus['state']): void {
    this.state = state
    if (state === 'Disconnected') {
      this.stopping = false
    }
    this.emit('status', this.status)
  }
}
