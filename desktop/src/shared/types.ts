import type {
  AppServerNotificationEvent,
  AppServerNotificationMethod,
  AppServerRequestId,
  AppServerRequestMethod,
  AppServerStatus,
} from './appServer'

export type DesktopPlatform = 'darwin' | 'linux' | 'win32'

export interface DesktopInfo {
  platform: DesktopPlatform
  electronVersion: string
  nodeVersion: string
  chromeVersion: string
}

export interface CodexDesktopBridge {
  getDesktopInfo(): Promise<DesktopInfo>
  startAppServer(): Promise<AppServerStatus>
  stopAppServer(): Promise<AppServerStatus>
  getAppServerStatus(): Promise<AppServerStatus>
  requestAppServer(method: AppServerRequestMethod, params?: unknown): Promise<unknown>
  notifyAppServer(method: AppServerNotificationMethod, params?: unknown): Promise<void>
  respondAppServerRequest(id: AppServerRequestId, result: unknown): Promise<void>
  onAppServerStatusChanged(listener: (status: AppServerStatus) => void): () => void
  onAppServerNotification(listener: (event: AppServerNotificationEvent) => void): () => void
}
