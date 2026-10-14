import { describe, expect, it, vi, beforeEach } from 'vitest'
import { APP_SERVER_CHANNELS, type AppServerStatus } from '../../src/shared/appServer'

const { handle, isDestroyed, send } = vi.hoisted(() => ({
  handle: vi.fn(),
  isDestroyed: vi.fn(() => false),
  send: vi.fn(),
}))

vi.mock('electron', () => ({
  ipcMain: { handle },
  WebContents: class {},
}))

import { registerAppServerIpc } from '../../src/main/appServer/ipc'

function createManager() {
  const statusListeners: Array<(status: AppServerStatus) => void> = []
  const notificationListeners: Array<(event: { method: string; params?: unknown }) => void> = []
  const status: AppServerStatus = {
    state: 'Disconnected',
    pid: null,
    pendingRequests: 0,
    queuedMessages: 0,
    reconnectAttempt: null,
    lastError: null,
  }
  return {
    status,
    onStatus: vi.fn((listener) => {
      statusListeners.push(listener)
      return () => undefined
    }),
    onNotification: vi.fn((listener) => {
      notificationListeners.push(listener)
      return () => undefined
    }),
    start: vi.fn(async () => status),
    stop: vi.fn(async () => status),
    request: vi.fn(async () => ({ ok: true })),
    notify: vi.fn(),
    respondToServerRequest: vi.fn(async () => undefined),
    emitStatus: (value: AppServerStatus) => {
      for (const listener of statusListeners) {
        listener(value)
      }
    },
    emitNotification: (value: { method: string; params?: unknown }) => {
      for (const listener of notificationListeners) {
        listener(value)
      }
    },
  }
}

function createWebContents() {
  return { send, isDestroyed }
}

function getHandler(channel: string): (...args: unknown[]) => unknown {
  const call = handle.mock.calls.find(([registeredChannel]) => registeredChannel === channel)
  if (!call) {
    throw new Error(`no handler registered for ${channel}`)
  }
  const handler = call[1] as (event: unknown, ...args: unknown[]) => unknown
  return (...args: unknown[]) => handler(null, ...args)
}

describe('registerAppServerIpc', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    isDestroyed.mockReturnValue(false)
  })

  it('registers the typed app-server channels', () => {
    const manager = createManager()

    registerAppServerIpc({ manager, getWebContents: () => [createWebContents()] })

    const channels = handle.mock.calls.map(([channel]) => channel)
    expect(channels).toEqual([
      APP_SERVER_CHANNELS.start,
      APP_SERVER_CHANNELS.stop,
      APP_SERVER_CHANNELS.status,
      APP_SERVER_CHANNELS.request,
      APP_SERVER_CHANNELS.notify,
      APP_SERVER_CHANNELS.respondRequest,
    ])
  })

  it('forwards IPC calls to the manager', async () => {
    const manager = createManager()

    registerAppServerIpc({ manager, getWebContents: () => [createWebContents()] })

    await getHandler(APP_SERVER_CHANNELS.start)()
    await getHandler(APP_SERVER_CHANNELS.stop)()
    await getHandler(APP_SERVER_CHANNELS.status)()
    await getHandler(APP_SERVER_CHANNELS.request)({ method: 'account/logout' })
    await getHandler(APP_SERVER_CHANNELS.notify)({ method: 'test/notify' })
    await getHandler(APP_SERVER_CHANNELS.respondRequest)('request-1', { decision: 'decline' })

    expect(manager.start).toHaveBeenCalledTimes(1)
    expect(manager.stop).toHaveBeenCalledTimes(1)
    expect(manager.status).toBeTypeOf('object')
    expect(manager.request).toHaveBeenCalledWith('account/logout', undefined)
    expect(manager.notify).toHaveBeenCalledWith('test/notify', undefined)
    expect(manager.respondToServerRequest).toHaveBeenCalledWith('request-1', {
      decision: 'decline',
    })
  })

  it('validates request and notification payloads', async () => {
    const manager = createManager()

    registerAppServerIpc({ manager, getWebContents: () => [createWebContents()] })

    await expect(getHandler(APP_SERVER_CHANNELS.request)(null)).rejects.toThrowError(
      /payload must be an object/,
    )
    await expect(getHandler(APP_SERVER_CHANNELS.request)({ method: 123 })).rejects.toThrowError(
      /method must be a non-empty string/,
    )
    await expect(getHandler(APP_SERVER_CHANNELS.notify)('bad')).rejects.toThrowError(
      /payload must be an object/,
    )
  })

  it('forwards manager status and notifications to live renderer windows', () => {
    const manager = createManager()

    registerAppServerIpc({ manager, getWebContents: () => [createWebContents()] })

    const status: AppServerStatus = {
      state: 'Ready',
      pid: 4321,
      pendingRequests: 0,
      queuedMessages: 0,
      reconnectAttempt: null,
      lastError: null,
    }
    manager.emitStatus(status)
    manager.emitNotification({ method: 'thread/started', params: { id: 'thread-1' } })

    expect(send).toHaveBeenCalledWith(APP_SERVER_CHANNELS.statusChanged, status)
    expect(send).toHaveBeenCalledWith(APP_SERVER_CHANNELS.notification, {
      method: 'thread/started',
      params: { id: 'thread-1' },
    })
  })

  it('skips destroyed renderer windows', () => {
    const manager = createManager()

    registerAppServerIpc({ manager, getWebContents: () => [createWebContents()] })

    isDestroyed.mockReturnValue(true)
    manager.emitStatus({ ...manager.status, state: 'Reconnecting' })

    expect(send).not.toHaveBeenCalled()
  })
})
