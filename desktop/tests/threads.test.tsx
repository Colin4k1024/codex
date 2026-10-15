import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import * as axe from 'axe-core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { App } from '../src/renderer/App'
import { ConversationStream } from '../src/renderer/features/threads/ConversationStream'
import { initialThreadWorkspaceState } from '../src/renderer/features/threads/state'
import type { AppServerNotificationEvent, AppServerStatus } from '../src/shared/appServer'
import type { CodexDesktopBridge, DesktopInfo } from '../src/shared/types'

interface MockThread {
  id: string
  preview: string
  name?: string
  turns?: Record<string, unknown>[]
}

interface MockTurn {
  id: string
  status?: 'inProgress' | 'completed' | 'failed' | 'interrupted'
  items?: Record<string, unknown>[]
}

function readyStatus(): AppServerStatus {
  return {
    state: 'Ready',
    pid: 1234,
    pendingRequests: 0,
    queuedMessages: 0,
    reconnectAttempt: null,
    lastError: null,
  }
}

function disconnectedStatus(state: AppServerStatus['state']): AppServerStatus {
  return {
    state,
    pid: null,
    pendingRequests: 0,
    queuedMessages: 0,
    reconnectAttempt: state === 'Reconnecting' ? 1 : null,
    lastError: {
      code: null,
      message: 'app-server connection closed',
      kind: 'connectionClosed',
      retryable: true,
    },
  }
}

function mockThread({ id, preview, name, turns = [] }: MockThread): Record<string, unknown> {
  return {
    id,
    sessionId: `session-${id}`,
    forkedFromId: null,
    parentThreadId: null,
    preview,
    ephemeral: false,
    section: null,
    sectionEnteredAt: null,
    projectId: null,
    historyMode: 'paginated',
    modelProvider: 'openai',
    model: 'gpt-5',
    reasoningEffort: null,
    createdAt: 1,
    updatedAt: 2,
    recencyAt: 2,
    status: turns.some((turn) => turn.status === 'inProgress')
      ? { type: 'active', activeFlags: [] }
      : { type: 'idle' },
    path: null,
    cwd: '/tmp',
    cliVersion: 'test',
    originator: null,
    source: 'appServer',
    threadSource: null,
    agentNickname: null,
    agentRole: null,
    gitInfo: null,
    name: name ?? null,
    turns,
  }
}

function mockTurn({ id, status = 'inProgress', items = [] }: MockTurn): Record<string, unknown> {
  return {
    id,
    rootTurnId: id,
    items,
    itemsView: 'full',
    status,
    error: null,
    startedAt: 1,
    completedAt: status === 'inProgress' ? null : 2,
    durationMs: status === 'inProgress' ? null : 1,
  }
}

function userItem(id: string, text: string): Record<string, unknown> {
  return {
    type: 'userMessage',
    id,
    clientId: null,
    content: [{ type: 'text', text, text_elements: [] }],
  }
}

function assistantItem(id: string, text: string): Record<string, unknown> {
  return {
    type: 'agentMessage',
    id,
    text,
    phase: null,
    memoryCitation: null,
    delivery: null,
    questions: null,
  }
}

function reasoningItem(id: string, content: string[] = []): Record<string, unknown> {
  return { type: 'reasoning', id, summary: [], content }
}

function toolItem(id: string, output = ''): Record<string, unknown> {
  return {
    type: 'commandExecution',
    id,
    pluginId: null,
    scriptPath: null,
    command: 'echo hello',
    cwd: '/tmp',
    processId: null,
    source: 'shell',
    status: 'completed',
    commandActions: [],
    aggregatedOutput: output,
    exitCode: 0,
    durationMs: 1,
  }
}

function listResponse(
  threads: Record<string, unknown>[],
  nextCursor: string | null = null,
): Record<string, unknown> {
  return { data: threads, nextCursor, backwardsCursor: null }
}

function threadEnvelope(thread: Record<string, unknown>): Record<string, unknown> {
  return {
    thread,
    model: 'gpt-5',
    modelProvider: 'openai',
    serviceTier: null,
    disabledPluginIds: [],
    cwd: '/tmp',
    instructionSources: [],
    approvalPolicy: 'untrusted',
    approvalsReviewer: 'none',
    sandbox: {},
    reasoningEffort: null,
  }
}

function resumeEnvelope(thread: Record<string, unknown>): Record<string, unknown> {
  return {
    ...threadEnvelope(thread),
    collaborationMode: null,
    turnsBackwardsCursor: null,
    itemsBackwardsCursor: null,
  }
}

function createMockBridge(request: (method: string, params?: unknown) => Promise<unknown>) {
  const notificationListeners = new Set<(event: AppServerNotificationEvent) => void>()
  const statusListeners = new Set<(status: AppServerStatus) => void>()
  const requestMock = vi.fn(request)

  const bridge: CodexDesktopBridge = {
    getDesktopInfo: vi.fn(
      async (): Promise<DesktopInfo> => ({
        platform: 'darwin',
        electronVersion: '33.2.0',
        nodeVersion: '20.18.1',
        chromeVersion: '130.0.6723.44',
      }),
    ),
    startAppServer: vi.fn(async () => readyStatus()),
    stopAppServer: vi.fn(async () => disconnectedStatus('Disconnected')),
    getAppServerStatus: vi.fn(async () => readyStatus()),
    requestAppServer: requestMock,
    notifyAppServer: vi.fn(),
    respondAppServerRequest: vi.fn(async () => undefined),
    onAppServerStatusChanged: vi.fn((listener: (status: AppServerStatus) => void) => {
      statusListeners.add(listener)
      return () => statusListeners.delete(listener)
    }),
    onAppServerNotification: vi.fn((listener: (event: AppServerNotificationEvent) => void) => {
      notificationListeners.add(listener)
      return () => notificationListeners.delete(listener)
    }),
  }

  function emitNotification(method: string, params: unknown): void {
    act(() => {
      for (const listener of notificationListeners) {
        listener({ method, params })
      }
    })
  }

  function emitStatus(status: AppServerStatus): void {
    act(() => {
      for (const listener of statusListeners) {
        listener(status)
      }
    })
  }

  return { bridge, request: requestMock, emitNotification, emitStatus }
}

function renderApp(bridge: CodexDesktopBridge) {
  vi.stubGlobal('codexDesktop', bridge)
  return render(<App />)
}

beforeEach(() => {
  vi.unstubAllGlobals()
  window.localStorage.clear()
})

describe('thread workspace', () => {
  it('loads strict cursor pages without dropping or duplicating threads', async () => {
    const first = mockThread({ id: 'thread-1', preview: 'First thread', name: 'Thread one' })
    const second = mockThread({ id: 'thread-2', preview: 'Second thread', name: 'Thread two' })
    const { bridge, request } = createMockBridge(async () => listResponse([]))
    request
      .mockResolvedValueOnce(listResponse([first], 'cursor-1'))
      .mockResolvedValueOnce(listResponse([second], null))

    renderApp(bridge)
    expect(await screen.findByText('Thread one')).toBeInTheDocument()
    expect(request).toHaveBeenNthCalledWith(1, 'thread/list', {
      cursor: null,
      limit: 50,
      searchTerm: null,
      archived: false,
    })

    fireEvent.click(screen.getByRole('button', { name: 'Load more threads' }))
    expect(await screen.findByText('Thread two')).toBeInTheDocument()
    expect(request).toHaveBeenNthCalledWith(2, 'thread/list', {
      cursor: 'cursor-1',
      limit: 50,
      searchTerm: null,
      archived: false,
    })
    expect(screen.getAllByRole('button', { name: /Thread one/ })).toHaveLength(1)
    expect(screen.queryByRole('button', { name: 'Load more threads' })).not.toBeInTheDocument()
  })

  it('does not render business items before thread/start resolves', async () => {
    const { bridge, request } = createMockBridge(async () => listResponse([]))
    request.mockResolvedValueOnce(listResponse([]))
    let resolveStart: (value: Record<string, unknown>) => void = () => undefined
    request.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveStart = resolve
        }),
    )

    renderApp(bridge)
    await waitFor(() => expect(request).toHaveBeenCalledTimes(1))
    fireEvent.click(screen.getByRole('button', { name: 'New thread' }))

    expect(await screen.findByText('Loading thread history')).toBeInTheDocument()
    expect(screen.queryByText('Assistant')).not.toBeInTheDocument()
    expect(screen.getByText('No thread selected')).toBeInTheDocument()

    const thread = mockThread({
      id: 'new-thread',
      preview: 'New thread',
      name: 'New thread',
      turns: [mockTurn({ id: 'turn-1', items: [assistantItem('assistant-1', 'Hello')] })],
    })
    await act(async () => {
      resolveStart(threadEnvelope(thread))
    })

    expect(await screen.findByRole('heading', { name: 'New thread', level: 2 })).toBeInTheDocument()
    expect(screen.getByText('Hello')).toBeInTheDocument()
    expect(request).toHaveBeenNthCalledWith(2, 'thread/start', {
      sessionStartSource: 'clear',
    })
  })

  it('resumes a thread and renders durable history items', async () => {
    const existing = mockThread({ id: 'thread-1', preview: 'Existing', name: 'Existing thread' })
    const { bridge, request } = createMockBridge(async () => listResponse([]))
    request.mockResolvedValueOnce(listResponse([existing], null)).mockResolvedValueOnce(
      resumeEnvelope(
        mockThread({
          id: 'thread-1',
          preview: 'Existing',
          name: 'Existing thread',
          turns: [
            mockTurn({
              id: 'turn-1',
              status: 'completed',
              items: [assistantItem('assistant-1', 'Existing assistant')],
            }),
            mockTurn({
              id: 'turn-2',
              status: 'completed',
              items: [reasoningItem('reasoning-1', ['Existing reasoning'])],
            }),
          ],
        }),
      ),
    )

    renderApp(bridge)
    fireEvent.click(await screen.findByRole('button', { name: /Existing thread/ }))
    expect(await screen.findByText('Existing assistant')).toBeInTheDocument()
    expect(screen.getByText('Existing reasoning')).toBeInTheDocument()
    expect(request).toHaveBeenNthCalledWith(2, 'thread/resume', {
      threadId: 'thread-1',
      excludeTurns: false,
    })
    expect(screen.getByText('Turn: completed')).toBeInTheDocument()
  })

  it('starts a turn and processes ordered incremental items', async () => {
    const { bridge, request, emitNotification } = createMockBridge(async () => listResponse([]))
    request
      .mockResolvedValueOnce(listResponse([]))
      .mockResolvedValueOnce(
        threadEnvelope(mockThread({ id: 'thread-1', preview: 'New', name: 'Conversation thread' })),
      )
      .mockResolvedValueOnce({
        turn: mockTurn({
          id: 'turn-1',
          status: 'inProgress',
          items: [userItem('user-1', 'Say hello')],
        }),
      })

    renderApp(bridge)
    await screen.findByText('No threads are available yet.')
    fireEvent.click(screen.getByRole('button', { name: 'New thread' }))
    expect(
      await screen.findByRole('heading', { name: 'Conversation thread', level: 2 }),
    ).toBeInTheDocument()

    fireEvent.change(screen.getByLabelText('Message'), { target: { value: 'Say hello' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    expect(await screen.findByText('Say hello')).toBeInTheDocument()
    expect(request).toHaveBeenNthCalledWith(3, 'turn/start', {
      threadId: 'thread-1',
      input: [{ type: 'text', text: 'Say hello', text_elements: [] }],
    })

    emitNotification('item/started', {
      threadId: 'thread-1',
      turnId: 'turn-1',
      item: assistantItem('assistant-1', ''),
    })
    emitNotification('item/agentMessage/delta', {
      threadId: 'thread-1',
      turnId: 'turn-1',
      itemId: 'assistant-1',
      delta: 'Hello',
    })
    emitNotification('item/started', {
      threadId: 'thread-1',
      turnId: 'turn-1',
      item: reasoningItem('reasoning-1'),
    })
    emitNotification('item/reasoning/textDelta', {
      threadId: 'thread-1',
      turnId: 'turn-1',
      itemId: 'reasoning-1',
      delta: ' thinking',
      contentIndex: 0,
    })
    emitNotification('item/started', {
      threadId: 'thread-1',
      turnId: 'turn-1',
      item: toolItem('tool-1'),
    })
    emitNotification('item/commandExecution/outputDelta', {
      threadId: 'thread-1',
      turnId: 'turn-1',
      itemId: 'tool-1',
      delta: 'hello\n',
    })
    emitNotification('turn/completed', {
      threadId: 'thread-1',
      turn: mockTurn({ id: 'turn-1', status: 'completed' }),
    })

    expect(await screen.findByText('Turn: completed')).toBeInTheDocument()
    expect(screen.getByText('Hello')).toBeInTheDocument()
    expect(screen.getByText(/thinking/)).toBeInTheDocument()
    expect(screen.getAllByText(/hello/).length).toBeGreaterThan(0)
    const log = screen.getByRole('log', { name: 'Conversation stream' })
    expect(log.textContent?.indexOf('Say hello')).toBeLessThan(
      log.textContent?.indexOf('Hello') ?? -1,
    )
    expect(screen.getByLabelText('Message')).toBeEnabled()
  })

  it('keeps terminal state when late notifications arrive', async () => {
    const { bridge, request, emitNotification } = createMockBridge(async () => listResponse([]))
    request
      .mockResolvedValueOnce(listResponse([]))
      .mockResolvedValueOnce(threadEnvelope(mockThread({ id: 'thread-1', preview: 'Thread' })))
      .mockResolvedValueOnce({ turn: mockTurn({ id: 'turn-1', status: 'inProgress' }) })

    renderApp(bridge)
    await screen.findByText('No threads are available yet.')
    fireEvent.click(screen.getByRole('button', { name: 'New thread' }))
    await waitFor(() => expect(request).toHaveBeenCalledTimes(2))
    fireEvent.change(screen.getByLabelText('Message'), { target: { value: 'Hello' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))
    await waitFor(() => expect(screen.getByText('Turn: running')).toBeInTheDocument())

    emitNotification('turn/completed', {
      threadId: 'thread-1',
      turn: mockTurn({ id: 'turn-1', status: 'completed' }),
    })
    expect(screen.getByText('Turn: completed')).toBeInTheDocument()

    emitNotification('turn/started', {
      threadId: 'thread-1',
      turn: mockTurn({ id: 'turn-1', status: 'inProgress' }),
    })
    emitNotification('turn/completed', {
      threadId: 'thread-1',
      turn: mockTurn({ id: 'turn-1', status: 'failed' }),
    })

    expect(screen.getByText('Turn: completed')).toBeInTheDocument()
    expect(screen.getByLabelText('Message')).toBeEnabled()
  })

  it('shows errors for missing threads, invalid cursors, duplicate pages, and oversized responses', async () => {
    const thread = mockThread({ id: 'thread-1', preview: 'Thread', name: 'Thread one' })
    const { bridge, request } = createMockBridge(async () => listResponse([]))
    request
      .mockResolvedValueOnce(listResponse([thread], null))
      .mockRejectedValueOnce(new Error('thread not found'))

    const firstRender = renderApp(bridge)
    fireEvent.click(await screen.findByRole('button', { name: /Thread one/ }))
    expect(await screen.findByText('thread not found')).toBeInTheDocument()
    expect(screen.getByText('No thread selected')).toBeInTheDocument()
    firstRender.unmount()

    request.mockReset()
    request
      .mockResolvedValueOnce(listResponse([thread], 'cursor-1'))
      .mockRejectedValueOnce(new Error('invalid cursor'))
    const cursorBridge = createMockBridge(request)
    vi.stubGlobal('codexDesktop', cursorBridge.bridge)
    const { unmount } = render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Load more threads' }))
    expect(await screen.findByText('invalid cursor')).toBeInTheDocument()
    unmount()

    request.mockReset()
    const thread2 = mockThread({ id: 'thread-2', preview: 'Thread 2', name: 'Thread two' })
    request
      .mockResolvedValueOnce(listResponse([thread], 'cursor-1'))
      .mockResolvedValueOnce(listResponse([thread, thread2], null))
    const duplicateBridge = createMockBridge(request)
    vi.stubGlobal('codexDesktop', duplicateBridge.bridge)
    render(<App />)
    fireEvent.click(await screen.findByRole('button', { name: 'Load more threads' }))
    expect(await screen.findByRole('button', { name: /Thread two/ })).toBeInTheDocument()
    expect(screen.queryByText(/duplicate thread/)).not.toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: /Thread one/ })).toHaveLength(1)

    request.mockReset()
    request.mockResolvedValueOnce({
      data: [],
      nextCursor: null,
      backwardsCursor: null,
      padding: 'x'.repeat(8 * 1024 * 1024 + 1),
    })
    const oversizedBridge = createMockBridge(request)
    vi.stubGlobal('codexDesktop', oversizedBridge.bridge)
    render(<App />)
    expect(await screen.findByText(/exceeds the 8388608 byte limit/)).toBeInTheDocument()
  })

  it('handles server overload and auth-change notifications', async () => {
    const { bridge, request, emitNotification } = createMockBridge(async () => listResponse([]))
    request
      .mockResolvedValueOnce(listResponse([]))
      .mockResolvedValueOnce(threadEnvelope(mockThread({ id: 'thread-1', preview: 'Thread' })))
      .mockRejectedValueOnce(new Error('server overloaded'))

    renderApp(bridge)
    await screen.findByText('No threads are available yet.')
    fireEvent.click(screen.getByRole('button', { name: 'New thread' }))
    await waitFor(() => expect(request).toHaveBeenCalledTimes(2))
    fireEvent.change(screen.getByLabelText('Message'), { target: { value: 'Hello' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))

    expect(await screen.findByText('Turn: Terminal — server overloaded')).toBeInTheDocument()
    expect(screen.getByLabelText('Message')).toBeEnabled()

    emitNotification('account/updated', { authMode: 'chatgpt', planType: 'pro' })
    expect(
      screen.getByText('Account authentication changed. Some requests may require re-login.'),
    ).toBeInTheDocument()
  })

  it('does not hang in loading when the app server crashes', async () => {
    const { bridge, request, emitStatus } = createMockBridge(async () => listResponse([]))
    request.mockImplementationOnce(
      () =>
        new Promise(() => {
          // Intentionally unresolved: the connection event must terminate loading.
        }),
    )

    renderApp(bridge)
    expect(await screen.findByText('Loading threads')).toBeInTheDocument()
    emitStatus(disconnectedStatus('Reconnecting'))

    expect(await screen.findAllByText('app-server connection closed')).not.toHaveLength(0)
    expect(screen.queryByText('Loading threads')).not.toBeInTheDocument()
    expect(screen.queryByText('Loading thread history')).not.toBeInTheDocument()
  })

  it('rebuilds list and selected thread from durable state after reconnect', async () => {
    const existing = mockThread({ id: 'thread-1', preview: 'Old', name: 'Durable thread' })
    const { bridge, request, emitStatus } = createMockBridge(async () => listResponse([]))
    request
      .mockResolvedValueOnce(listResponse([existing], null))
      .mockResolvedValueOnce(
        resumeEnvelope(
          mockThread({
            id: 'thread-1',
            preview: 'Old',
            name: 'Durable thread',
            turns: [mockTurn({ id: 'turn-1', status: 'completed' })],
          }),
        ),
      )
      .mockResolvedValueOnce(listResponse([existing], null))
      .mockResolvedValueOnce({
        thread: mockThread({
          id: 'thread-1',
          preview: 'Recovered',
          name: 'Recovered thread',
          turns: [
            mockTurn({
              id: 'turn-2',
              status: 'completed',
              items: [assistantItem('assistant-2', 'Recovered from durable state')],
            }),
          ],
        }),
      })

    renderApp(bridge)
    fireEvent.click(await screen.findByRole('button', { name: /Durable thread/ }))
    await screen.findByText('Durable thread')

    emitStatus(disconnectedStatus('Reconnecting'))
    emitStatus(readyStatus())

    await waitFor(() => expect(request).toHaveBeenCalledTimes(4))
    expect(request).toHaveBeenNthCalledWith(3, 'thread/list', {
      cursor: null,
      limit: 50,
      searchTerm: null,
      archived: false,
    })
    expect(request).toHaveBeenNthCalledWith(4, 'thread/read', {
      threadId: 'thread-1',
      includeTurns: true,
    })
    expect(await screen.findByText('Recovered thread')).toBeInTheDocument()
    expect(screen.getByText('Recovered from durable state')).toBeInTheDocument()
  })
})

it('rejects thread notifications with missing or empty thread IDs', async () => {
  const { bridge, request, emitNotification } = createMockBridge(async () => listResponse([]))
  request.mockResolvedValueOnce(listResponse([])).mockResolvedValueOnce(
    threadEnvelope(
      mockThread({
        id: 'thread-1',
        preview: 'Thread',
        name: 'Thread',
        turns: [mockTurn({ id: 'turn-1', status: 'completed' })],
      }),
    ),
  )

  renderApp(bridge)
  await screen.findByText('No threads are available yet.')
  fireEvent.click(screen.getByRole('button', { name: 'New thread' }))
  await screen.findByRole('heading', { name: 'Thread', level: 2 })

  emitNotification('error', {
    threadId: '',
    turnId: 'turn-1',
    error: { message: 'Invalid thread event' },
    willRetry: false,
  })

  expect(screen.getByText('Thread event is missing a valid thread ID')).toBeInTheDocument()
  expect(screen.queryByText('Invalid thread event')).not.toBeInTheDocument()
})

it('ignores late or different-turn error notifications after a terminal turn', async () => {
  const { bridge, request, emitNotification } = createMockBridge(async () => listResponse([]))
  request.mockResolvedValueOnce(listResponse([])).mockResolvedValueOnce(
    threadEnvelope(
      mockThread({
        id: 'thread-1',
        preview: 'Thread',
        name: 'Thread',
        turns: [
          mockTurn({
            id: 'turn-1',
            status: 'completed',
            items: [assistantItem('assistant-1', 'Completed')],
          }),
        ],
      }),
    ),
  )

  renderApp(bridge)
  await screen.findByText('No threads are available yet.')
  fireEvent.click(screen.getByRole('button', { name: 'New thread' }))
  await screen.findByText('Completed')
  expect(screen.getByText('Turn: completed')).toBeInTheDocument()

  emitNotification('error', {
    threadId: 'thread-1',
    turnId: 'different-turn',
    error: { message: 'Different turn error' },
    willRetry: false,
  })
  emitNotification('error', {
    threadId: 'thread-1',
    turnId: 'turn-1',
    error: { message: 'Late turn error' },
    willRetry: false,
  })

  expect(screen.getByText('Turn: completed')).toBeInTheDocument()
  expect(screen.queryByText('Different turn error')).not.toBeInTheDocument()
  expect(screen.queryByText('Late turn error')).not.toBeInTheDocument()
})

it('disables message submission while the app server is disconnected', async () => {
  const { bridge, request, emitStatus } = createMockBridge(async () => listResponse([]))
  request.mockResolvedValueOnce(listResponse([])).mockResolvedValueOnce(
    threadEnvelope(
      mockThread({
        id: 'thread-1',
        preview: 'Thread',
        name: 'Thread',
        turns: [mockTurn({ id: 'turn-1', status: 'completed' })],
      }),
    ),
  )

  renderApp(bridge)
  await screen.findByText('No threads are available yet.')
  fireEvent.click(screen.getByRole('button', { name: 'New thread' }))
  await screen.findByText('Turn: completed')

  emitStatus(disconnectedStatus('Reconnecting'))
  expect(screen.getByLabelText('Message')).toBeDisabled()
  expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled()
})

it('rejects thread envelopes with empty thread IDs', async () => {
  const { bridge, request } = createMockBridge(async () => listResponse([]))
  request
    .mockResolvedValueOnce(listResponse([]))
    .mockResolvedValueOnce(threadEnvelope(mockThread({ id: '', preview: 'Invalid' })))

  renderApp(bridge)
  await screen.findByText('No threads are available yet.')
  fireEvent.click(screen.getByRole('button', { name: 'New thread' }))

  expect(await screen.findByText('thread/start response is invalid')).toBeInTheDocument()
})

it('disables thread creation until the first thread page is ready', async () => {
  const { bridge, request } = createMockBridge(async () => listResponse([]))
  request.mockImplementationOnce(
    () =>
      new Promise(() => {
        // Keep the first list request pending.
      }),
  )

  renderApp(bridge)
  expect(await screen.findByText('Loading threads')).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'New thread' })).toBeDisabled()
})

it('restores the selected thread after a desktop restart and keeps only its ID durable', async () => {
  const thread = mockThread({ id: 'thread-1', preview: 'Restart', name: 'Restart thread' })
  const firstBridge = createMockBridge(async () => listResponse([]))
  firstBridge.request.mockResolvedValueOnce(listResponse([thread], null)).mockResolvedValueOnce(
    resumeEnvelope(
      mockThread({
        id: 'thread-1',
        preview: 'Restart',
        name: 'Restart thread',
        turns: [
          mockTurn({
            id: 'turn-1',
            status: 'completed',
            items: [assistantItem('assistant-1', 'Before restart')],
          }),
        ],
      }),
    ),
  )

  const firstRender = renderApp(firstBridge.bridge)
  fireEvent.click(await screen.findByRole('button', { name: /Restart thread/ }))
  expect(await screen.findByText('Before restart')).toBeInTheDocument()

  expect(window.localStorage.getItem('codexDesktop.selectedThreadId')).toBe('thread-1')
  expect(window.localStorage.length).toBe(1)
  firstRender.unmount()

  const restartBridge = createMockBridge(async () => listResponse([]))
  restartBridge.request.mockResolvedValueOnce(listResponse([thread], null)).mockResolvedValueOnce({
    thread: mockThread({
      id: 'thread-1',
      preview: 'Restart',
      name: 'Restart thread',
      turns: [
        mockTurn({
          id: 'turn-2',
          status: 'completed',
          items: [
            userItem('user-2', 'After restart'),
            assistantItem('assistant-2', 'Recovered from durable app-server state'),
          ],
        }),
      ],
    }),
  })

  const secondRender = renderApp(restartBridge.bridge)
  expect(await screen.findByText('Recovered from durable app-server state')).toBeInTheDocument()
  expect(screen.getByText('After restart')).toBeInTheDocument()
  expect(restartBridge.request).toHaveBeenNthCalledWith(2, 'thread/read', {
    threadId: 'thread-1',
    includeTurns: true,
  })
  expect(window.localStorage.getItem('codexDesktop.selectedThreadId')).toBe('thread-1')
  expect(window.localStorage.length).toBe(1)

  const a11y = await axe.run(secondRender.container)
  expect(a11y.violations).toHaveLength(0)
  expect(secondRender.container).toMatchSnapshot()
})

it('applies thread filters and resets strict pagination', async () => {
  const first = mockThread({ id: 'thread-1', preview: 'Alpha', name: 'Alpha thread' })
  const second = mockThread({ id: 'thread-2', preview: 'Beta', name: 'Beta thread' })
  const archived = mockThread({ id: 'thread-3', preview: 'Beta archived', name: 'Beta archived' })
  const { bridge, request } = createMockBridge(async () => listResponse([]))
  request
    .mockResolvedValueOnce(listResponse([first], 'cursor-1'))
    .mockResolvedValueOnce(listResponse([second], null))
    .mockResolvedValueOnce(listResponse([archived], null))

  renderApp(bridge)
  expect(await screen.findByText('Alpha thread')).toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Load more threads' }))
  expect(await screen.findByText('Beta thread')).toBeInTheDocument()

  fireEvent.change(screen.getByLabelText('Search threads'), { target: { value: 'Beta' } })
  fireEvent.change(screen.getByLabelText('Thread archive filter'), {
    target: { value: 'archived' },
  })
  fireEvent.click(screen.getByRole('button', { name: 'Apply filters' }))

  expect(await screen.findByRole('button', { name: /Beta archived/ })).toBeInTheDocument()
  expect(screen.queryByRole('button', { name: /Alpha thread/ })).not.toBeInTheDocument()
  expect(screen.queryByRole('button', { name: /Beta thread/ })).not.toBeInTheDocument()
  expect(screen.queryByRole('button', { name: 'Load more threads' })).not.toBeInTheDocument()
  expect(request).toHaveBeenNthCalledWith(3, 'thread/list', {
    cursor: null,
    limit: 50,
    searchTerm: 'Beta',
    archived: true,
  })
})

it('ignores a late turn response after switching threads', async () => {
  const first = mockThread({ id: 'thread-1', preview: 'First', name: 'First thread' })
  const second = mockThread({ id: 'thread-2', preview: 'Second', name: 'Second thread' })
  const { bridge, request } = createMockBridge(async () => listResponse([]))
  request
    .mockResolvedValueOnce(listResponse([first, second], null))
    .mockResolvedValueOnce(
      resumeEnvelope(
        mockThread({
          id: 'thread-1',
          preview: 'First',
          name: 'First thread',
          turns: [mockTurn({ id: 'turn-1', status: 'completed' })],
        }),
      ),
    )
    .mockImplementationOnce(
      () =>
        new Promise(() => {
          // Keep turn/start pending while the user switches threads.
        }),
    )
    .mockResolvedValueOnce(
      resumeEnvelope(
        mockThread({
          id: 'thread-2',
          preview: 'Second',
          name: 'Second thread',
          turns: [
            mockTurn({
              id: 'turn-2',
              status: 'completed',
              items: [assistantItem('assistant-2', 'Second thread history')],
            }),
          ],
        }),
      ),
    )

  renderApp(bridge)
  fireEvent.click(await screen.findByRole('button', { name: /First thread/ }))
  await screen.findByText('First thread')

  fireEvent.change(screen.getByLabelText('Message'), { target: { value: 'Hello' } })
  fireEvent.click(screen.getByRole('button', { name: 'Send' }))
  await waitFor(() => expect(request).toHaveBeenCalledTimes(3))

  fireEvent.click(screen.getByRole('button', { name: /Second thread/ }))
  expect(await screen.findByText('Second thread history')).toBeInTheDocument()
  expect(screen.queryByText('Turn: running')).not.toBeInTheDocument()
})

it('converges when turn completion arrives before the turn/start response', async () => {
  const { bridge, request, emitNotification } = createMockBridge(async () => listResponse([]))
  request
    .mockResolvedValueOnce(listResponse([]))
    .mockResolvedValueOnce(threadEnvelope(mockThread({ id: 'thread-1', preview: 'Thread' })))
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          setTimeout(() => {
            resolve({
              turn: mockTurn({
                id: 'turn-1',
                status: 'inProgress',
                items: [userItem('user-1', 'Hello')],
              }),
            })
          }, 0)
        }),
    )

  renderApp(bridge)
  await screen.findByText('No threads are available yet.')
  fireEvent.click(screen.getByRole('button', { name: 'New thread' }))
  await screen.findByRole('heading', { name: 'No thread selected', level: 2 })

  fireEvent.change(screen.getByLabelText('Message'), { target: { value: 'Hello' } })
  fireEvent.click(screen.getByRole('button', { name: 'Send' }))

  emitNotification('turn/completed', {
    threadId: 'thread-1',
    turn: mockTurn({ id: 'turn-1', status: 'completed' }),
  })
  expect(await screen.findByText('Turn: completed')).toBeInTheDocument()

  await waitFor(() => expect(screen.getByText('Turn: running')).toBeInTheDocument())
  expect(screen.getByLabelText('Message')).toBeDisabled()

  emitNotification('turn/completed', {
    threadId: 'thread-1',
    turn: mockTurn({ id: 'turn-1', status: 'completed' }),
  })
  expect(await screen.findByText('Turn: completed')).toBeInTheDocument()
  expect(screen.getByLabelText('Message')).toBeEnabled()
})

it('rejects thread-scoped notifications with omitted or non-record parameters', async () => {
  const { bridge, request, emitNotification } = createMockBridge(async () => listResponse([]))
  request
    .mockResolvedValueOnce(listResponse([]))
    .mockResolvedValueOnce(threadEnvelope(mockThread({ id: 'thread-1', preview: 'Thread' })))

  renderApp(bridge)
  await screen.findByText('No threads are available yet.')
  fireEvent.click(screen.getByRole('button', { name: 'New thread' }))
  await screen.findByRole('heading', { name: 'No thread selected', level: 2 })

  emitNotification('error', { turnId: 'turn-1' })
  expect(screen.getByText('Thread event is missing a valid thread ID')).toBeInTheDocument()

  emitNotification('turn/completed', null)
  expect(screen.getByText('Thread event is missing a valid thread ID')).toBeInTheDocument()
})

it('allows ordinary input after an interrupted terminal turn', async () => {
  const { bridge, request } = createMockBridge(async () => listResponse([]))
  request
    .mockResolvedValueOnce(listResponse([]))
    .mockResolvedValueOnce(
      threadEnvelope(
        mockThread({
          id: 'thread-1',
          preview: 'Thread',
          turns: [mockTurn({ id: 'turn-1', status: 'interrupted' })],
        }),
      ),
    )
    .mockResolvedValueOnce({ turn: mockTurn({ id: 'turn-2', status: 'inProgress' }) })

  renderApp(bridge)
  await screen.findByText('No threads are available yet.')
  fireEvent.click(screen.getByRole('button', { name: 'New thread' }))
  expect(await screen.findByText('Turn: interrupted')).toBeInTheDocument()

  fireEvent.change(screen.getByLabelText('Message'), { target: { value: 'Continue' } })
  fireEvent.click(screen.getByRole('button', { name: 'Send' }))

  await waitFor(() =>
    expect(request).toHaveBeenNthCalledWith(3, 'turn/start', {
      threadId: 'thread-1',
      input: [{ type: 'text', text: 'Continue', text_elements: [] }],
    }),
  )
  expect(await screen.findByText('Turn: running')).toBeInTheDocument()
})

it('rejects malformed rendered item fields', async () => {
  const { bridge, request } = createMockBridge(async () => listResponse([]))
  const malformedThread = mockThread({
    id: 'thread-1',
    preview: 'Thread',
    turns: [
      mockTurn({
        id: 'turn-1',
        status: 'completed',
        items: [{ type: 'agentMessage', id: 'assistant-1' }],
      }),
    ],
  })
  request
    .mockResolvedValueOnce(listResponse([]))
    .mockResolvedValueOnce(threadEnvelope(malformedThread))

  renderApp(bridge)
  await screen.findByText('No threads are available yet.')
  fireEvent.click(screen.getByRole('button', { name: 'New thread' }))

  expect(await screen.findByText('thread/start response is invalid')).toBeInTheDocument()
})

it('follows incremental output near the bottom and preserves intentional upward scrolling', () => {
  const baseView = {
    ...initialThreadWorkspaceState.view,
    phase: 'Ready' as const,
    threadId: 'thread-1',
    thread: mockThread({ id: 'thread-1', preview: 'Thread', name: 'Thread' }) as never,
    entries: [
      {
        id: 'assistant-1',
        sequence: 0,
        item: assistantItem('assistant-1', 'First'),
      },
    ],
  }
  const { rerender } = render(
    <ConversationStream state={baseView as never} onSubmit={() => undefined} />,
  )
  const log = screen.getByRole('log', { name: 'Conversation stream' })
  Object.defineProperty(log, 'scrollHeight', { configurable: true, value: 100 })
  Object.defineProperty(log, 'clientHeight', { configurable: true, value: 40 })
  log.scrollTop = 50

  const appendedView = {
    ...baseView,
    entries: [
      ...baseView.entries,
      { id: 'assistant-2', sequence: 1, item: assistantItem('assistant-2', 'Second') },
    ],
  }
  rerender(<ConversationStream state={appendedView as never} onSubmit={() => undefined} />)
  expect(log.scrollTop).toBe(100)

  log.scrollTop = 0
  fireEvent.scroll(log)
  const upwardView = {
    ...baseView,
    entries: [
      ...appendedView.entries,
      { id: 'assistant-3', sequence: 2, item: assistantItem('assistant-3', 'Third') },
    ],
  }
  rerender(<ConversationStream state={upwardView as never} onSubmit={() => undefined} />)
  expect(log.scrollTop).toBe(0)
})

it('can disable the thread workspace through the rollback feature flag', async () => {
  const { bridge, request } = createMockBridge(async () => listResponse([]))
  window.history.replaceState(null, '', '?disableThreads=1')
  try {
    renderApp(bridge)
    expect(await screen.findByText('Threads are disabled')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'New thread' })).not.toBeInTheDocument()
    expect(request).not.toHaveBeenCalled()
  } finally {
    window.history.replaceState(null, '', window.location.pathname)
  }
})
