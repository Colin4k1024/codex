import type { AppServerRpcError } from '../../shared/appServer'
import { localError } from './rpcError'

/**
 * Structural subset of a Writable stream used for newline-delimited frames.
 * A structural type keeps the queue unit-testable with a minimal fake stream.
 */
export interface OutboundStream {
  write(frame: string): boolean
  on(event: 'drain', listener: () => void): unknown
  on(event: 'error', listener: (error: Error) => void): unknown
}

export interface OutboundQueueOptions {
  stream: OutboundStream
  maxQueuedMessages: number
  onOverflow: (error: AppServerRpcError) => void
  onWriteError?: (error: Error) => void
}

/**
 * FIFO frame writer with a hard capacity bound.
 *
 * All outbound JSON-RPC messages (requests, notifications, initialize handshake)
 * go through this queue, which preserves wire order for a single connection and
 * respects stream backpressure instead of buffering unbounded.
 */
export class OutboundQueue {
  private readonly stream: OutboundStream
  private readonly maxQueuedMessages: number
  private readonly onOverflow: (error: AppServerRpcError) => void
  private readonly onWriteError?: (error: Error) => void
  private queue: string[] = []
  private draining = false
  private waitingForDrain = false
  private closed = false
  private outstanding = 0

  constructor(options: OutboundQueueOptions) {
    this.stream = options.stream
    this.maxQueuedMessages = options.maxQueuedMessages
    this.onOverflow = options.onOverflow
    this.onWriteError = options.onWriteError
    this.stream.on('drain', () => {
      this.waitingForDrain = false
      void this.drain()
    })
    this.stream.on('error', (error: Error) => {
      this.onWriteError?.(error)
    })
  }

  /** Number of frames accepted but not yet handed to the stream. */
  get length(): number {
    return this.outstanding
  }

  push(frame: string): void {
    if (this.closed) {
      return
    }
    if (this.outstanding >= this.maxQueuedMessages) {
      this.closed = true
      this.queue = []
      this.outstanding = 0
      this.onOverflow(
        localError(
          'capacity',
          `app-server outbound queue limit reached (${this.maxQueuedMessages})`,
        ),
      )
      return
    }
    this.outstanding += 1
    this.queue.push(frame)
    if (!this.draining && !this.waitingForDrain) {
      this.drain()
    }
  }

  close(): void {
    this.closed = true
    this.queue = []
    this.outstanding = 0
  }

  private drain(): void {
    if (this.draining || this.waitingForDrain || this.closed) {
      return
    }
    this.draining = true
    try {
      while (!this.closed && this.queue.length > 0 && !this.waitingForDrain) {
        const frame = this.queue[0]
        const canContinue = this.stream.write(frame)
        this.queue.shift()
        this.outstanding -= 1
        if (!canContinue) {
          this.waitingForDrain = true
          return
        }
      }
    } finally {
      this.draining = false
    }
  }
}
