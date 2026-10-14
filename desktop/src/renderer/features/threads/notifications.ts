import type { V2ThreadItem as ThreadItem, V2Turn as Turn } from '@codex/app-server-protocol'
import type { AppServerNotificationEvent } from '../../../shared/appServer'
import { isThreadItem, isTurn } from './protocol'
import type {
  ConversationEntry,
  ConversationItem,
  TerminalTurnStatus,
  ThreadViewState,
} from './state'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function notificationParams<T>(event: AppServerNotificationEvent): T | null {
  return isRecord(event.params) ? (event.params as T) : null
}

export function appendEntries(
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

export function terminalFromTurn(turn: Turn): TerminalTurnStatus {
  if (turn.status === 'interrupted' || turn.status === 'failed') {
    return turn.status
  }
  return 'completed'
}

export function turnStartedReducer(view: ThreadViewState, turn: Turn): ThreadViewState {
  if (view.turn.phase === 'Terminal') {
    if (view.activeTurnId !== null && view.activeTurnId !== turn.id) {
      return view
    }
    return {
      ...view,
      turn: {
        phase: 'Running',
        turn,
        terminalStatus: null,
        error: turn.error?.message ?? null,
      },
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

export function turnCompletedReducer(view: ThreadViewState, turn: Turn): ThreadViewState {
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

function canApplyTurnItem(view: ThreadViewState, turnId: string): boolean {
  if (view.activeTurnId === null) {
    return view.turn.phase === 'Starting' || view.turn.phase === 'Running'
  }
  return turnId === view.activeTurnId
}

const globalNotificationMethods = new Set([
  'account/updated',
  'account/gatewayOAuth/changed',
  'modelProvider/authRecoveryStarted',
  'modelProvider/authRecoveryCompleted',
])
const threadScopedNotificationMethods = new Set([
  'turn/started',
  'turn/completed',
  'item/started',
  'item/completed',
  'item/agentMessage/delta',
  'item/plan/delta',
  'item/reasoning/textDelta',
  'item/commandExecution/outputDelta',
  'item/fileChange/outputDelta',
  'error',
])

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

export function notificationReducer(
  state: ThreadViewState,
  event: AppServerNotificationEvent,
): ThreadViewState {
  const params = notificationParams<{ threadId?: unknown }>(event)
  if (threadScopedNotificationMethods.has(event.method)) {
    const threadId = params?.threadId
    if (typeof threadId !== 'string' || threadId.length === 0) {
      return {
        ...state,
        error: 'Thread event is missing a valid thread ID',
      }
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
