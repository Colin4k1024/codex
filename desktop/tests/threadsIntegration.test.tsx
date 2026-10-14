import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
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

describe('thread workspace app-server integration', () => {
  it('drives list, resume, turn input, and model notifications through JSON-RPC', async () => {
    const manager = await startReady('threads')
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
      notifyAppServer: async (method, params) => {
        manager.notify(method, params)
      },
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

    expect(await screen.findByRole('button', { name: /Integration thread/ })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /Integration thread/ }))
    await waitFor(() =>
      expect(requestAppServer).toHaveBeenCalledWith('thread/resume', {
        threadId: 'integration-thread-1',
        excludeTurns: false,
      }),
    )

    fireEvent.change(screen.getByLabelText('Message'), {
      target: { value: 'Hello from integration' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Send' }))

    expect(await screen.findByText('Model reply')).toBeInTheDocument()
    expect(await screen.findByText('Turn: completed')).toBeInTheDocument()
    expect(requestAppServer).toHaveBeenNthCalledWith(3, 'turn/start', {
      threadId: 'integration-thread-1',
      input: [{ type: 'text', text: 'Hello from integration', text_elements: [] }],
    })
  })
})
