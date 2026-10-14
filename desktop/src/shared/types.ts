export type DesktopPlatform = 'darwin' | 'linux' | 'win32'

export interface DesktopInfo {
  platform: DesktopPlatform
  electronVersion: string
  nodeVersion: string
  chromeVersion: string
}

export interface CodexDesktopBridge {
  getDesktopInfo(): Promise<DesktopInfo>
}
