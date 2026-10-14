import { describe, expect, it } from 'vitest'
import { formatDesktopInfo } from '../src/shared/desktopInfo'
import type { DesktopInfo } from '../src/shared/types'

describe('formatDesktopInfo', () => {
  it('formats all desktop version fields', () => {
    const info: DesktopInfo = {
      platform: 'darwin',
      electronVersion: '33.2.0',
      nodeVersion: '20.18.1',
      chromeVersion: '130.0.6723.44',
    }

    expect(formatDesktopInfo(info)).toBe(
      'Platform: darwin • Electron: 33.2.0 • Node: 20.18.1 • Chromium: 130.0.6723.44',
    )
  })
})
