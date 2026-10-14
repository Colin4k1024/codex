import { useEffect, useState } from 'react'
import { AppShell, EmptyState } from './components/AppShell'
import { Button } from './components/ui/Button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from './components/ui/Card'
import { Spinner } from './components/ui/Spinner'
import { formatDesktopInfo } from '../shared/desktopInfo'
import { ThreadsWorkspace } from './features/threads/ThreadsWorkspace'
import { useThreadWorkspace } from './features/threads/useThreadWorkspace'
import type { AppServerStatus } from '../shared/appServer'
import type { CodexDesktopBridge, DesktopInfo } from '../shared/types'

function ThreadsPanel({
  bridge,
  appServerStatus,
}: {
  bridge: CodexDesktopBridge
  appServerStatus: AppServerStatus | null
}) {
  const threadController = useThreadWorkspace({ bridge, appServerStatus })

  return <ThreadsWorkspace controller={threadController} />
}

declare global {
  interface Window {
    codexDesktop: CodexDesktopBridge
  }
}

export function App() {
  const [info, setInfo] = useState<DesktopInfo | null>(null)
  const [appServerStatus, setAppServerStatus] = useState<AppServerStatus | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [retryCount, setRetryCount] = useState(0)

  useEffect(() => {
    let cancelled = false

    async function loadDesktopState(): Promise<void> {
      try {
        const [desktopInfo, status] = await Promise.all([
          window.codexDesktop.getDesktopInfo(),
          window.codexDesktop.getAppServerStatus(),
        ])
        if (!cancelled) {
          setInfo(desktopInfo)
          setAppServerStatus(status)
          setError(null)
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : 'Unable to load desktop state')
        }
      }
    }

    void loadDesktopState()
    const unsubscribe = window.codexDesktop.onAppServerStatusChanged((status) => {
      if (!cancelled) {
        setAppServerStatus(status)
      }
    })

    return () => {
      cancelled = true
      unsubscribe()
    }
  }, [retryCount])

  const connectionState = appServerStatus?.state ?? 'Disconnected'
  const threadsEnabled = !window.location.search.includes('disableThreads=1')

  return (
    <AppShell connectionState={connectionState}>
      <div className="mx-auto grid max-w-5xl gap-6">
        {error ? (
          <Card>
            <CardHeader>
              <CardTitle>Unable to load desktop state</CardTitle>
              <CardDescription>{error}</CardDescription>
            </CardHeader>
            <CardContent>
              <Button onClick={() => setRetryCount((count) => count + 1)}>Try again</Button>
            </CardContent>
          </Card>
        ) : info ? (
          <Card>
            <CardHeader>
              <CardTitle>Runtime</CardTitle>
              <CardDescription>{formatDesktopInfo(info)}</CardDescription>
            </CardHeader>
            <CardContent className="flex flex-wrap gap-3">
              <Button
                variant={connectionState === 'Ready' ? 'secondary' : 'primary'}
                onClick={() => void window.codexDesktop.startAppServer()}
              >
                {connectionState === 'Ready' ? 'Restart app server' : 'Start app server'}
              </Button>
              <Button variant="ghost" onClick={() => void window.codexDesktop.stopAppServer()}>
                Stop app server
              </Button>
            </CardContent>
          </Card>
        ) : (
          <Card>
            <CardContent className="flex h-24 items-center justify-center">
              <Spinner label="Loading desktop runtime" />
            </CardContent>
          </Card>
        )}

        {threadsEnabled ? (
          <ThreadsPanel bridge={window.codexDesktop} appServerStatus={appServerStatus} />
        ) : (
          <EmptyState
            title="Threads are disabled"
            description="Enable threads to list, resume, and continue Codex conversations."
          />
        )}
      </div>
    </AppShell>
  )
}
