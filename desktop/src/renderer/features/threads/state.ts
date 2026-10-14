import type {
  V2Thread as Thread,
  V2ThreadItem as ThreadItem,
  V2ThreadListResponse as ThreadListResponse,
  V2Turn as Turn,
  V2TurnStatus as TurnStatus,
} from '@codex/app-server-protocol'
import type { AppServerNotificationEvent } from '../../../shared/appServer'
import {
  appendEntries,
  notificationReducer,
  terminalFromTurn,
  turnCompletedReducer,
  turnStartedReducer,
} from './notifications'

export type ThreadListPhase = 'NotLoaded' | 'Loading' | 'Ready'
export type TurnPhase = 'Idle' | 'Starting' | 'Running' | 'Terminal'
export type TerminalTurnStatus = Extract<TurnStatus, 'completed' | 'interrupted' | 'failed'>
export type ThreadViewOperation = 'create' | 'resume' | 'read'

export interface ConversationErrorItem {
  type: 'error'
  id: string
  message: string
  willRetry: boolean
}

export type ConversationItem = ThreadItem | ConversationErrorItem

export interface ConversationEntry {
  id: string
  sequence: number
  item: ConversationItem
}

export interface TurnRuntimeState {
  phase: TurnPhase
  turn: Turn | null
  terminalStatus: TerminalTurnStatus | null
  error: string | null
}

export interface ThreadListState {
  phase: ThreadListPhase
  searchTerm: string
  archivedOnly: boolean
  threads: Thread[]
  nextCursor: string | null
  loadingMore: boolean
  inFlightCursor: string | null
  usedCursors: string[]
  requestId: number
  error: string | null
}

export interface ThreadViewState {
  phase: ThreadListPhase
  operation: ThreadViewOperation | null
  requestId: number
  threadId: string | null
  thread: Thread | null
  entries: ConversationEntry[]
  turn: TurnRuntimeState
  activeTurnId: string | null
  error: string | null
  notice: string | null
}

export interface ThreadWorkspaceState {
  list: ThreadListState
  view: ThreadViewState
  notificationSequence: number
}

export type ThreadWorkspaceAction =
  | { type: 'listLoadStarted'; requestId: number; cursor: string | null; first: boolean }
  | { type: 'filterChanged'; searchTerm: string; archivedOnly: boolean }
  | {
      type: 'listLoadSucceeded'
      requestId: number
      cursor: string | null
      first: boolean
      response: ThreadListResponse
    }
  | { type: 'listLoadFailed'; requestId: number; message: string }
  | { type: 'threadCreated'; thread: Thread }
  | {
      type: 'viewLoadStarted'
      requestId: number
      threadId: string | null
      operation: ThreadViewOperation
    }
  | {
      type: 'viewLoadSucceeded'
      requestId: number
      expectedThreadId: string | null
      thread: Thread
      operation: ThreadViewOperation
    }
  | { type: 'viewLoadFailed'; requestId: number; message: string }
  | { type: 'turnStartStarted'; requestId: number }
  | { type: 'turnStartSucceeded'; requestId: number; turn: Turn }
  | { type: 'turnStartFailed'; requestId: number; message: string }
  | { type: 'notificationReceived'; event: AppServerNotificationEvent }
  | { type: 'connectionLost'; message: string }
  | { type: 'durableReset' }

const initialTurn: TurnRuntimeState = {
  phase: 'Idle',
  turn: null,
  terminalStatus: null,
  error: null,
}

export const initialThreadWorkspaceState: ThreadWorkspaceState = {
  list: {
    phase: 'NotLoaded',
    searchTerm: '',
    archivedOnly: false,
    threads: [],
    nextCursor: null,
    loadingMore: false,
    inFlightCursor: null,
    usedCursors: [],
    requestId: 0,
    error: null,
  },
  view: {
    phase: 'NotLoaded',
    operation: null,
    requestId: 0,
    threadId: null,
    thread: null,
    entries: [],
    turn: initialTurn,
    activeTurnId: null,
    error: null,
    notice: null,
  },
  notificationSequence: 0,
}

function threadLabel(thread: Thread): string {
  return thread.name ?? thread.preview ?? thread.id
}

function threadMatchesFilters(thread: Thread, searchTerm: string, archivedOnly: boolean): boolean {
  if (archivedOnly) {
    return false
  }
  const term = searchTerm.trim().toLowerCase()
  return term.length === 0 || threadLabel(thread).toLowerCase().includes(term)
}

export function getThreadTitle(thread: Thread | null): string {
  return thread ? threadLabel(thread) : 'No thread selected'
}

function makeListError(state: ThreadListState, message: string): ThreadListState {
  return {
    ...state,
    phase: 'Ready',
    loadingMore: false,
    inFlightCursor: null,
    error: message,
  }
}

function hydrateView(
  view: ThreadViewState,
  thread: Thread,
  operation: ThreadViewOperation,
): ThreadViewState {
  const entries = thread.turns.reduce<ConversationEntry[]>((accumulator, turn) => {
    const start = accumulator.length
    return appendEntries(accumulator, turn.items, start)
  }, [])
  const activeTurn = [...thread.turns].reverse().find((turn) => turn.status === 'inProgress')
  const terminalTurn =
    activeTurn ?? [...thread.turns].reverse().find((turn) => turn.status !== 'inProgress')
  const turn: TurnRuntimeState = activeTurn
    ? {
        phase: 'Running',
        turn: activeTurn,
        terminalStatus: null,
        error: activeTurn.error?.message ?? null,
      }
    : terminalTurn
      ? {
          phase: 'Terminal',
          turn: terminalTurn,
          terminalStatus: terminalFromTurn(terminalTurn),
          error: terminalTurn.error?.message ?? null,
        }
      : initialTurn

  return {
    ...view,
    phase: 'Ready',
    operation,
    threadId: thread.id,
    thread,
    entries,
    turn,
    activeTurnId: activeTurn?.id ?? terminalTurn?.id ?? null,
    error: null,
  }
}

function listReducer(state: ThreadListState, action: ThreadWorkspaceAction): ThreadListState {
  switch (action.type) {
    case 'listLoadStarted':
      return action.first
        ? {
            ...state,
            phase: 'Loading',
            threads: [],
            nextCursor: null,
            loadingMore: false,
            inFlightCursor: null,
            usedCursors: [],
            requestId: action.requestId,
            error: null,
          }
        : {
            ...state,
            loadingMore: true,
            inFlightCursor: action.cursor,
            requestId: action.requestId,
            error: null,
          }
    case 'listLoadSucceeded': {
      if (action.requestId !== state.requestId) {
        return state
      }
      if (!action.first && action.cursor !== state.nextCursor) {
        return makeListError(state, 'Thread page cursor changed before the response arrived')
      }
      if (!action.first && (action.cursor === null || state.usedCursors.includes(action.cursor))) {
        return makeListError(state, 'Thread page requested an invalid or duplicate cursor')
      }
      if (
        action.response.nextCursor !== null &&
        (action.response.nextCursor === action.cursor ||
          state.usedCursors.includes(action.response.nextCursor))
      ) {
        return makeListError(state, 'App server returned an invalid or repeated next cursor')
      }

      const currentIds = new Set(action.first ? [] : state.threads.map((thread) => thread.id))
      for (const thread of action.response.data) {
        if (currentIds.has(thread.id)) {
          return makeListError(state, `App server returned duplicate thread ${thread.id}`)
        }
        currentIds.add(thread.id)
      }

      return {
        ...state,
        phase: 'Ready',
        threads: action.first ? action.response.data : [...state.threads, ...action.response.data],
        nextCursor: action.response.nextCursor,
        loadingMore: false,
        inFlightCursor: null,
        usedCursors:
          action.cursor === null || state.usedCursors.includes(action.cursor)
            ? state.usedCursors
            : [...state.usedCursors, action.cursor],
        error: null,
      }
    }
    case 'listLoadFailed':
      return action.requestId === state.requestId ? makeListError(state, action.message) : state
    case 'filterChanged':
      return {
        ...initialThreadWorkspaceState.list,
        searchTerm: action.searchTerm,
        archivedOnly: action.archivedOnly,
      }
    case 'threadCreated':
      if (
        state.threads.some((thread) => thread.id === action.thread.id) ||
        !threadMatchesFilters(action.thread, state.searchTerm, state.archivedOnly)
      ) {
        return state
      }
      return {
        ...state,
        threads: [action.thread, ...state.threads],
      }
    case 'durableReset':
      return {
        ...initialThreadWorkspaceState.list,
        searchTerm: state.searchTerm,
        archivedOnly: state.archivedOnly,
      }
    case 'connectionLost':
      if (state.phase !== 'Loading' && !state.loadingMore) {
        return state
      }
      return makeListError(state, action.message)
    default:
      return state
  }
}

function viewReducer(state: ThreadViewState, action: ThreadWorkspaceAction): ThreadViewState {
  switch (action.type) {
    case 'viewLoadStarted':
      return {
        ...state,
        phase: 'Loading',
        operation: action.operation,
        requestId: action.requestId,
        threadId: action.threadId,
        thread: null,
        entries: [],
        turn: initialTurn,
        activeTurnId: null,
        error: null,
        notice: null,
      }
    case 'viewLoadSucceeded': {
      if (action.requestId !== state.requestId) {
        return state
      }
      if (action.expectedThreadId !== null && action.thread.id !== action.expectedThreadId) {
        return {
          ...state,
          phase: 'Ready',
          operation: null,
          thread: null,
          entries: [],
          turn: initialTurn,
          activeTurnId: null,
          error: `App server returned thread ${action.thread.id}, expected ${action.expectedThreadId}`,
        }
      }
      return hydrateView(state, action.thread, action.operation)
    }
    case 'viewLoadFailed':
      return action.requestId === state.requestId
        ? {
            ...state,
            phase: 'Ready',
            operation: null,
            thread: null,
            entries: [],
            turn: initialTurn,
            activeTurnId: null,
            error: action.message,
          }
        : state
    case 'turnStartStarted':
      return {
        ...state,
        requestId: action.requestId,
        turn: { ...initialTurn, phase: 'Starting' },
        activeTurnId: null,
        error: null,
      }
    case 'turnStartSucceeded':
      if (action.requestId !== state.requestId) {
        return state
      }
      return action.turn.status === 'inProgress'
        ? turnStartedReducer(state, action.turn)
        : turnCompletedReducer(state, action.turn)
    case 'turnStartFailed':
      if (action.requestId !== state.requestId) {
        return state
      }
      return {
        ...state,
        turn:
          state.turn.phase === 'Terminal'
            ? { ...state.turn, error: action.message }
            : {
                ...state.turn,
                phase: 'Terminal',
                turn: state.turn.turn,
                terminalStatus: 'failed',
                error: action.message,
              },
      }
    case 'notificationReceived':
      return notificationReducer(state, action.event)
    case 'durableReset':
      return {
        ...state,
        phase: 'NotLoaded',
        operation: null,
        thread: null,
        entries: [],
        turn: initialTurn,
        activeTurnId: null,
        error: null,
        notice: null,
      }
    case 'connectionLost':
      return {
        ...state,
        phase: state.phase === 'Loading' ? 'Ready' : state.phase,
        operation: state.phase === 'Loading' ? null : state.operation,
        turn:
          state.turn.phase === 'Starting' || state.turn.phase === 'Running'
            ? {
                ...state.turn,
                phase: 'Terminal',
                terminalStatus: 'interrupted',
                error: action.message,
              }
            : state.turn,
        error: state.error ?? action.message,
      }
    default:
      return state
  }
}

export function threadWorkspaceReducer(
  state: ThreadWorkspaceState,
  action: ThreadWorkspaceAction,
): ThreadWorkspaceState {
  if (action.type === 'notificationReceived') {
    return {
      list: state.list,
      view: viewReducer(state.view, action),
      notificationSequence: state.notificationSequence + 1,
    }
  }

  return {
    list: listReducer(state.list, action),
    view: viewReducer(state.view, action),
    notificationSequence: state.notificationSequence,
  }
}

export function canStartTurn(view: ThreadViewState): boolean {
  return (
    view.phase === 'Ready' &&
    view.threadId !== null &&
    view.error === null &&
    (view.turn.phase === 'Idle' || view.turn.phase === 'Terminal')
  )
}
