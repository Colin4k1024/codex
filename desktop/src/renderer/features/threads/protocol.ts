import type {
  V2Thread as Thread,
  V2ThreadItem as ThreadItem,
  V2ThreadListParams as ThreadListParams,
  V2ThreadListResponse as ThreadListResponse,
  V2ThreadReadParams as ThreadReadParams,
  V2ThreadReadResponse as ThreadReadResponse,
  V2ThreadResumeParams as ThreadResumeParams,
  V2ThreadResumeResponse as ThreadResumeResponse,
  V2ThreadStartParams as ThreadStartParams,
  V2ThreadStartResponse as ThreadStartResponse,
  V2Turn as Turn,
  V2TurnStartParams as TurnStartParams,
  V2TurnStartResponse as TurnStartResponse,
} from '@codex/app-server-protocol'
import type { CodexDesktopBridge } from '../../../shared/types'

export const THREAD_PAGE_LIMIT = 50
/** Mirrors the main process JSON-RPC frame limit for renderer-side defense in depth. */
export const MAX_THREAD_RESPONSE_BYTES = 8 * 1024 * 1024

export type ThreadProtocolErrorKind =
  | 'invalidResponse'
  | 'oversizedResponse'
  | 'requestFailed'
  | 'unknown'

export class ThreadProtocolError extends Error {
  readonly kind: ThreadProtocolErrorKind

  constructor(kind: ThreadProtocolErrorKind, message: string) {
    super(message)
    this.name = 'ThreadProtocolError'
    this.kind = kind
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === 'string'
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((entry) => typeof entry === 'string')
}

function isNullableNumber(value: unknown): value is number | null {
  return value === null || typeof value === 'number'
}

const turnStatuses = new Set(['completed', 'interrupted', 'failed', 'inProgress'])
const threadStatuses = new Set(['notLoaded', 'idle', 'systemError', 'active'])

export function isThreadItem(value: unknown): value is ThreadItem {
  if (
    !isRecord(value) ||
    typeof value.type !== 'string' ||
    typeof value.id !== 'string' ||
    value.id.length === 0
  ) {
    return false
  }

  switch (value.type) {
    case 'userMessage':
      return Array.isArray(value.content) && value.content.every(isRecord)
    case 'agentMessage':
    case 'plan':
      return typeof value.text === 'string'
    case 'reasoning':
      return isStringArray(value.summary) && isStringArray(value.content)
    case 'commandExecution':
      return (
        typeof value.command === 'string' &&
        typeof value.cwd === 'string' &&
        typeof value.status === 'string' &&
        Array.isArray(value.commandActions) &&
        isNullableString(value.aggregatedOutput) &&
        isNullableNumber(value.exitCode) &&
        isNullableNumber(value.durationMs)
      )
    case 'fileChange':
      return Array.isArray(value.changes) && typeof value.status === 'string'
    case 'mcpToolCall':
    case 'dynamicToolCall':
      return typeof value.tool === 'string' && typeof value.status === 'string'
    default:
      return true
  }
}

export function isTurn(value: unknown): value is Turn {
  return (
    isRecord(value) &&
    typeof value.id === 'string' &&
    value.id.length > 0 &&
    Array.isArray(value.items) &&
    value.items.every(isThreadItem) &&
    typeof value.status === 'string' &&
    turnStatuses.has(value.status) &&
    (value.error === null || isRecord(value.error))
  )
}

export function isThread(value: unknown): value is Thread {
  return (
    isRecord(value) &&
    typeof value.id === 'string' &&
    value.id.length > 0 &&
    typeof value.sessionId === 'string' &&
    typeof value.preview === 'string' &&
    typeof value.ephemeral === 'boolean' &&
    typeof value.modelProvider === 'string' &&
    isNullableString(value.model) &&
    isNullableNumber(value.recencyAt) &&
    isNullableString(value.path) &&
    typeof value.cwd === 'string' &&
    typeof value.cliVersion === 'string' &&
    isNullableString(value.name) &&
    typeof value.createdAt === 'number' &&
    typeof value.updatedAt === 'number' &&
    isRecord(value.status) &&
    typeof value.status.type === 'string' &&
    threadStatuses.has(value.status.type) &&
    (value.status.type !== 'active' || Array.isArray(value.status.activeFlags)) &&
    Array.isArray(value.turns) &&
    value.turns.every(isTurn)
  )
}

function isThreadListResponse(value: unknown): value is ThreadListResponse {
  return (
    isRecord(value) &&
    Array.isArray(value.data) &&
    value.data.every(isThread) &&
    isNullableString(value.nextCursor) &&
    isNullableString(value.backwardsCursor)
  )
}

function isThreadStartResponse(value: unknown): value is ThreadStartResponse {
  return (
    isRecord(value) &&
    isThread(value.thread) &&
    typeof value.model === 'string' &&
    typeof value.modelProvider === 'string' &&
    typeof value.cwd === 'string'
  )
}

function isThreadResumeResponse(value: unknown): value is ThreadResumeResponse {
  return (
    isThreadStartResponse(value) &&
    isNullableString((value as { turnsBackwardsCursor?: unknown }).turnsBackwardsCursor) &&
    isNullableString((value as { itemsBackwardsCursor?: unknown }).itemsBackwardsCursor)
  )
}

function isThreadReadResponse(value: unknown): value is ThreadReadResponse {
  return isRecord(value) && isThread(value.thread)
}

function isTurnStartResponse(value: unknown): value is TurnStartResponse {
  return isRecord(value) && isTurn(value.turn)
}

export function getThreadErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof ThreadProtocolError) {
    return error.message
  }
  if (error instanceof Error && error.message) {
    return error.message
  }
  if (isRecord(error) && typeof error.message === 'string' && error.message) {
    return error.message
  }
  return fallback
}

async function requestValidated<TResponse>(
  bridge: Pick<CodexDesktopBridge, 'requestAppServer'>,
  method: 'thread/list' | 'thread/start' | 'thread/resume' | 'thread/read' | 'turn/start',
  params:
    | ThreadListParams
    | ThreadStartParams
    | ThreadResumeParams
    | ThreadReadParams
    | TurnStartParams,
  guard: (value: unknown) => value is TResponse,
): Promise<TResponse> {
  let response: unknown
  try {
    response = await bridge.requestAppServer(method, params)
  } catch (error) {
    throw new ThreadProtocolError(
      'requestFailed',
      getThreadErrorMessage(error, `${method} request failed`),
    )
  }

  const serialized = JSON.stringify(response ?? null)
  const serializedByteLength = new TextEncoder().encode(serialized).length
  if (serializedByteLength > MAX_THREAD_RESPONSE_BYTES) {
    throw new ThreadProtocolError(
      'oversizedResponse',
      `${method} response exceeds the ${MAX_THREAD_RESPONSE_BYTES} byte limit`,
    )
  }
  if (!guard(response)) {
    throw new ThreadProtocolError('invalidResponse', `${method} response is invalid`)
  }
  return response
}

export interface ThreadBridgeClient {
  listThreads(params?: ThreadListParams): Promise<ThreadListResponse>
  startThread(params: ThreadStartParams): Promise<ThreadStartResponse>
  resumeThread(params: ThreadResumeParams): Promise<ThreadResumeResponse>
  readThread(params: ThreadReadParams): Promise<ThreadReadResponse>
  startTurn(params: TurnStartParams): Promise<TurnStartResponse>
}

export function createThreadBridgeClient(
  bridge: Pick<CodexDesktopBridge, 'requestAppServer'>,
): ThreadBridgeClient {
  return {
    listThreads: (params = {}) =>
      requestValidated(bridge, 'thread/list', params, isThreadListResponse),
    startThread: (params) =>
      requestValidated(bridge, 'thread/start', params, isThreadStartResponse),
    resumeThread: (params) =>
      requestValidated(bridge, 'thread/resume', params, isThreadResumeResponse),
    readThread: (params) => requestValidated(bridge, 'thread/read', params, isThreadReadResponse),
    startTurn: (params) => requestValidated(bridge, 'turn/start', params, isTurnStartResponse),
  }
}
