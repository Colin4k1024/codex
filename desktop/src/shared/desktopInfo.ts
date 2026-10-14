import type { DesktopInfo } from './types'

export function formatDesktopInfo(info: DesktopInfo): string {
  return [
    `Platform: ${info.platform}`,
    `Electron: ${info.electronVersion}`,
    `Node: ${info.nodeVersion}`,
    `Chromium: ${info.chromeVersion}`,
  ].join(' • ')
}
