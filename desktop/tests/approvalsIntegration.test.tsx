import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { App } from '../src/renderer/App'
import { AppServerProcessManager } from '../src/main/appServer/processManager'
import type { AppServerNotificationEvent, AppServerStatus } from '../src/shared/appServer'
import type { CodexDesktopBridge, DesktopInfo } from '../src/shared/types'
import { startReady } from './appServer/processManagerTestUtils'

const managers: AppServerProcessManager[] = []

afterEach(async () => {
  for (const manager of managers.splice(0)) {
    manager.removeProcessExitHooks()
    await manager.stop()
  }
  vi.unstubAllGlobals()
})

describe('approval workspace app-server integration', () => {
  it('responds to a real server request through JSON-RPC', async () => {
    const manager = await startReady('approvals')
    managers.push(manager)
    const requestAppServer = vi.fn(
      (method: string, params?: unknown): Promise<unknown> =>
        manager.request(method as Parameters<typeof manager.request>[0], params),
    )

    const bridge: CodexDesktopBridge = {
      getDesktopInfo: async (): Promise<DesktopInfo> => ({
        platform: 'darwin',
        electronVersion: '33.2.0',
        nodeVersion: '22.23.3',
        chromeVersion: '130.0.6723.44',
      }),
      startAppServer: async () => manager.status,
      stopAppServer: async () => manager.stop(),
      getAppServerStatus: async () => manager.status,
      requestAppServer,
      notifyAppServer: async () => undefined,
      respondAppServerRequest: (id, result) => manager.respondToServerRequest(id, result),
      onAppServerStatusChanged: (listener: (status: AppServerStatus) => void) =>
        manager.onStatus((status) => {
          act(() => {
            listener(status)
          })
        }),
      onAppServerNotification: (listener: (event: AppServerNotificationEvent) => void) =>
        manager.onNotification((event) => {
          act(() => {
            listener(event)
          })
        }),
    }
    vi.stubGlobal('codexDesktop', bridge)

    render(<App />)
    fireEvent.click(screen.getByRole('button', { name: 'Show approvals' }))

    expect(
      await screen.findByRole('heading', { name: 'Command execution approval', level: 2 }),
    ).toBeInTheDocument()
    expect(screen.getByText('echo approval-integration', { selector: 'pre' })).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: 'Deny' }))
    expect(await screen.findByText('Responded')).toBeInTheDocument()
    expect(requestAppServer).not.toHaveBeenCalledWith('command/exec', expect.anything())
  })
})
