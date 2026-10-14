import { ipcMain, type WebContents } from 'electron'
import {
  APP_SERVER_CHANNELS,
  type AppServerNotificationEvent,
  type AppServerRequestMethod,
  type AppServerStatus,
} from '../../shared/appServer'

/** Structural contract for the app-server lifecycle surface exposed over IPC. */
export interface AppServerIpcManager {
  readonly status: AppServerStatus
  onStatus(listener: (status: AppServerStatus) => void): () => void
  onNotification(listener: (event: AppServerNotificationEvent) => void): () => void
  start(): Promise<AppServerStatus>
  stop(): Promise<AppServerStatus>
  request(method: AppServerRequestMethod, params?: unknown): Promise<unknown>
  notify(method: string, params?: unknown): void
  respondToServerRequest(id: string | number, result: unknown): Promise<void>
}

export type AppServerRenderer = Pick<WebContents, 'send' | 'isDestroyed'>

export interface AppServerIpcOptions {
  manager: AppServerIpcManager
  getWebContents: () => AppServerRenderer[]
}

function isValidRpcPayload(value: unknown): value is { method?: unknown; params?: unknown } {
  return typeof value === 'object' && value !== null
}

/**
 * Registers the typed renderer bridge channels for app-server lifecycle,
 * requests, and notifications, and forwards main-side events to the renderer.
 */
export function registerAppServerIpc(options: AppServerIpcOptions): void {
  const { manager, getWebContents } = options

  const sendToRenderers = (
    channel: string,
    payload: AppServerStatus | AppServerNotificationEvent,
  ) => {
    for (const webContents of getWebContents()) {
      if (!webContents.isDestroyed()) {
        webContents.send(channel, payload)
      }
    }
  }

  manager.onStatus((status) => {
    sendToRenderers(APP_SERVER_CHANNELS.statusChanged, status)
  })
  manager.onNotification((event) => {
    sendToRenderers(APP_SERVER_CHANNELS.notification, event)
  })

  ipcMain.handle(APP_SERVER_CHANNELS.start, () => manager.start())
  ipcMain.handle(APP_SERVER_CHANNELS.stop, () => manager.stop())
  ipcMain.handle(APP_SERVER_CHANNELS.status, () => manager.status)
  ipcMain.handle(APP_SERVER_CHANNELS.request, async (_event, payload: unknown) => {
    if (!isValidRpcPayload(payload)) {
      throw new Error('appServer:request payload must be an object with method and params')
    }
    const { method, params } = payload
    if (typeof method !== 'string' || method.length === 0) {
      throw new Error('appServer:request method must be a non-empty string')
    }
    return manager.request(method as AppServerRequestMethod, params)
  })
  ipcMain.handle(APP_SERVER_CHANNELS.notify, async (_event, payload: unknown) => {
    if (!isValidRpcPayload(payload)) {
      throw new Error('appServer:notify payload must be an object with method and params')
    }
    const { method, params } = payload
    if (typeof method !== 'string' || method.length === 0) {
      throw new Error('appServer:notify method must be a non-empty string')
    }
    manager.notify(method, params)
  })
  ipcMain.handle(APP_SERVER_CHANNELS.respondRequest, (_event, id: unknown, result: unknown) => {
    if (typeof id !== 'string' && (typeof id !== 'number' || !Number.isFinite(id))) {
      throw new Error('appServer:respondRequest id must be a string or finite number')
    }
    return manager.respondToServerRequest(id, result)
  })
}
