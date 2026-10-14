import type { AppServerRpcError } from '../../shared/appServer'

export const JSON_RPC_ERROR_CODES = {
  parse: -32700,
  invalidRequest: -32600,
  methodNotFound: -32601,
  invalidParams: -32602,
  internal: -32603,
  overloaded: -32001,
} as const

export const SHUTDOWN_ADMISSION_REASON = 'serverShuttingDown'

interface RawRpcError {
  code?: unknown
  message?: unknown
  data?: unknown
}

function readErrorData(value: unknown): unknown {
  if (typeof value !== 'object' || value === null || !('data' in value)) {
    return undefined
  }
  return (value as { data?: unknown }).data
}

/**
 * Classifies a JSON-RPC error object. `serverShuttingDown` admission errors
 * (-32600 + data.reason) are distinguished from generic invalid requests so the
 * lifecycle layer can reconnect instead of reporting a bad request.
 */
export function classifyRpcError(raw: RawRpcError): AppServerRpcError {
  const code = typeof raw.code === 'number' && Number.isFinite(raw.code) ? raw.code : null
  const message = typeof raw.message === 'string' ? raw.message : 'app-server returned an error'
  const data = readErrorData(raw)

  if (code === JSON_RPC_ERROR_CODES.invalidRequest) {
    const isShutdown =
      typeof data === 'object' &&
      data !== null &&
      'reason' in data &&
      (data as { reason?: unknown }).reason === SHUTDOWN_ADMISSION_REASON
    if (isShutdown) {
      return { code, message, kind: 'serverShuttingDown', data, retryable: false }
    }
    return { code, message, kind: 'invalidRequest', data, retryable: false }
  }

  if (code === JSON_RPC_ERROR_CODES.parse) {
    return { code, message, kind: 'parse', data, retryable: false }
  }
  if (code === JSON_RPC_ERROR_CODES.methodNotFound) {
    return { code, message, kind: 'methodNotFound', data, retryable: false }
  }
  if (code === JSON_RPC_ERROR_CODES.invalidParams) {
    return { code, message, kind: 'invalidParams', data, retryable: false }
  }
  if (code === JSON_RPC_ERROR_CODES.overloaded) {
    return { code, message, kind: 'overloaded', data, retryable: true }
  }
  if (code === JSON_RPC_ERROR_CODES.internal) {
    return { code, message, kind: 'internal', data, retryable: false }
  }

  return { code, message, kind: 'unknown', data, retryable: false }
}

export function localError(
  kind: AppServerRpcError['kind'],
  message: string,
  options: { retryable?: boolean; code?: number | null } = {},
): AppServerRpcError {
  return {
    code: options.code ?? null,
    message,
    kind,
    retryable: options.retryable ?? false,
  }
}
