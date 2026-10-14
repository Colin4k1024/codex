import { useState, type FormEvent } from 'react'
import { Button } from '../../components/ui/Button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/Card'
import { Input } from '../../components/ui/Input'
import { Spinner } from '../../components/ui/Spinner'
import { StatusDot } from '../../components/ui/StatusDot'
import { canStartTurn, getThreadTitle, type ConversationEntry, type ThreadViewState } from './state'

export interface ConversationStreamProps {
  state: ThreadViewState
  onSubmit: (text: string) => void
}

function userText(content: ConversationEntry['item']): string {
  if (content.type !== 'userMessage') {
    return ''
  }
  return content.content
    .filter((input): input is Extract<typeof input, { type: 'text' }> => input.type === 'text')
    .map((input) => input.text)
    .join('\n')
}

function renderEntry(entry: ConversationEntry) {
  const item = entry.item
  const base =
    'rounded-[var(--radius-md)] border border-border bg-surface p-3 text-sm whitespace-pre-wrap'

  switch (item.type) {
    case 'error':
      return (
        <article
          key={item.id}
          className={`${base} border-danger text-danger`}
          aria-label="Error item"
        >
          <p className="font-semibold">Error</p>
          <p>{item.message}</p>
          <p className="mt-1 text-xs">
            {item.willRetry ? 'The server will retry.' : 'The turn stopped.'}
          </p>
        </article>
      )
    case 'userMessage':
      return (
        <article key={item.id} className={`${base} bg-surface-muted`} aria-label="User message">
          <p>{userText(item)}</p>
        </article>
      )
    case 'agentMessage':
      return (
        <article key={item.id} className={base} aria-label="Assistant message">
          <p className="mb-1 text-xs font-semibold text-foreground-muted">Assistant</p>
          <p>{item.text}</p>
        </article>
      )
    case 'reasoning':
      return (
        <article key={item.id} className={`${base} bg-surface-muted`} aria-label="Reasoning item">
          <p className="mb-1 text-xs font-semibold text-foreground-muted">Reasoning</p>
          {item.summary.length > 0 ? <p>{item.summary.join('\n')}</p> : null}
          {item.content.length > 0 ? <p>{item.content.join('\n')}</p> : null}
        </article>
      )
    case 'plan':
      return (
        <article key={item.id} className={base} aria-label="Plan item">
          <p className="mb-1 text-xs font-semibold text-foreground-muted">Plan</p>
          <p>{item.text}</p>
        </article>
      )
    case 'commandExecution':
      return (
        <article key={item.id} className={base} aria-label="Tool item">
          <p className="mb-1 text-xs font-semibold text-foreground-muted">Tool: command</p>
          <p className="font-mono text-xs">{item.command}</p>
          <StatusDot tone="neutral" label={item.status} />
          {item.aggregatedOutput ? (
            <pre className="mt-2 max-h-40 overflow-auto rounded-[var(--radius-sm)] bg-surface-muted p-2 font-mono text-xs">
              {item.aggregatedOutput}
            </pre>
          ) : null}
        </article>
      )
    case 'mcpToolCall':
    case 'dynamicToolCall':
      return (
        <article key={item.id} className={base} aria-label="Tool item">
          <p className="mb-1 text-xs font-semibold text-foreground-muted">Tool: {item.tool}</p>
          <StatusDot tone="neutral" label={item.status} />
        </article>
      )
    default:
      return (
        <article key={item.id} className={base} aria-label={`${item.type} item`}>
          <p className="mb-1 text-xs font-semibold text-foreground-muted">{item.type}</p>
        </article>
      )
  }
}

function turnLabel(state: ThreadViewState): {
  tone: 'neutral' | 'success' | 'warning' | 'danger'
  label: string
} {
  if (state.turn.error) {
    return { tone: 'danger', label: `Turn: ${state.turn.phase} — ${state.turn.error}` }
  }
  if (state.turn.phase === 'Terminal' && state.turn.terminalStatus) {
    return { tone: 'neutral', label: `Turn: ${state.turn.terminalStatus}` }
  }
  if (state.turn.phase === 'Running') {
    return { tone: 'success', label: 'Turn: running' }
  }
  if (state.turn.phase === 'Starting') {
    return { tone: 'warning', label: 'Turn: starting' }
  }
  return { tone: 'neutral', label: 'Turn: idle' }
}

export function ConversationStream({ state, onSubmit }: ConversationStreamProps) {
  const [message, setMessage] = useState('')
  const turn = turnLabel(state)
  const canSubmit = canStartTurn(state) && message.trim().length > 0

  function handleSubmit(event: FormEvent<HTMLFormElement>): void {
    event.preventDefault()
    if (!canSubmit) {
      return
    }
    onSubmit(message.trim())
    setMessage('')
  }

  return (
    <Card className="min-w-0">
      <CardHeader className="space-y-2">
        <CardTitle>{getThreadTitle(state.thread)}</CardTitle>
        <CardDescription className="flex flex-wrap items-center gap-3">
          {state.thread?.model ? <span>{state.thread.model}</span> : null}
          {state.thread?.modelProvider ? <span>{state.thread.modelProvider}</span> : null}
          <StatusDot tone={turn.tone} label={turn.label} />
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {state.phase === 'Loading' ? <Spinner label="Loading thread history" /> : null}
        {state.error ? (
          <p role="alert" className="text-sm text-danger">
            {state.error}
          </p>
        ) : null}
        {state.notice ? (
          <p role="status" className="text-sm text-warning">
            {state.notice}
          </p>
        ) : null}

        <div
          role="log"
          aria-label="Conversation stream"
          aria-live="polite"
          className="flex max-h-[28rem] min-h-48 flex-col gap-3 overflow-y-auto rounded-[var(--radius-md)] border border-border bg-surface-muted p-3"
        >
          {state.entries.map(renderEntry)}
          {state.entries.length === 0 && state.phase === 'Ready' ? (
            <p className="m-auto text-sm text-foreground-muted">
              This thread is ready. Send a message to start a turn.
            </p>
          ) : null}
        </div>

        <form className="flex flex-col gap-2 sm:flex-row" onSubmit={handleSubmit}>
          <Input
            aria-label="Message"
            placeholder="Send a message"
            value={message}
            onChange={(event) => setMessage(event.target.value)}
            disabled={!canStartTurn(state)}
          />
          <Button type="submit" disabled={!canSubmit}>
            Send
          </Button>
        </form>
      </CardContent>
    </Card>
  )
}
