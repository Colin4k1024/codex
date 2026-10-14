import type { AppServerRpcError } from '../../shared/appServer'
import { classifyRpcError, JSON_RPC_ERROR_CODES, localError } from './rpcError'

export interface JsonRpcLimits {
  maxPendingRequests: number
  requestTimeoutMs: number
}

export const DEFAULT_JSON_RPC_LIMITS: JsonRpcLimits = {
  maxPendingRequests: 128,
  requestTimeoutMs: 30_000,
}

export type RequestIdFactory = () => number

export interface JsonRpcClientOptions {
  send: (frame: string) => void
  limits?: Partial<JsonRpcLimits>
  nextRequestId?: RequestIdFactory
  onNotification?: (method: string, params: unknown) => void
  onServerRequest?: (request: { id: string | number; method: string; params?: unknown }) => void
  onProtocolError?: (error: AppServerRpcError) => void
  onRequestIds?: (ids: ReadonlySet<number>) => void
  setTimeoutFn?: typeof setTimeout
  clearTimeoutFn?: typeof clearTimeout
}

interface PendingRequest {
  resolve: (result: unknown) => void
  reject: (error: AppServerRpcError) => void
  timer: ReturnType<typeof setTimeout>
}

function isResponse(value: unknown): value is { id: unknown; result?: unknown; error?: unknown } {
  return typeof value === 'object' && value !== null && 'id' in value && !('method' in value)
}

/**
 * Newline-delimited JSON-RPC client half of one app-server connection.
 *
 * One instance owns exactly one connection. It guarantees:
 * - request ids come from the injected factory and are never reused internally,
 * - pending entries are removed when a response, timeout, or disconnect resolves them,
 * - request count is bounded before a frame is queued,
 * - notification callbacks are invoked in wire order.
 */
export class JsonRpcClient {
  private readonly send: (frame: string) => void
  private readonly limits: JsonRpcLimits
  private readonly nextRequestId: RequestIdFactory
  private readonly onNotification: (method: string, params: unknown) => void
  private readonly onServerRequest: (request: {
    id: string | number
    method: string
    params?: unknown
  }) => void
  private readonly onProtocolError: (error: AppServerRpcError) => void
  private readonly setTimeoutFn: typeof setTimeout
  private readonly clearTimeoutFn: typeof clearTimeout
  private pending = new Map<number, PendingRequest>()
  private activeServerRequests = new Set<string | number>()
  private ended = false

  constructor(options: JsonRpcClientOptions) {
    this.send = options.send
    this.limits = { ...DEFAULT_JSON_RPC_LIMITS, ...options.limits }
    let defaultId = 0
    this.nextRequestId = options.nextRequestId ?? (() => ++defaultId)
    this.onNotification = options.onNotification ?? (() => undefined)
    this.onServerRequest = options.onServerRequest ?? (() => undefined)
    this.onProtocolError = options.onProtocolError ?? (() => undefined)
    this.setTimeoutFn = options.setTimeoutFn ?? setTimeout
    this.clearTimeoutFn = options.clearTimeoutFn ?? clearTimeout
  }

  get pendingCount(): number {
    return this.pending.size
  }

  get isEnded(): boolean {
    return this.ended
  }

  request(method: string, params?: unknown): Promise<unknown> {
    if (this.ended) {
      return Promise.reject(localError('connectionClosed', 'app-server connection is closed'))
    }
    if (typeof method !== 'string' || method.length === 0) {
      return Promise.reject(
        localError('invalidRequest', 'JSON-RPC method must be a non-empty string', {
          code: JSON_RPC_ERROR_CODES.invalidRequest,
        }),
      )
    }
    if (this.pending.size >= this.limits.maxPendingRequests) {
      return Promise.reject(
        localError(
          'capacity',
          `app-server pending request limit reached (${this.limits.maxPendingRequests})`,
        ),
      )
    }

    const id = this.nextRequestId()
    if (this.pending.has(id)) {
      return Promise.reject(localError('internal', `duplicate JSON-RPC request id ${id}`))
    }

    return new Promise<unknown>((resolve, reject) => {
      const timer = this.setTimeoutFn(() => {
        if (!this.pending.has(id)) {
          return
        }
        this.pending.delete(id)
        reject(
          localError(
            'timeout',
            `app-server request timed out after ${this.limits.requestTimeoutMs}ms`,
            { retryable: true },
          ),
        )
      }, this.limits.requestTimeoutMs)

      this.pending.set(id, { resolve, reject, timer })
      this.sendFrame({ id, method, params })
    })
  }

  notify(method: string, params?: unknown): void {
    if (this.ended) {
      throw localError('connectionClosed', 'app-server connection is closed')
    }
    if (typeof method !== 'string' || method.length === 0) {
      throw localError('invalidRequest', 'JSON-RPC notification method must be non-empty', {
        code: JSON_RPC_ERROR_CODES.invalidRequest,
      })
    }
    this.sendFrame({ method, params })
  }

  /** Feeds one complete newline-terminated frame (without the newline). */
  feedLine(line: string): void {
    if (this.ended) {
      return
    }

    let message: unknown
    try {
      message = JSON.parse(line)
    } catch {
      this.onProtocolError(
        localError('parse', 'app-server sent malformed JSON', {
          code: JSON_RPC_ERROR_CODES.parse,
        }),
      )
      return
    }

    this.handleMessage(message)
  }

  private handleMessage(message: unknown): void {
    if (typeof message !== 'object' || message === null) {
      this.onProtocolError(
        localError('parse', 'app-server sent a non-object JSON-RPC message', {
          code: JSON_RPC_ERROR_CODES.parse,
        }),
      )
      return
    }

    const record = message as Record<string, unknown>

    if ('method' in record) {
      if (typeof record.method !== 'string' || record.method.length === 0) {
        this.onProtocolError(
          localError('parse', 'app-server notification has an invalid method', {
            code: JSON_RPC_ERROR_CODES.parse,
          }),
        )
        return
      }

      if ('id' in record) {
        const id = record.id
        if (typeof id !== 'string' && (typeof id !== 'number' || !Number.isFinite(id))) {
          this.onProtocolError(
            localError('parse', `app-server request has invalid id ${JSON.stringify(id)}`, {
              code: JSON_RPC_ERROR_CODES.parse,
            }),
          )
          return
        }
        if (this.activeServerRequests.has(id)) {
          this.onProtocolError(
            localError('parse', `app-server repeated active request id ${JSON.stringify(id)}`, {
              code: JSON_RPC_ERROR_CODES.parse,
            }),
          )
          return
        }
        this.activeServerRequests.add(id)
        this.onServerRequest({
          id,
          method: record.method,
          params: 'params' in record ? record.params : undefined,
        })
        return
      }

      this.onNotification(record.method, 'params' in record ? record.params : undefined)
      return
    }

    if (!isResponse(record)) {
      this.onProtocolError(
        localError('parse', 'app-server message is neither a notification nor a response', {
          code: JSON_RPC_ERROR_CODES.parse,
        }),
      )
      return
    }

    const id = record.id
    if (typeof id !== 'number' || !Number.isFinite(id)) {
      this.onProtocolError(
        localError('parse', `app-server response has invalid id ${JSON.stringify(id)}`, {
          code: JSON_RPC_ERROR_CODES.parse,
        }),
      )
      return
    }

    const pending = this.pending.get(id)
    if (!pending) {
      // Late or duplicate response for an already-resolved request is dropped,
      // never mapped onto a different request.
      return
    }

    this.pending.delete(id)
    this.clearTimeoutFn(pending.timer)

    if ('error' in record && record.error !== undefined) {
      pending.reject(classifyRpcError(record.error as Record<string, unknown>))
      return
    }

    pending.resolve('result' in record ? record.result : undefined)
  }

  respondToServerRequest(id: string | number, result: unknown): void {
    if (this.ended) {
      throw localError('connectionClosed', 'app-server connection is closed')
    }
    if (!this.activeServerRequests.has(id)) {
      throw localError(
        'invalidRequest',
        `app-server request id ${JSON.stringify(id)} is not active`,
        { code: JSON_RPC_ERROR_CODES.invalidRequest },
      )
    }
    this.activeServerRequests.delete(id)
    this.sendFrame({ id, result })
  }

  forgetServerRequest(id: string | number): void {
    this.activeServerRequests.delete(id)
  }

  /**
   * Called when stdin/stdout closes or the process exits. Every pending request
   * must fail immediately; late responses are ignored afterwards.
   */
  end(error?: AppServerRpcError): void {
    if (this.ended) {
      return
    }
    this.ended = true
    const failure =
      error ??
      localError('connectionClosed', 'app-server connection closed before the response arrived')
    for (const pending of this.pending.values()) {
      this.clearTimeoutFn(pending.timer)
      pending.reject(failure)
    }
    this.pending.clear()
    this.activeServerRequests.clear()
  }

  private sendFrame(frame: Record<string, unknown>): void {
    this.send(`${JSON.stringify(frame)}\n`)
  }
}
