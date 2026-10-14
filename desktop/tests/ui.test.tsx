import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import * as axe from 'axe-core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { App } from '../src/renderer/App'
import { AppShell, EmptyState } from '../src/renderer/components/AppShell'
import { Button } from '../src/renderer/components/ui/Button'
import { Input } from '../src/renderer/components/ui/Input'
import { Spinner } from '../src/renderer/components/ui/Spinner'
import { SidebarItem } from '../src/renderer/components/ui/SidebarItem'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '../src/renderer/components/ui/Card'
import { Badge } from '../src/renderer/components/ui/Badge'
import { StatusDot } from '../src/renderer/components/ui/StatusDot'

const mockBridge = {
  getDesktopInfo: vi.fn(async () => ({
    platform: 'darwin',
    electronVersion: '33.2.0',
    nodeVersion: '20.18.1',
    chromeVersion: '130.0.6723.44',
  })),
  getAppServerStatus: vi.fn(async () => ({
    state: 'Ready',
    pid: 1234,
    pendingRequests: 0,
    queuedMessages: 0,
    reconnectAttempt: null,
    lastError: null,
  })),
  startAppServer: vi.fn(async () => undefined),
  stopAppServer: vi.fn(async () => undefined),
  requestAppServer: vi.fn(async () => ({
    data: [],
    nextCursor: null,
    backwardsCursor: null,
  })),
  notifyAppServer: vi.fn(),
  onAppServerStatusChanged: vi.fn(() => () => undefined),
  onAppServerNotification: vi.fn(() => () => undefined),
}

describe('design system primitives', () => {
  it('renders button variants with accessible names', () => {
    render(
      <div>
        <Button>Start app server</Button>
        <Button variant="secondary">Restart</Button>
        <Button variant="ghost">Stop</Button>
      </div>,
    )

    expect(screen.getByRole('button', { name: 'Start app server' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Restart' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument()
  })

  it('renders input, spinner, and sidebar item states', () => {
    render(
      <div>
        <Input aria-label="Thread title" disabled />
        <Spinner label="Loading runtime" />
        <SidebarItem active>Threads</SidebarItem>
        <SidebarItem>Settings</SidebarItem>
      </div>,
    )

    expect(screen.getByLabelText('Thread title')).toBeDisabled()
    expect(screen.getByRole('status')).toHaveTextContent('Loading runtime')
    expect(screen.getByRole('button', { name: 'Threads' })).toHaveAttribute('aria-current', 'page')
    expect(screen.getByRole('button', { name: 'Settings' })).not.toHaveAttribute('aria-current')
  })

  it('renders cards, badges, and status dots with semantic structure', () => {
    render(
      <div>
        <Card>
          <CardHeader>
            <CardTitle>Runtime</CardTitle>
            <CardDescription>Electron runtime details</CardDescription>
          </CardHeader>
          <CardContent>Connected</CardContent>
        </Card>
        <Badge variant="success">Ready</Badge>
        <StatusDot tone="success" label="App server ready" />
      </div>,
    )

    expect(screen.getByText('Runtime')).toBeInTheDocument()
    expect(screen.getByText('Electron runtime details')).toBeInTheDocument()
    expect(screen.getByText('Connected')).toBeInTheDocument()
    expect(screen.getByText('Ready')).toBeInTheDocument()
    expect(screen.getByText('App server ready')).toBeInTheDocument()
  })
})

describe('AppShell', () => {
  it('renders primary navigation and responsive layout landmarks', () => {
    render(
      <AppShell connectionState="Ready">
        <div>Content</div>
      </AppShell>,
    )

    expect(screen.getByRole('banner')).toBeInTheDocument()
    expect(screen.getByRole('navigation', { name: 'Primary' })).toBeInTheDocument()
    expect(screen.getByRole('main')).toBeInTheDocument()
    expect(screen.getByRole('contentinfo')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Threads' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Approvals' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Settings' })).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Skip to main content' })).toBeInTheDocument()
    expect(screen.getByRole('navigation', { name: 'Primary' })).toBeVisible()
  })

  it('matches the responsive shell snapshot', () => {
    const { container } = render(
      <AppShell connectionState="Ready">
        <div>Content</div>
      </AppShell>,
    )
    expect(container.firstChild).toMatchSnapshot()
  })

  it('renders the empty state card', () => {
    render(<EmptyState title="No active thread" description="Start a new Codex thread." />)
    expect(screen.getByRole('heading', { name: 'No active thread', level: 2 })).toBeInTheDocument()
    expect(screen.getByText('Start a new Codex thread.')).toBeInTheDocument()
    expect(screen.getByText('No sessions yet')).toBeInTheDocument()
  })

  it('has no accessibility violations', async () => {
    const { container } = render(
      <AppShell connectionState="Ready">
        <div>Content</div>
      </AppShell>,
    )
    const results = await axe.run(container)
    expect(results.violations).toHaveLength(0)
  })
})

describe('App', () => {
  beforeEach(() => {
    vi.stubGlobal('codexDesktop', mockBridge)
  })

  it('loads desktop state and renders the shell', async () => {
    render(<App />)

    await waitFor(() => {
      expect(screen.getByText('Runtime')).toBeInTheDocument()
    })
    expect(screen.getByText(/Platform: darwin/)).toBeInTheDocument()
    expect(screen.getByText('No thread selected')).toBeInTheDocument()
  })

  it('shows a retry action when desktop state cannot load', async () => {
    mockBridge.getDesktopInfo.mockRejectedValueOnce(new Error('mock failure'))
    render(<App />)

    const retryButton = await screen.findByRole('button', { name: 'Try again' })
    expect(screen.getByText(/mock failure/)).toBeInTheDocument()

    fireEvent.click(retryButton)
    await waitFor(() => {
      expect(screen.getByText(/Platform: darwin/)).toBeInTheDocument()
    })
  })

  it('has no accessibility violations after loading', async () => {
    const { container } = render(<App />)
    await waitFor(() => {
      expect(screen.getByText(/Platform: darwin/)).toBeInTheDocument()
    })

    const results = await axe.run(container)
    expect(results.violations).toHaveLength(0)
  })
})
