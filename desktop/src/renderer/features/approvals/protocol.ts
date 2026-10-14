import type {
  V2CommandExecutionApprovalDecision as CommandExecutionApprovalDecision,
  V2CommandExecutionRequestApprovalParams as CommandExecutionRequestApprovalParams,
  V2CommandExecutionRequestApprovalResponse as CommandExecutionRequestApprovalResponse,
  V2FileChangeApprovalDecision as FileChangeApprovalDecision,
  V2FileChangeRequestApprovalParams as FileChangeRequestApprovalParams,
  V2FileChangeRequestApprovalResponse as FileChangeRequestApprovalResponse,
  JsonValue,
  V2McpServerElicitationAction as McpServerElicitationAction,
  V2McpServerElicitationRequestParams as McpServerElicitationRequestParams,
  V2McpServerElicitationRequestResponse as McpServerElicitationRequestResponse,
  V2ToolRequestUserInputAnswer as ToolRequestUserInputAnswer,
  V2ToolRequestUserInputParams as ToolRequestUserInputParams,
  V2ToolRequestUserInputQuestion as ToolRequestUserInputQuestion,
  V2ToolRequestUserInputResponse as ToolRequestUserInputResponse,
} from '@codex/app-server-protocol'
import type { AppServerNotificationEvent, AppServerRequestId } from '../../../shared/appServer'

export type ApprovalKind = 'commandExecution' | 'fileChange' | 'toolUserInput' | 'mcpElicitation'

export interface ApprovalRequestBase {
  id: AppServerRequestId
  kind: ApprovalKind
  threadId: string
  turnId: string | null
  itemId: string
  startedAtMs: number | null
}

export interface CommandExecutionApprovalRequest extends ApprovalRequestBase {
  kind: 'commandExecution'
  params: CommandExecutionRequestApprovalParams
}

export interface FileChangeApprovalRequest extends ApprovalRequestBase {
  kind: 'fileChange'
  params: FileChangeRequestApprovalParams
  changes?: FileChangeDiff[]
}

export interface ToolUserInputApprovalRequest extends ApprovalRequestBase {
  kind: 'toolUserInput'
  params: ToolRequestUserInputParams
}

export interface McpElicitationApprovalRequest extends ApprovalRequestBase {
  kind: 'mcpElicitation'
  params: McpServerElicitationRequestParams
}

export type ApprovalRequest =
  | CommandExecutionApprovalRequest
  | FileChangeApprovalRequest
  | ToolUserInputApprovalRequest
  | McpElicitationApprovalRequest

export type ApprovalDecision =
  | { kind: 'commandExecution'; decision: CommandExecutionApprovalDecision }
  | { kind: 'fileChange'; decision: FileChangeApprovalDecision }
  | { kind: 'toolUserInput'; answers: Record<string, ToolRequestUserInputAnswer> }
  | {
      kind: 'mcpElicitation'
      action: McpServerElicitationAction
      content: JsonValue
    }

export type ApprovalState = 'Pending' | 'Responded' | 'Cancelled' | 'Expired' | 'Disconnected'

export interface ApprovalEntry {
  request: ApprovalRequest
  state: ApprovalState
  error: string | null
}

export interface FileChangeDiff {
  path: string
  kind: string
  diff: string
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function isString(value: unknown): value is string {
  return typeof value === 'string'
}

function isNumber(value: unknown): value is number {
  return typeof value === 'number'
}

function isNullableString(value: unknown): value is string | null {
  return value === null || typeof value === 'string'
}

function isNullableNumber(value: unknown): value is number | null {
  return value === null || typeof value === 'number'
}

function isJsonValue(value: unknown): boolean {
  if (
    value === null ||
    typeof value === 'string' ||
    typeof value === 'number' ||
    typeof value === 'boolean'
  ) {
    return true
  }
  if (Array.isArray(value)) {
    return value.every(isJsonValue)
  }
  return isRecord(value) && Object.values(value).every(isJsonValue)
}

function isToolRequestUserInputOption(
  value: unknown,
): value is { label: string; description: string } {
  return isRecord(value) && isString(value.label) && isString(value.description)
}

function isToolRequestUserInputQuestion(value: unknown): value is ToolRequestUserInputQuestion {
  return (
    isRecord(value) &&
    isString(value.id) &&
    isString(value.header) &&
    isString(value.question) &&
    typeof value.isOther === 'boolean' &&
    typeof value.isSecret === 'boolean' &&
    (value.options === null ||
      (Array.isArray(value.options) && value.options.every(isToolRequestUserInputOption)))
  )
}

export function isCommandExecutionApprovalParams(
  value: unknown,
): value is CommandExecutionRequestApprovalParams {
  return (
    isRecord(value) &&
    isString(value.threadId) &&
    isString(value.turnId) &&
    isString(value.itemId) &&
    isNumber(value.startedAtMs) &&
    (value.approvalId === undefined || value.approvalId === null || isString(value.approvalId)) &&
    (value.environmentId === undefined ||
      value.environmentId === null ||
      isString(value.environmentId)) &&
    (value.reason === undefined || value.reason === null || isString(value.reason)) &&
    (value.command === undefined || value.command === null || isString(value.command)) &&
    (value.cwd === undefined || value.cwd === null || isString(value.cwd))
  )
}

export function isFileChangeApprovalParams(
  value: unknown,
): value is FileChangeRequestApprovalParams {
  return (
    isRecord(value) &&
    isString(value.threadId) &&
    isString(value.turnId) &&
    isString(value.itemId) &&
    isNumber(value.startedAtMs) &&
    (value.reason === undefined || value.reason === null || isString(value.reason)) &&
    (value.grantRoot === undefined || value.grantRoot === null || isString(value.grantRoot))
  )
}

export function isToolUserInputParams(value: unknown): value is ToolRequestUserInputParams {
  return (
    isRecord(value) &&
    isString(value.threadId) &&
    isString(value.turnId) &&
    isString(value.itemId) &&
    Array.isArray(value.questions) &&
    value.questions.every(isToolRequestUserInputQuestion) &&
    typeof value.isBlocking === 'boolean' &&
    isNullableNumber(value.autoResolutionMs)
  )
}

function isMcpElicitationParams(value: unknown): value is McpServerElicitationRequestParams {
  if (
    !isRecord(value) ||
    !isString(value.threadId) ||
    !isNullableString(value.turnId) ||
    !isString(value.serverName) ||
    !isString(value.mode) ||
    !isString(value.message)
  ) {
    return false
  }
  if (value.mode === 'url') {
    return isString(value.url) && isString(value.elicitationId)
  }
  if (value.mode === 'form') {
    return isRecord(value.requestedSchema)
  }
  return isJsonValue(value.requestedSchema)
}

export function isRequestId(value: unknown): value is AppServerRequestId {
  return isString(value) || (isNumber(value) && Number.isFinite(value))
}

export function parseApprovalRequest(event: AppServerNotificationEvent): ApprovalRequest | null {
  if (!isRequestId(event.id) || !isRecord(event.params)) {
    return null
  }
  const id = event.id
  const params = event.params

  if (event.method === 'item/commandExecution/requestApproval') {
    if (!isCommandExecutionApprovalParams(params)) {
      return null
    }
    return {
      id,
      kind: 'commandExecution',
      threadId: params.threadId,
      turnId: params.turnId,
      itemId: params.itemId,
      startedAtMs: params.startedAtMs,
      params,
    }
  }

  if (event.method === 'item/fileChange/requestApproval') {
    if (!isFileChangeApprovalParams(params)) {
      return null
    }
    return {
      id,
      kind: 'fileChange',
      threadId: params.threadId,
      turnId: params.turnId,
      itemId: params.itemId,
      startedAtMs: params.startedAtMs,
      params,
    }
  }

  if (event.method === 'item/tool/requestUserInput') {
    if (!isToolUserInputParams(params)) {
      return null
    }
    return {
      id,
      kind: 'toolUserInput',
      threadId: params.threadId,
      turnId: params.turnId,
      itemId: params.itemId,
      startedAtMs: null,
      params,
    }
  }

  if (event.method === 'mcpServer/elicitation/request') {
    if (!isMcpElicitationParams(params)) {
      return null
    }
    return {
      id,
      kind: 'mcpElicitation',
      threadId: params.threadId,
      turnId: params.turnId,
      itemId: `${params.serverName}:${id}`,
      startedAtMs: null,
      params,
    }
  }

  return null
}

export function isApprovalRequestMethod(method: string): boolean {
  return (
    method === 'item/commandExecution/requestApproval' ||
    method === 'item/fileChange/requestApproval' ||
    method === 'item/tool/requestUserInput' ||
    method === 'mcpServer/elicitation/request'
  )
}

export function buildApprovalResponse(request: ApprovalRequest, decision: ApprovalDecision) {
  switch (request.kind) {
    case 'commandExecution': {
      const response: CommandExecutionRequestApprovalResponse = {
        decision: decision.kind === 'commandExecution' ? decision.decision : ('decline' as const),
      }
      return response
    }
    case 'fileChange': {
      const response: FileChangeRequestApprovalResponse = {
        decision: decision.kind === 'fileChange' ? decision.decision : ('decline' as const),
      }
      return response
    }
    case 'toolUserInput': {
      const response: ToolRequestUserInputResponse = {
        answers: decision.kind === 'toolUserInput' ? decision.answers : {},
      }
      return response
    }
    case 'mcpElicitation': {
      const response: McpServerElicitationRequestResponse = {
        action: decision.kind === 'mcpElicitation' ? decision.action : ('decline' as const),
        content:
          decision.kind === 'mcpElicitation' && decision.action === 'accept'
            ? decision.content
            : null,
        _meta: null,
      }
      return response
    }
  }
}

export function isFileChangePatchUpdate(value: unknown): value is {
  threadId: string
  turnId: string
  itemId: string
  changes: FileChangeDiff[]
} {
  return (
    isRecord(value) &&
    isString(value.threadId) &&
    isString(value.turnId) &&
    isString(value.itemId) &&
    Array.isArray(value.changes) &&
    value.changes.every(
      (change) =>
        isRecord(change) &&
        isString(change.path) &&
        change.path.length > 0 &&
        isString(change.kind) &&
        isString(change.diff),
    )
  )
}

export function getApprovalTitle(request: ApprovalRequest): string {
  switch (request.kind) {
    case 'commandExecution':
      return 'Command execution approval'
    case 'fileChange':
      return 'File change approval'
    case 'toolUserInput':
      return 'Tool input requested'
    case 'mcpElicitation':
      return 'MCP elicitation requested'
  }
}

export function getApprovalDescription(request: ApprovalRequest): string {
  switch (request.kind) {
    case 'commandExecution':
      return request.params.command ?? 'Unknown command'
    case 'fileChange':
      return request.params.reason ?? 'File change requested'
    case 'toolUserInput':
      return `${request.params.questions.length} question(s) from tool`
    case 'mcpElicitation':
      return `${request.params.serverName}: ${request.params.message}`
  }
}
