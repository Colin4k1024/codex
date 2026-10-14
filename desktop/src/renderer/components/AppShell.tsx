import type { ReactNode } from 'react'
import type { AppServerConnectionState } from '../../shared/appServer'
import { Badge } from './ui/Badge'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from './ui/Card'
import { SidebarItem } from './ui/SidebarItem'
import { StatusDot } from './ui/StatusDot'

export interface AppShellProps {
  children: ReactNode
  connectionState: AppServerConnectionState
}

const navigationItems = [
  { id: 'threads', label: 'Threads' },
  { id: 'approvals', label: 'Approvals' },
  { id: 'settings', label: 'Settings' },
]

function connectionTone(
  state: AppServerConnectionState,
): 'neutral' | 'success' | 'warning' | 'danger' {
  switch (state) {
    case 'Ready':
      return 'success'
    case 'Reconnecting':
      return 'warning'
    case 'ShuttingDown':
      return 'danger'
    case 'Disconnected':
    case 'Starting':
    case 'Initializing':
      return 'neutral'
  }
}

export function AppShell({ children, connectionState }: AppShellProps) {
  return (
    <div className="flex h-full min-h-screen flex-col bg-background text-foreground">
      <header className="flex h-16 shrink-0 items-center justify-between border-b border-border bg-surface px-6">
        <div className="flex items-center gap-3">
          <h1 className="text-lg font-semibold tracking-tight">Codex Desktop</h1>
          <Badge variant="default">Preview</Badge>
        </div>
        <StatusDot
          tone={connectionTone(connectionState)}
          label={`App server: ${connectionState}`}
        />
      </header>

      <a
        href="#main-content"
        className="sr-only focus:not-sr-only focus:absolute focus:left-4 focus:top-4 focus:z-10 focus:rounded-[var(--radius-md)] focus:bg-primary focus:px-4 focus:py-2 focus:text-primary-foreground"
      >
        Skip to main content
      </a>

      <div className="flex min-h-0 flex-1 flex-col md:flex-row">
        <nav
          aria-label="Primary"
          className="flex w-full shrink-0 flex-row gap-1 overflow-x-auto border-b border-border bg-surface p-3 md:w-64 md:flex-col md:border-b-0 md:border-r"
        >
          {navigationItems.map((item, index) => (
            <SidebarItem key={item.id} active={index === 0}>
              {item.label}
            </SidebarItem>
          ))}
        </nav>

        <main id="main-content" className="min-w-0 flex-1 overflow-y-auto bg-background p-6">
          {children}
        </main>
      </div>

      <footer className="flex h-12 shrink-0 items-center justify-between border-t border-border bg-surface px-6 text-sm text-foreground-muted">
        <span>Codex Desktop Preview</span>
        <span>Keyboard navigation enabled</span>
      </footer>
    </div>
  )
}

export function EmptyState({ title, description }: { title: string; description: string }) {
  return (
    <Card className="mx-auto max-w-xl">
      <CardHeader>
        <CardTitle>{title}</CardTitle>
        <CardDescription>{description}</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="flex h-32 items-center justify-center rounded-[var(--radius-md)] border border-dashed border-border bg-surface-muted text-sm text-foreground-muted">
          No sessions yet
        </div>
      </CardContent>
    </Card>
  )
}
