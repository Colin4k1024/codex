import { beforeEach, describe, expect, it, vi } from 'vitest'
import { APP_SERVER_CHANNELS } from '../src/shared/appServer'

const exposeInMainWorld = vi.fn()
const invoke = vi.fn()
const on = vi.fn()
const removeListener = vi.fn()

vi.mock('electron', () => ({
  contextBridge: {
    exposeInMainWorld,
  },
  ipcRenderer: {
    invoke,
    on,
    removeListener,
  },
}))

describe('preload bridge', () => {
  beforeEach(() => {
    vi.resetModules()
    vi.clearAllMocks()
  })

  it('exposes only the typed desktop bridge', async () => {
    await import('../src/preload')

    expect(exposeInMainWorld).toHaveBeenCalledTimes(1)
    expect(exposeInMainWorld).toHaveBeenCalledWith(
      'codexDesktop',
      expect.objectContaining({
        getDesktopInfo: expect.any(Function),
        startAppServer: expect.any(Function),
        stopAppServer: expect.any(Function),
        getAppServerStatus: expect.any(Function),
        requestAppServer: expect.any(Function),
        notifyAppServer: expect.any(Function),
        respondAppServerRequest: expect.any(Function),
        onAppServerStatusChanged: expect.any(Function),
        onAppServerNotification: expect.any(Function),
      }),
    )

    const bridge = exposeInMainWorld.mock.calls[0][1] as {
      getDesktopInfo: () => Promise<unknown>
      startAppServer: () => Promise<unknown>
      stopAppServer: () => Promise<unknown>
      getAppServerStatus: () => Promise<unknown>
      requestAppServer: (method: string, params?: unknown) => Promise<unknown>
      notifyAppServer: (method: string, params?: unknown) => Promise<void>
      respondAppServerRequest: (id: string | number, result: unknown) => Promise<void>
      onAppServerStatusChanged: (listener: (status: unknown) => void) => () => void
      onAppServerNotification: (listener: (event: unknown) => void) => () => void
    }
    expect(Object.keys(bridge)).toEqual([
      'getDesktopInfo',
      'startAppServer',
      'stopAppServer',
      'getAppServerStatus',
      'requestAppServer',
      'notifyAppServer',
      'respondAppServerRequest',
      'onAppServerStatusChanged',
      'onAppServerNotification',
    ])

    await bridge.getDesktopInfo()
    expect(invoke).toHaveBeenCalledWith('desktop:info')

    invoke.mockClear()
    await bridge.startAppServer()
    expect(invoke).toHaveBeenCalledWith(APP_SERVER_CHANNELS.start)

    invoke.mockClear()
    await bridge.stopAppServer()
    expect(invoke).toHaveBeenCalledWith(APP_SERVER_CHANNELS.stop)

    invoke.mockClear()
    await bridge.getAppServerStatus()
    expect(invoke).toHaveBeenCalledWith(APP_SERVER_CHANNELS.status)

    invoke.mockClear()
    await bridge.requestAppServer('account/logout', undefined)
    expect(invoke).toHaveBeenCalledWith(APP_SERVER_CHANNELS.request, {
      method: 'account/logout',
      params: undefined,
    })

    invoke.mockClear()
    await bridge.notifyAppServer('account/logout', undefined)
    expect(invoke).toHaveBeenCalledWith(APP_SERVER_CHANNELS.notify, {
      method: 'account/logout',
      params: undefined,
    })
  })

  it('subscribes and unsubscribes app-server events through the typed channels', async () => {
    await import('../src/preload')

    const bridge = exposeInMainWorld.mock.calls[0][1] as {
      onAppServerStatusChanged: (listener: (status: unknown) => void) => () => void
      onAppServerNotification: (listener: (event: unknown) => void) => () => void
    }
    const statusListener = vi.fn()
    const notificationListener = vi.fn()

    const unsubscribeStatus = bridge.onAppServerStatusChanged(statusListener)
    const unsubscribeNotification = bridge.onAppServerNotification(notificationListener)

    expect(on).toHaveBeenCalledWith(APP_SERVER_CHANNELS.statusChanged, expect.any(Function))
    expect(on).toHaveBeenCalledWith(APP_SERVER_CHANNELS.notification, expect.any(Function))

    const statusHandler = on.mock.calls.find(
      ([channel]) => channel === APP_SERVER_CHANNELS.statusChanged,
    )?.[1] as (event: unknown, payload: unknown) => void
    const notificationHandler = on.mock.calls.find(
      ([channel]) => channel === APP_SERVER_CHANNELS.notification,
    )?.[1] as (event: unknown, payload: unknown) => void

    statusHandler(null, { state: 'Ready' })
    notificationHandler(null, { method: 'thread/started' })

    expect(statusListener).toHaveBeenCalledWith({ state: 'Ready' })
    expect(notificationListener).toHaveBeenCalledWith({ method: 'thread/started' })

    unsubscribeStatus()
    unsubscribeNotification()

    expect(removeListener).toHaveBeenCalledWith(APP_SERVER_CHANNELS.statusChanged, statusHandler)
    expect(removeListener).toHaveBeenCalledWith(
      APP_SERVER_CHANNELS.notification,
      notificationHandler,
    )
  })
})
