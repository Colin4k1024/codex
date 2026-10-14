import type { ClientNotification, ClientRequest } from '@codex/app-server-protocol'

export type AppServerConnectionState =
  | 'Disconnected'
  | 'Starting'
  | 'Initializing'
  | 'Ready'
  | 'Reconnecting'
  | 'ShuttingDown'

export type AppServerErrorKind =
  | 'parse'
  | 'invalidRequest'
  | 'serverShuttingDown'
  | 'methodNotFound'
  | 'invalidParams'
  | 'overloaded'
  | 'internal'
  | 'timeout'
  | 'connectionClosed'
  | 'notReady'
  | 'capacity'
  | 'spawn'
  | 'unknown'

/**
 * Serialized JSON-RPC failure shape shared by the main process, preload, and renderer.
 */
export interface AppServerRpcError {
  /** JSON-RPC error code, or null for client-local failures (timeout, spawn, capacity, ...). */
  code: number | null
  message: string
  kind: AppServerErrorKind
  /** Raw JSON-RPC error data when the server supplied it. */
  data?: unknown
  /** True only for errors where an immediate retry on the same connection is meaningful. */
  retryable: boolean
}

export interface AppServerStatus {
  state: AppServerConnectionState
  pid: number | null
  pendingRequests: number
  queuedMessages: number
  /** Reconnect attempt counter after an unexpected exit, or null outside Reconnecting. */
  reconnectAttempt: number | null
  lastError: AppServerRpcError | null
}

export interface AppServerNotificationEvent {
  method: AppServerNotificationMethod
  params?: unknown
}

export type AppServerRequestMethod = ClientRequest['method']
export type AppServerNotificationMethod =
  | ClientNotification['method']
  | (string & Record<never, never>)

export const APP_SERVER_CHANNELS = {
  start: 'appServer:start',
  stop: 'appServer:stop',
  status: 'appServer:status',
  request: 'appServer:request',
  notify: 'appServer:notify',
  statusChanged: 'appServer:statusChanged',
  notification: 'appServer:notification',
} as const

export function isAppServerConnectionState(value: unknown): value is AppServerConnectionState {
  return (
    value === 'Disconnected' ||
    value === 'Starting' ||
    value === 'Initializing' ||
    value === 'Ready' ||
    value === 'Reconnecting' ||
    value === 'ShuttingDown'
  )
}
