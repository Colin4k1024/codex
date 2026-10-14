import type { DesktopPlatform } from './types'

const supportedPlatforms: ReadonlySet<DesktopPlatform> = new Set(['darwin', 'linux', 'win32'])

export function normalizePlatform(platform: string): DesktopPlatform {
  return supportedPlatforms.has(platform as DesktopPlatform)
    ? (platform as DesktopPlatform)
    : 'linux'
}
