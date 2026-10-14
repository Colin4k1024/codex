import { useCallback, useEffect, useReducer, useRef } from 'react'
import type { AppServerRequestId, AppServerStatus } from '../../../shared/appServer'
import type { CodexDesktopBridge } from '../../../shared/types'
import {
  approvalWorkspaceReducer,
  initialApprovalWorkspaceState,
  type ApprovalWorkspaceState,
} from './state'
import {
  buildApprovalResponse,
  isApprovalRequestMethod,
  isFileChangePatchUpdate,
  parseApprovalRequest,
  type ApprovalDecision,
  type ApprovalRequest,
} from './protocol'

export interface ApprovalWorkspaceController {
  state: ApprovalWorkspaceState
  respond: (request: ApprovalRequest, decision: ApprovalDecision) => Promise<void>
  cancel: (requestId: AppServerRequestId) => Promise<void>
  select: (requestId: AppServerRequestId | null) => void
}

export interface UseApprovalWorkspaceOptions {
  bridge: CodexDesktopBridge
  appServerStatus: AppServerStatus | null
}

export function useApprovalWorkspace({
  bridge,
  appServerStatus,
}: UseApprovalWorkspaceOptions): ApprovalWorkspaceController {
  const [state, dispatch] = useReducer(approvalWorkspaceReducer, initialApprovalWorkspaceState)
  const stateRef = useRef(state)
  const inFlightRef = useRef<Set<AppServerRequestId>>(new Set())

  useEffect(() => {
    stateRef.current = state
  }, [state])

  useEffect(() => {
    const unsubscribe = bridge.onAppServerNotification((event) => {
      if (isApprovalRequestMethod(event.method)) {
        const request = parseApprovalRequest(event)
        if (request === null) {
          dispatch({ type: 'malformed', message: 'Malformed approval request' })
          return
        }
        dispatch({ type: 'requestReceived', request })
        return
      }

      if (event.method === 'item/fileChange/patchUpdated') {
        if (isFileChangePatchUpdate(event.params)) {
          dispatch({
            type: 'patchUpdated',
            threadId: event.params.threadId,
            turnId: event.params.turnId,
            itemId: event.params.itemId,
            changes: event.params.changes,
          })
        } else {
          dispatch({ type: 'malformed', message: 'Malformed file change diff' })
        }
        return
      }

      if (event.method === 'serverRequest/resolved') {
        const params = event.params as { requestId?: unknown } | undefined
        if (
          params !== null &&
          typeof params === 'object' &&
          (typeof params.requestId === 'string' ||
            (typeof params.requestId === 'number' && Number.isFinite(params.requestId)))
        ) {
          dispatch({ type: 'responded', requestId: params.requestId })
        }
      }
    })
    return unsubscribe
  }, [bridge])

  useEffect(() => {
    if (appServerStatus === null || appServerStatus.state === 'Ready') {
      return
    }
    const message =
      appServerStatus.lastError?.message ??
      'App server disconnected; pending approvals cannot be resolved'
    dispatch({ type: 'disconnected', message })
  }, [appServerStatus])

  const sendResponse = useCallback(
    async (request: ApprovalRequest, decision: ApprovalDecision): Promise<void> => {
      if (inFlightRef.current.has(request.id)) {
        return
      }
      const entry = stateRef.current.entries.find((item) => item.request.id === request.id)
      if (!entry || entry.state !== 'Pending') {
        return
      }

      inFlightRef.current.add(request.id)
      try {
        const result = buildApprovalResponse(request, decision)
        await bridge.respondAppServerRequest(request.id, result)
        dispatch({ type: 'responded', requestId: request.id })
      } catch (error) {
        dispatch({
          type: 'responseFailed',
          requestId: request.id,
          message:
            error instanceof Error
              ? error.message
              : 'Unable to send the approval response; the request remains pending',
        })
      } finally {
        inFlightRef.current.delete(request.id)
      }
    },
    [bridge],
  )

  const respond = useCallback(
    (request: ApprovalRequest, decision: ApprovalDecision): Promise<void> =>
      sendResponse(request, decision),
    [sendResponse],
  )

  const cancel = useCallback(
    async (requestId: AppServerRequestId): Promise<void> => {
      const entry = stateRef.current.entries.find((item) => item.request.id === requestId)
      if (!entry || entry.state !== 'Pending') {
        return
      }
      const request = entry.request
      if (request.kind === 'commandExecution') {
        await sendResponse(request, { kind: 'commandExecution', decision: 'cancel' })
        return
      }
      if (request.kind === 'fileChange') {
        await sendResponse(request, { kind: 'fileChange', decision: 'cancel' })
        return
      }
      if (request.kind === 'toolUserInput') {
        await sendResponse(request, { kind: 'toolUserInput', answers: {} })
        return
      }
      await sendResponse(request, {
        kind: 'mcpElicitation',
        action: 'cancel',
        content: null,
      })
    },
    [sendResponse],
  )

  const select = useCallback((requestId: AppServerRequestId | null): void => {
    dispatch({ type: 'select', requestId })
  }, [])

  return {
    state,
    respond,
    cancel,
    select,
  }
}
