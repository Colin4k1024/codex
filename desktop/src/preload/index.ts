import { contextBridge, ipcRenderer } from 'electron'
import {
  APP_SERVER_CHANNELS,
  type AppServerNotificationEvent,
  type AppServerStatus,
} from '../shared/appServer'
import type { CodexDesktopBridge } from '../shared/types'

function subscribe(
  channel: string,
  listener: (payload: AppServerStatus | AppServerNotificationEvent) => void,
): () => void {
  const wrappedListener = (
    _event: unknown,
    payload: AppServerStatus | AppServerNotificationEvent,
  ): void => {
    listener(payload)
  }
  ipcRenderer.on(channel, wrappedListener)
  return () => {
    ipcRenderer.removeListener(channel, wrappedListener)
  }
}

const bridge: CodexDesktopBridge = {
  getDesktopInfo: () => ipcRenderer.invoke('desktop:info'),
  startAppServer: () => ipcRenderer.invoke(APP_SERVER_CHANNELS.start),
  stopAppServer: () => ipcRenderer.invoke(APP_SERVER_CHANNELS.stop),
  getAppServerStatus: () => ipcRenderer.invoke(APP_SERVER_CHANNELS.status),
  requestAppServer: (method, params) =>
    ipcRenderer.invoke(APP_SERVER_CHANNELS.request, { method, params }),
  notifyAppServer: (method, params) =>
    ipcRenderer.invoke(APP_SERVER_CHANNELS.notify, { method, params }),
  respondAppServerRequest: (id, result) =>
    ipcRenderer.invoke(APP_SERVER_CHANNELS.respondRequest, id, result),
  onAppServerStatusChanged: (listener) =>
    subscribe(APP_SERVER_CHANNELS.statusChanged, (payload) => {
      listener(payload as AppServerStatus)
    }),
  onAppServerNotification: (listener) =>
    subscribe(APP_SERVER_CHANNELS.notification, (payload) => {
      listener(payload as AppServerNotificationEvent)
    }),
}

contextBridge.exposeInMainWorld('codexDesktop', bridge)
