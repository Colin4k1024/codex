import { beforeEach, describe, expect, it, vi } from 'vitest'

const exposeInMainWorld = vi.fn()
const invoke = vi.fn()

vi.mock('electron', () => ({
  contextBridge: {
    exposeInMainWorld,
  },
  ipcRenderer: {
    invoke,
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
      }),
    )

    const bridge = exposeInMainWorld.mock.calls[0][1] as {
      getDesktopInfo: () => Promise<unknown>
    }
    expect(Object.keys(bridge)).toEqual(['getDesktopInfo'])

    await bridge.getDesktopInfo()
    expect(invoke).toHaveBeenCalledWith('desktop:info')
  })
})
