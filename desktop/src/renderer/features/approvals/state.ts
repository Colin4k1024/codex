import type { ApprovalEntry, ApprovalRequest, ApprovalState, FileChangeDiff } from './protocol'
import type { AppServerRequestId } from '../../../shared/appServer'

export type { ApprovalEntry }

export interface ApprovalWorkspaceState {
  entries: ApprovalEntry[]
  selectedId: AppServerRequestId | null
  error: string | null
}

export type ApprovalWorkspaceAction =
  | { type: 'requestReceived'; request: ApprovalRequest }
  | { type: 'responded'; requestId: AppServerRequestId }
  | { type: 'cancelled'; requestId: AppServerRequestId }
  | { type: 'expired'; requestId: AppServerRequestId }
  | { type: 'disconnected'; message: string }
  | { type: 'select'; requestId: AppServerRequestId | null }
  | { type: 'clearError' }
  | {
      type: 'patchUpdated'
      threadId: string
      turnId: string
      itemId: string
      changes: FileChangeDiff[]
    }
  | { type: 'responseFailed'; requestId: AppServerRequestId; message: string }
  | { type: 'malformed'; message: string }
  | { type: 'reset' }

export const initialApprovalWorkspaceState: ApprovalWorkspaceState = {
  entries: [],
  selectedId: null,
  error: null,
}

export function approvalWorkspaceReducer(
  state: ApprovalWorkspaceState,
  action: ApprovalWorkspaceAction,
): ApprovalWorkspaceState {
  switch (action.type) {
    case 'requestReceived': {
      if (state.entries.some((entry) => entry.request.id === action.request.id)) {
        return state
      }
      const entry: ApprovalEntry = {
        request: action.request,
        state: 'Pending',
        error: null,
      }
      return {
        ...state,
        entries: [...state.entries, entry],
        selectedId: state.selectedId ?? action.request.id,
      }
    }
    case 'responded': {
      return {
        ...state,
        entries: state.entries.map((entry) =>
          entry.request.id === action.requestId
            ? { ...entry, state: 'Responded' as ApprovalState }
            : entry,
        ),
      }
    }
    case 'cancelled': {
      return {
        ...state,
        entries: state.entries.map((entry) =>
          entry.request.id === action.requestId
            ? { ...entry, state: 'Cancelled' as ApprovalState }
            : entry,
        ),
      }
    }
    case 'expired': {
      return {
        ...state,
        entries: state.entries.map((entry) =>
          entry.request.id === action.requestId
            ? { ...entry, state: 'Expired' as ApprovalState }
            : entry,
        ),
      }
    }
    case 'patchUpdated': {
      return {
        ...state,
        entries: state.entries.map((entry) =>
          entry.request.kind === 'fileChange' &&
          entry.request.threadId === action.threadId &&
          entry.request.turnId === action.turnId &&
          entry.request.itemId === action.itemId
            ? { ...entry, request: { ...entry.request, changes: action.changes } }
            : entry,
        ),
      }
    }
    case 'malformed': {
      return { ...state, error: action.message }
    }
    case 'responseFailed': {
      return {
        ...state,
        error: action.message,
        entries: state.entries.map((entry) =>
          entry.request.id === action.requestId ? { ...entry, error: action.message } : entry,
        ),
      }
    }
    case 'disconnected': {
      return {
        ...state,
        entries: state.entries.map((entry) =>
          entry.state === 'Pending'
            ? { ...entry, state: 'Disconnected' as ApprovalState, error: action.message }
            : entry,
        ),
        error: action.message,
      }
    }
    case 'select': {
      return { ...state, selectedId: action.requestId }
    }
    case 'clearError': {
      return { ...state, error: null }
    }
    case 'reset': {
      return initialApprovalWorkspaceState
    }
  }
}

export function getPendingApprovals(state: ApprovalWorkspaceState): ApprovalEntry[] {
  return state.entries.filter((entry) => entry.state === 'Pending')
}

export function getApprovalById(
  state: ApprovalWorkspaceState,
  id: string,
): ApprovalEntry | undefined {
  return state.entries.find((entry) => entry.request.id === id)
}

export function hasPendingApprovals(state: ApprovalWorkspaceState): boolean {
  return state.entries.some((entry) => entry.state === 'Pending')
}
