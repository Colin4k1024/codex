import { useCallback, useEffect, useReducer, useRef } from 'react'
import type { AppServerStatus } from '../../../shared/appServer'
import type { CodexDesktopBridge } from '../../../shared/types'
import { createThreadBridgeClient, getThreadErrorMessage, THREAD_PAGE_LIMIT } from './protocol'
import { persistDurableThreadId, readDurableThreadId } from './durableSelection'
import {
  canStartTurn,
  initialThreadWorkspaceState,
  threadWorkspaceReducer,
  type ThreadWorkspaceState,
} from './state'

export interface ThreadWorkspaceController {
  state: ThreadWorkspaceState
  loadFirstPage: () => Promise<void>
  loadNextPage: () => Promise<void>
  createThread: () => Promise<void>
  resumeThread: (threadId: string) => Promise<void>
  updateFilters: (searchTerm: string, archivedOnly: boolean) => void
  startTurn: (text: string) => Promise<void>
}

export interface UseThreadWorkspaceOptions {
  bridge: CodexDesktopBridge
  appServerStatus: AppServerStatus | null
}

export function useThreadWorkspace({
  bridge,
  appServerStatus,
}: UseThreadWorkspaceOptions): ThreadWorkspaceController {
  const [state, dispatch] = useReducer(threadWorkspaceReducer, initialThreadWorkspaceState)
  const client = useRef(createThreadBridgeClient(bridge))
  const stateRef = useRef(state)
  const listRequestSeqRef = useRef(0)
  const viewRequestSeqRef = useRef(0)
  const listInFlightRef = useRef(false)
  const turnInFlightRef = useRef(false)
  const readyRef = useRef(false)
  const wasReadyRef = useRef<boolean | null>(null)

  useEffect(() => {
    stateRef.current = state
  }, [state])

  const loadFirstPage = useCallback(async () => {
    if (!readyRef.current || listInFlightRef.current) {
      return
    }
    listInFlightRef.current = true
    const requestId = ++listRequestSeqRef.current
    dispatch({ type: 'listLoadStarted', requestId, cursor: null, first: true })

    try {
      const response = await client.current.listThreads({
        cursor: null,
        limit: THREAD_PAGE_LIMIT,
        searchTerm: stateRef.current.list.searchTerm.trim() || null,
        archived: stateRef.current.list.archivedOnly,
      })
      dispatch({
        type: 'listLoadSucceeded',
        requestId,
        cursor: null,
        first: true,
        response,
      })
    } catch (error) {
      dispatch({
        type: 'listLoadFailed',
        requestId,
        message: getThreadErrorMessage(error, 'Unable to load Codex threads'),
      })
    } finally {
      listInFlightRef.current = false
    }
  }, [])

  const loadNextPage = useCallback(async () => {
    const list = stateRef.current.list
    if (
      !readyRef.current ||
      listInFlightRef.current ||
      list.phase !== 'Ready' ||
      list.loadingMore ||
      list.nextCursor === null
    ) {
      return
    }

    listInFlightRef.current = true
    const cursor = list.nextCursor
    const requestId = ++listRequestSeqRef.current
    dispatch({ type: 'listLoadStarted', requestId, cursor, first: false })

    try {
      const response = await client.current.listThreads({
        cursor,
        limit: THREAD_PAGE_LIMIT,
        searchTerm: stateRef.current.list.searchTerm.trim() || null,
        archived: stateRef.current.list.archivedOnly,
      })
      dispatch({ type: 'listLoadSucceeded', requestId, cursor, first: false, response })
    } catch (error) {
      dispatch({
        type: 'listLoadFailed',
        requestId,
        message: getThreadErrorMessage(error, 'Unable to load the next thread page'),
      })
    } finally {
      listInFlightRef.current = false
    }
  }, [])

  const readThread = useCallback(async (threadId: string) => {
    const requestId = ++viewRequestSeqRef.current
    dispatch({ type: 'viewLoadStarted', requestId, threadId, operation: 'read' })

    try {
      const response = await client.current.readThread({ threadId, includeTurns: true })
      dispatch({
        type: 'viewLoadSucceeded',
        requestId,
        expectedThreadId: threadId,
        thread: response.thread,
        operation: 'read',
      })
    } catch (error) {
      dispatch({
        type: 'viewLoadFailed',
        requestId,
        message: getThreadErrorMessage(error, 'Unable to restore the selected thread'),
      })
    }
  }, [])

  const createThread = useCallback(async () => {
    if (!readyRef.current || stateRef.current.list.phase !== 'Ready') {
      return
    }
    const requestId = ++viewRequestSeqRef.current
    dispatch({ type: 'viewLoadStarted', requestId, threadId: null, operation: 'create' })

    try {
      const response = await client.current.startThread({ sessionStartSource: 'clear' })
      dispatch({
        type: 'viewLoadSucceeded',
        requestId,
        expectedThreadId: null,
        thread: response.thread,
        operation: 'create',
      })
      dispatch({ type: 'threadCreated', thread: response.thread })
    } catch (error) {
      dispatch({
        type: 'viewLoadFailed',
        requestId,
        message: getThreadErrorMessage(error, 'Unable to start a new Codex thread'),
      })
    }
  }, [])

  const resumeThread = useCallback(async (threadId: string) => {
    if (!readyRef.current) {
      return
    }
    const requestId = ++viewRequestSeqRef.current
    dispatch({ type: 'viewLoadStarted', requestId, threadId, operation: 'resume' })

    try {
      const response = await client.current.resumeThread({
        threadId,
        excludeTurns: false,
      })
      dispatch({
        type: 'viewLoadSucceeded',
        requestId,
        expectedThreadId: threadId,
        thread: response.thread,
        operation: 'resume',
      })
    } catch (error) {
      dispatch({
        type: 'viewLoadFailed',
        requestId,
        message: getThreadErrorMessage(error, 'Unable to resume the selected thread'),
      })
    }
  }, [])

  const updateFilters = useCallback((searchTerm: string, archivedOnly: boolean): void => {
    const trimmed = searchTerm.trim()
    if (
      stateRef.current.list.searchTerm === trimmed &&
      stateRef.current.list.archivedOnly === archivedOnly
    ) {
      return
    }
    dispatch({ type: 'filterChanged', searchTerm: trimmed, archivedOnly })
  }, [])

  const startTurn = useCallback(async (text: string) => {
    const view = stateRef.current.view
    if (!readyRef.current || turnInFlightRef.current || !canStartTurn(view)) {
      return
    }

    const trimmed = text.trim()
    if (!trimmed) {
      return
    }

    turnInFlightRef.current = true
    const requestId = ++viewRequestSeqRef.current
    dispatch({ type: 'turnStartStarted', requestId })

    try {
      const response = await client.current.startTurn({
        threadId: view.threadId ?? '',
        input: [{ type: 'text', text: trimmed, text_elements: [] }],
      })
      dispatch({ type: 'turnStartSucceeded', requestId, turn: response.turn })
    } catch (error) {
      dispatch({
        type: 'turnStartFailed',
        requestId,
        message: getThreadErrorMessage(error, 'Unable to start the Codex turn'),
      })
    } finally {
      turnInFlightRef.current = false
    }
  }, [])

  useEffect(() => {
    const unsubscribe = bridge.onAppServerNotification((event) => {
      dispatch({ type: 'notificationReceived', event })
    })
    return unsubscribe
  }, [bridge])

  useEffect(() => {
    const ready = appServerStatus?.state === 'Ready'
    readyRef.current = ready

    if (!ready) {
      if (wasReadyRef.current === true) {
        dispatch({
          type: 'connectionLost',
          message:
            appServerStatus?.lastError?.message ??
            'App server connection lost; the current operation was interrupted',
        })
      }
      wasReadyRef.current = false
      return
    }

    const isReconnect = wasReadyRef.current === false
    wasReadyRef.current = true
    const durableThreadId = stateRef.current.view.threadId ?? readDurableThreadId()

    if (isReconnect) {
      dispatch({ type: 'durableReset' })
    }

    void loadFirstPage()
    if (durableThreadId !== null) {
      void readThread(durableThreadId)
    }
  }, [appServerStatus?.state, appServerStatus?.lastError?.message, loadFirstPage, readThread])

  useEffect(() => {
    if (
      state.view.phase === 'Ready' &&
      state.view.thread !== null &&
      state.view.threadId !== null
    ) {
      persistDurableThreadId(state.view.threadId)
    }
  }, [state.view.phase, state.view.thread, state.view.threadId])

  useEffect(() => {
    if (readyRef.current && state.list.phase === 'NotLoaded' && !listInFlightRef.current) {
      void loadFirstPage()
    }
  }, [state.list.phase, state.list.searchTerm, state.list.archivedOnly, loadFirstPage])

  return {
    state,
    loadFirstPage,
    loadNextPage,
    createThread,
    resumeThread,
    updateFilters,
    startTurn,
  }
}
