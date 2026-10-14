import { Button } from '../../components/ui/Button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/Card'
import { SidebarItem } from '../../components/ui/SidebarItem'
import { Spinner } from '../../components/ui/Spinner'
import { StatusDot } from '../../components/ui/StatusDot'
import { getThreadTitle, type ThreadListState } from './state'

export interface ThreadListProps {
  state: ThreadListState
  onCreateThread: () => void
  onResumeThread: (threadId: string) => void
  onLoadNextPage: () => void
}

function threadStatusTone(status: string): 'neutral' | 'success' | 'warning' | 'danger' {
  if (status === 'systemError') {
    return 'danger'
  }
  return status === 'active' ? 'success' : 'neutral'
}

export function ThreadList({
  state,
  onCreateThread,
  onResumeThread,
  onLoadNextPage,
}: ThreadListProps) {
  return (
    <Card className="min-w-0">
      <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="space-y-1">
          <CardTitle>Threads</CardTitle>
          <CardDescription>
            {state.threads.length === 0
              ? 'Load or create a Codex thread.'
              : `${state.threads.length} thread${state.threads.length === 1 ? '' : 's'} loaded`}
          </CardDescription>
        </div>
        <Button onClick={onCreateThread} disabled={state.phase !== 'Ready'}>
          New thread
        </Button>
      </CardHeader>
      <CardContent className="space-y-4">
        {state.phase === 'Loading' ? (
          <Spinner label="Loading threads" />
        ) : (
          <ul className="space-y-1" aria-label="Thread list">
            {state.threads.map((thread) => (
              <li key={thread.id}>
                <SidebarItem onClick={() => onResumeThread(thread.id)}>
                  <span className="flex min-w-0 flex-col gap-1">
                    <span className="truncate font-medium">{getThreadTitle(thread)}</span>
                    <span className="truncate text-xs text-foreground-muted">
                      {thread.preview || thread.id}
                    </span>
                    <StatusDot
                      tone={threadStatusTone(thread.status.type)}
                      label={thread.status.type}
                    />
                  </span>
                </SidebarItem>
              </li>
            ))}
          </ul>
        )}

        {state.threads.length === 0 && state.phase === 'Ready' ? (
          <p className="rounded-[var(--radius-md)] border border-dashed border-border bg-surface-muted p-4 text-sm text-foreground-muted">
            No threads are available yet.
          </p>
        ) : null}

        {state.error ? (
          <p role="alert" className="text-sm text-danger">
            {state.error}
          </p>
        ) : null}

        {state.loadingMore ? <Spinner label="Loading next page" /> : null}
        {state.nextCursor !== null && state.phase === 'Ready' && !state.loadingMore ? (
          <Button variant="secondary" onClick={onLoadNextPage}>
            Load more threads
          </Button>
        ) : null}
      </CardContent>
    </Card>
  )
}
