import type {
  V2Thread as Thread,
  V2ThreadItem as ThreadItem,
  V2ThreadListResponse as ThreadListResponse,
  V2Turn as Turn,
  V2TurnStatus as TurnStatus,
} from '@codex/app-server-protocol'
import type { AppServerNotificationEvent } from '../../../shared/appServer'
import { isThreadItem, isTurn } from './protocol'

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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function notificationParams<T>(event: AppServerNotificationEvent): T | null {
  return isRecord(event.params) ? (event.params as T) : null
}

function threadLabel(thread: Thread): string {
  return thread.name ?? thread.preview ?? thread.id
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

function appendEntries(
  entries: ConversationEntry[],
  items: ConversationItem[],
  startSequence: number,
): ConversationEntry[] {
  let next = [...entries]
  for (const item of items) {
    if (next.some((entry) => entry.id === item.id)) {
      next = next.map((entry) => (entry.id === item.id ? { ...entry, item } : entry))
    } else {
      next.push({ id: item.id, sequence: startSequence + next.length, item })
    }
  }
  return next
}

function upsertItem(
  entries: ConversationEntry[],
  item: ConversationItem,
  sequence: number,
): ConversationEntry[] {
  const existing = entries.find((entry) => entry.id === item.id)
  if (!existing) {
    return [...entries, { id: item.id, sequence, item }]
  }
  return entries.map((entry) => (entry.id === item.id ? { ...entry, item } : entry))
}

function updateItem(
  entries: ConversationEntry[],
  itemId: string,
  update: (item: ThreadItem) => ThreadItem,
): ConversationEntry[] {
  return entries.map((entry) =>
    entry.id === itemId && entry.item.type !== 'error'
      ? { ...entry, item: update(entry.item) }
      : entry,
  )
}

function terminalFromTurn(turn: Turn): TerminalTurnStatus {
  if (turn.status === 'interrupted' || turn.status === 'failed') {
    return turn.status
  }
  return 'completed'
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
    case 'threadCreated':
      return state.threads.some((thread) => thread.id === action.thread.id)
        ? state
        : {
            ...state,
            threads: [action.thread, ...state.threads],
          }
    case 'durableReset':
      return initialThreadWorkspaceState.list
    case 'connectionLost':
      if (state.phase !== 'Loading' && !state.loadingMore) {
        return state
      }
      return makeListError(state, action.message)
    default:
      return state
  }
}

function turnStartedReducer(view: ThreadViewState, turn: Turn): ThreadViewState {
  if (view.turn.phase === 'Terminal') {
    if (view.activeTurnId !== null && view.activeTurnId !== turn.id) {
      return view
    }
    return {
      ...view,
      turn: { ...view.turn, turn },
      activeTurnId: turn.id,
      entries: appendEntries(view.entries, turn.items, view.entries.length),
    }
  }

  if (
    view.turn.phase === 'Running' &&
    view.activeTurnId !== null &&
    view.activeTurnId !== turn.id
  ) {
    return view
  }

  return {
    ...view,
    turn: {
      ...view.turn,
      phase: 'Running',
      turn,
      terminalStatus: null,
      error: turn.error?.message ?? null,
    },
    activeTurnId: turn.id,
    entries: appendEntries(view.entries, turn.items, view.entries.length),
  }
}

function turnCompletedReducer(view: ThreadViewState, turn: Turn): ThreadViewState {
  if (view.turn.phase === 'Idle' || view.turn.phase === 'Terminal') {
    return view
  }
  if (view.activeTurnId !== null && view.activeTurnId !== turn.id) {
    return view
  }

  return {
    ...view,
    turn: {
      ...view.turn,
      phase: 'Terminal',
      turn,
      terminalStatus: terminalFromTurn(turn),
      error: turn.error?.message ?? null,
    },
    activeTurnId: turn.id,
    entries: appendEntries(view.entries, turn.items, view.entries.length),
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
      return action.requestId === state.requestId ? turnStartedReducer(state, action.turn) : state
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

function canApplyTurnItem(view: ThreadViewState, turnId: string): boolean {
  if (view.activeTurnId === null) {
    return view.turn.phase === 'Starting' || view.turn.phase === 'Running'
  }
  return turnId === view.activeTurnId
}

const globalNotificationMethods = new Set(['account/updated', 'account/gatewayOAuth/changed'])

function isForView(event: AppServerNotificationEvent, view: ThreadViewState): boolean {
  if (globalNotificationMethods.has(event.method)) {
    return true
  }
  const params = notificationParams<{ threadId?: unknown }>(event)
  return Boolean(
    params &&
      typeof params.threadId === 'string' &&
      params.threadId.length > 0 &&
      params.threadId === view.threadId,
  )
}

function notificationReducer(
  state: ThreadViewState,
  event: AppServerNotificationEvent,
): ThreadViewState {
  const params = notificationParams<{ threadId?: unknown }>(event)
  if (
    params &&
    'threadId' in params &&
    (typeof params.threadId !== 'string' || params.threadId.length === 0)
  ) {
    return {
      ...state,
      error: 'Thread event is missing a valid thread ID',
    }
  }
  if (state.threadId === null || !isForView(event, state)) {
    return state
  }

  switch (event.method) {
    case 'turn/started': {
      const params = notificationParams<{ threadId: string; turn: Turn }>(event)
      if (
        !params ||
        !isTurn(params.turn) ||
        state.turn.phase === 'Idle' ||
        state.turn.phase === 'Terminal'
      ) {
        return state
      }
      return turnStartedReducer(state, params.turn)
    }
    case 'turn/completed': {
      const params = notificationParams<{ threadId: string; turn: Turn }>(event)
      return params && isTurn(params.turn) ? turnCompletedReducer(state, params.turn) : state
    }
    case 'item/started':
    case 'item/completed': {
      const params = notificationParams<{ threadId: string; turnId: string; item: ThreadItem }>(
        event,
      )
      if (!params || !isThreadItem(params.item) || !canApplyTurnItem(state, params.turnId)) {
        return state
      }
      return {
        ...state,
        entries: upsertItem(state.entries, params.item, state.entries.length),
      }
    }
    case 'item/agentMessage/delta': {
      const params = notificationParams<{
        threadId: string
        turnId: string
        itemId: string
        delta: string
      }>(event)
      if (
        !params ||
        typeof params.itemId !== 'string' ||
        typeof params.delta !== 'string' ||
        typeof params.turnId !== 'string' ||
        !canApplyTurnItem(state, params.turnId)
      ) {
        return state
      }
      return {
        ...state,
        entries: updateItem(state.entries, params.itemId, (item) =>
          item.type === 'agentMessage' ? { ...item, text: item.text + params.delta } : item,
        ),
      }
    }
    case 'item/plan/delta': {
      const params = notificationParams<{
        threadId: string
        turnId: string
        itemId: string
        delta: string
      }>(event)
      if (
        !params ||
        typeof params.itemId !== 'string' ||
        typeof params.delta !== 'string' ||
        typeof params.turnId !== 'string' ||
        !canApplyTurnItem(state, params.turnId)
      ) {
        return state
      }
      return {
        ...state,
        entries: updateItem(state.entries, params.itemId, (item) =>
          item.type === 'plan' ? { ...item, text: item.text + params.delta } : item,
        ),
      }
    }
    case 'item/reasoning/textDelta': {
      const params = notificationParams<{
        threadId: string
        turnId: string
        itemId: string
        delta: string
        contentIndex: number
      }>(event)
      if (
        !params ||
        typeof params.itemId !== 'string' ||
        typeof params.delta !== 'string' ||
        typeof params.turnId !== 'string' ||
        typeof params.contentIndex !== 'number' ||
        !canApplyTurnItem(state, params.turnId)
      ) {
        return state
      }
      return {
        ...state,
        entries: updateItem(state.entries, params.itemId, (item) => {
          if (item.type !== 'reasoning') {
            return item
          }
          const content = [...item.content]
          while (content.length <= params.contentIndex) {
            content.push('')
          }
          content[params.contentIndex] = (content[params.contentIndex] ?? '') + params.delta
          return { ...item, content }
        }),
      }
    }
    case 'item/commandExecution/outputDelta':
    case 'item/fileChange/outputDelta': {
      const params = notificationParams<{
        threadId: string
        turnId: string
        itemId: string
        delta: string
      }>(event)
      if (
        !params ||
        typeof params.itemId !== 'string' ||
        typeof params.delta !== 'string' ||
        typeof params.turnId !== 'string' ||
        !canApplyTurnItem(state, params.turnId)
      ) {
        return state
      }
      return {
        ...state,
        entries: updateItem(state.entries, params.itemId, (item) => {
          if (item.type === 'commandExecution') {
            return { ...item, aggregatedOutput: (item.aggregatedOutput ?? '') + params.delta }
          }
          return item
        }),
      }
    }
    case 'error': {
      const params = notificationParams<{
        threadId: string
        turnId: string
        error: { message: string }
        willRetry: boolean
      }>(event)
      if (
        !params ||
        typeof params.turnId !== 'string' ||
        params.turnId.length === 0 ||
        !canApplyTurnItem(state, params.turnId) ||
        (state.turn.phase === 'Terminal' && !params.willRetry) ||
        !isRecord(params.error) ||
        typeof params.error.message !== 'string' ||
        typeof params.willRetry !== 'boolean'
      ) {
        return state
      }
      const id = `error-${params.turnId}-${state.entries.length}`
      const nextEntries = upsertItem(
        state.entries,
        {
          type: 'error',
          id,
          message: params.error.message,
          willRetry: params.willRetry,
        },
        state.entries.length,
      )
      const terminal = state.turn.phase !== 'Terminal' && !params.willRetry
      return {
        ...state,
        entries: nextEntries,
        turn: terminal
          ? {
              ...state.turn,
              phase: 'Terminal',
              terminalStatus: 'failed',
              error: params.error.message,
            }
          : { ...state.turn, error: params.error.message },
      }
    }
    case 'account/updated':
    case 'account/gatewayOAuth/changed': {
      return {
        ...state,
        notice: 'Account authentication changed. Some requests may require re-login.',
      }
    }
    case 'modelProvider/authRecoveryStarted':
    case 'modelProvider/authRecoveryCompleted': {
      const params = notificationParams<{ threadId: string; message: string }>(event)
      if (
        !params ||
        typeof params.message !== 'string' ||
        (params.threadId !== undefined && params.threadId !== state.threadId)
      ) {
        return state
      }
      return {
        ...state,
        notice: params.message,
        error: event.method === 'modelProvider/authRecoveryStarted' ? params.message : state.error,
      }
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
    (view.turn.phase === 'Idle' ||
      (view.turn.phase === 'Terminal' && view.turn.terminalStatus !== 'interrupted'))
  )
}
