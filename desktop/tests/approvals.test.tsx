import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import * as axe from 'axe-core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { App } from '../src/renderer/App'
import { parseApprovalRequest } from '../src/renderer/features/approvals/protocol'
import type { AppServerNotificationEvent, AppServerStatus } from '../src/shared/appServer'
import type { CodexDesktopBridge, DesktopInfo } from '../src/shared/types'

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

function createMockBridge() {
  const notificationListeners = new Set<(event: AppServerNotificationEvent) => void>()
  const statusListeners = new Set<(status: AppServerStatus) => void>()
  const requestMock = vi.fn(async () => ({
    data: [],
    nextCursor: null,
    backwardsCursor: null,
  }))
  const notifyMock = vi.fn(async () => undefined)
  const respondMock = vi.fn(async () => undefined)

  const bridge: CodexDesktopBridge = {
    getDesktopInfo: vi.fn(
      async (): Promise<DesktopInfo> => ({
        platform: 'darwin',
        electronVersion: '33.2.0',
        nodeVersion: '22.23.3',
        chromeVersion: '130.0.6723.44',
      }),
    ),
    startAppServer: vi.fn(async () => readyStatus()),
    stopAppServer: vi.fn(
      async (): Promise<AppServerStatus> => ({
        state: 'Disconnected',
        pid: null,
        pendingRequests: 0,
        queuedMessages: 0,
        reconnectAttempt: null,
        lastError: null,
      }),
    ),
    getAppServerStatus: vi.fn(async () => readyStatus()),
    requestAppServer: requestMock,
    notifyAppServer: notifyMock,
    respondAppServerRequest: respondMock,
    onAppServerStatusChanged: vi.fn((listener: (status: AppServerStatus) => void) => {
      statusListeners.add(listener)
      return () => statusListeners.delete(listener)
    }),
    onAppServerNotification: vi.fn((listener: (event: AppServerNotificationEvent) => void) => {
      notificationListeners.add(listener)
      return () => notificationListeners.delete(listener)
    }),
  }

  function emitNotification(method: string, params: unknown, id?: string | number): void {
    act(() => {
      for (const listener of notificationListeners) {
        listener({ method, params, id })
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

  return {
    bridge,
    request: requestMock,
    notify: notifyMock,
    respond: respondMock,
    emitNotification,
    emitStatus,
  }
}

function renderApp(bridge: CodexDesktopBridge) {
  vi.stubGlobal('codexDesktop', bridge)
  return render(<App />)
}

async function openApprovals(): Promise<void> {
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Show approvals' }))
  })
}

const commandRequest = {
  threadId: 'thread-1',
  turnId: 'turn-1',
  itemId: 'item-1',
  startedAtMs: 1000,
  command: 'echo hello',
  cwd: '/tmp',
  reason: 'Run a test command',
}

const fileChangeRequest = {
  threadId: 'thread-1',
  turnId: 'turn-1',
  itemId: 'item-file',
  startedAtMs: 1000,
  reason: 'Write to protected directory',
  grantRoot: '/protected',
}

beforeEach(() => {
  vi.unstubAllGlobals()
  window.localStorage.clear()
})

describe('approval protocol parsing', () => {
  it('parses a command execution server request', () => {
    const request = parseApprovalRequest({
      id: 'req-command',
      method: 'item/commandExecution/requestApproval',
      params: commandRequest,
    })
    expect(request).toMatchObject({ id: 'req-command', kind: 'commandExecution' })
  })
})

describe('approvals workspace', () => {
  it('renders command execution approval and responds once with the original id', async () => {
    const { bridge, request, notify, respond, emitNotification } = createMockBridge()

    const { container } = renderApp(bridge)
    await openApprovals()
    emitNotification('item/commandExecution/requestApproval', commandRequest, 'req-command')

    expect(
      await screen.findByRole('heading', { name: 'Command execution approval', level: 2 }),
    ).toBeInTheDocument()
    expect(screen.getByText('echo hello', { selector: 'pre' })).toBeInTheDocument()
    const approveButton = screen.getByRole('button', { name: 'Approve' })
    fireEvent.click(approveButton)
    fireEvent.click(approveButton)

    await waitFor(() => expect(respond).toHaveBeenCalledTimes(1))
    expect(respond).toHaveBeenCalledWith('req-command', { decision: 'accept' })
    expect(notify).not.toHaveBeenCalled()
    expect(request).not.toHaveBeenCalledWith('command/exec', expect.anything())
    expect(await screen.findByText('Responded')).toBeInTheDocument()

    const results = await axe.run(container)
    expect(results.violations).toHaveLength(0)
  })

  it('renders file change diffs and deny does not request execution', async () => {
    const { bridge, notify, respond, emitNotification } = createMockBridge()

    renderApp(bridge)
    await openApprovals()
    emitNotification('item/fileChange/requestApproval', fileChangeRequest, 'req-file')
    emitNotification('item/fileChange/patchUpdated', {
      threadId: 'thread-1',
      turnId: 'turn-1',
      itemId: 'item-file',
      changes: [
        { path: '/protected/file.txt', kind: 'modify', diff: '--- a/file\n+++ b/file\n+hello' },
      ],
    })

    expect(
      await screen.findByRole('heading', { name: 'File change approval', level: 2 }),
    ).toBeInTheDocument()
    expect(screen.getByText('modify: /protected/file.txt')).toBeInTheDocument()
    expect(screen.getByText(/hello/)).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Deny' }))
    await waitFor(() => expect(respond).toHaveBeenCalledWith('req-file', { decision: 'decline' }))
    expect(notify).not.toHaveBeenCalled()
  })

  it('renders empty, binary, and oversized diffs safely', async () => {
    const { bridge, emitNotification } = createMockBridge()

    renderApp(bridge)
    await openApprovals()
    emitNotification('item/fileChange/requestApproval', fileChangeRequest, 'req-file')
    emitNotification('item/fileChange/patchUpdated', {
      threadId: 'thread-1',
      turnId: 'turn-1',
      itemId: 'item-file',
      changes: [],
    })

    expect(
      await screen.findByText('No file changes are available for this approval yet.'),
    ).toBeInTheDocument()

    emitNotification('item/fileChange/patchUpdated', {
      threadId: 'thread-1',
      turnId: 'turn-1',
      itemId: 'item-file',
      changes: [
        { path: '/protected/image.png', kind: 'binary', diff: 'Binary files differ' },
        {
          path: '/protected/large.txt',
          kind: 'modify',
          diff: 'x'.repeat(70 * 1024),
        },
      ],
    })

    expect(await screen.findByText('Binary diff content is hidden for safety.')).toBeInTheDocument()
    expect(screen.getByText(/Diff truncated at 65536 characters/)).toBeInTheDocument()
  })

  it('rejects malformed approval requests and file diffs without responding', async () => {
    const { bridge, respond, emitNotification } = createMockBridge()

    renderApp(bridge)
    await openApprovals()
    await waitFor(() => expect(bridge.onAppServerNotification).toHaveBeenCalledTimes(2))
    emitNotification('item/commandExecution/requestApproval', { command: 'echo bad' })
    expect(await screen.findByText('Malformed approval request')).toBeInTheDocument()

    emitNotification('item/fileChange/requestApproval', fileChangeRequest, 'req-bad-diff')
    emitNotification('item/fileChange/patchUpdated', {
      threadId: 'thread-1',
      turnId: 'turn-1',
      itemId: 'item-file',
      changes: [{ path: '', kind: 'modify', diff: 'bad' }],
    })

    expect(await screen.findByText('Malformed file change diff')).toBeInTheDocument()
    expect(respond).not.toHaveBeenCalled()
  })

  it('renders tool user input and submits answers with the original id', async () => {
    const { bridge, notify, respond, emitNotification } = createMockBridge()

    renderApp(bridge)
    await openApprovals()
    emitNotification(
      'item/tool/requestUserInput',
      {
        threadId: 'thread-1',
        turnId: 'turn-1',
        itemId: 'item-tool',
        isBlocking: true,
        autoResolutionMs: null,
        questions: [
          {
            id: 'q1',
            header: 'Environment',
            question: 'Which environment?',
            isOther: false,
            isSecret: false,
            options: [
              { label: 'Production', description: 'Live environment' },
              { label: 'Staging', description: 'Test environment' },
            ],
          },
        ],
      },
      'req-tool',
    )

    expect(
      await screen.findByRole('heading', { name: 'Tool input requested', level: 2 }),
    ).toBeInTheDocument()
    fireEvent.click(screen.getByLabelText(/Production/))
    fireEvent.click(screen.getByRole('button', { name: 'Submit' }))

    await waitFor(() =>
      expect(respond).toHaveBeenCalledWith('req-tool', {
        answers: { q1: { answers: ['Production'] } },
      }),
    )
    expect(notify).not.toHaveBeenCalled()
  })

  it('renders MCP elicitation and accepts valid JSON only', async () => {
    const { bridge, respond, emitNotification } = createMockBridge()

    renderApp(bridge)
    await openApprovals()
    emitNotification(
      'mcpServer/elicitation/request',
      {
        threadId: 'thread-1',
        turnId: 'turn-1',
        serverName: 'test-server',
        mode: 'form',
        message: 'Provide deployment details',
        _meta: null,
        requestedSchema: { type: 'object' },
      },
      'req-mcp',
    )

    expect(
      await screen.findByRole('heading', { name: 'MCP elicitation requested', level: 2 }),
    ).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Accept' }))
    expect(
      await screen.findByText('Enter valid JSON before accepting this elicitation.'),
    ).toBeInTheDocument()
    expect(respond).not.toHaveBeenCalled()

    fireEvent.change(screen.getByLabelText('JSON response'), {
      target: { value: '{"environment":"staging"}' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Accept' }))

    await waitFor(() =>
      expect(respond).toHaveBeenCalledWith('req-mcp', {
        action: 'accept',
        content: { environment: 'staging' },
        _meta: null,
      }),
    )
  })

  it('keeps a failed response pending and does not invoke external side effects', async () => {
    const { bridge, notify, respond, emitNotification } = createMockBridge()
    respond.mockRejectedValueOnce(new Error('response channel closed'))

    renderApp(bridge)
    await openApprovals()
    emitNotification('item/commandExecution/requestApproval', commandRequest, 'req-fail')
    expect(
      await screen.findByRole('heading', { name: 'Command execution approval', level: 2 }),
    ).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Approve' }))
    expect(await screen.findByText(/response channel closed/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Approve' })).toBeInTheDocument()
    expect(notify).not.toHaveBeenCalled()
  })

  it('marks pending approvals disconnected and does not auto-respond on reconnect', async () => {
    const { bridge, respond, emitNotification, emitStatus } = createMockBridge()

    renderApp(bridge)
    await openApprovals()
    emitNotification('item/commandExecution/requestApproval', commandRequest, 'req-disconnect')
    expect(await screen.findByText('Pending approval')).toBeInTheDocument()

    emitStatus({
      state: 'Reconnecting',
      pid: null,
      pendingRequests: 0,
      queuedMessages: 0,
      reconnectAttempt: 1,
      lastError: {
        code: null,
        message: 'Connection lost',
        kind: 'connectionClosed',
        retryable: true,
      },
    })

    expect(await screen.findByText('Disconnected')).toBeInTheDocument()
    emitStatus(readyStatus())
    await waitFor(() => expect(screen.getByText('Disconnected')).toBeInTheDocument())
    expect(respond).not.toHaveBeenCalled()
  })

  it('converges when another client resolves the request', async () => {
    const { bridge, respond, emitNotification } = createMockBridge()

    renderApp(bridge)
    await openApprovals()
    emitNotification('item/commandExecution/requestApproval', commandRequest, 'req-resolved')
    expect(await screen.findByText('Pending approval')).toBeInTheDocument()

    emitNotification('serverRequest/resolved', {
      threadId: 'thread-1',
      requestId: 'req-resolved',
    })

    expect(await screen.findByText('Responded')).toBeInTheDocument()
    expect(respond).not.toHaveBeenCalled()
  })

  it('sends cancel as a fail-closed response', async () => {
    const { bridge, notify, respond, emitNotification } = createMockBridge()

    renderApp(bridge)
    await openApprovals()
    emitNotification('item/commandExecution/requestApproval', commandRequest, 'req-cancel')
    expect(
      await screen.findByRole('heading', { name: 'Command execution approval', level: 2 }),
    ).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(respond).toHaveBeenCalledWith('req-cancel', { decision: 'cancel' }))
    expect(notify).not.toHaveBeenCalled()
  })
})
