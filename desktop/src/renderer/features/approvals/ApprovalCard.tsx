import { useState, type FormEvent } from 'react'
import { Button } from '../../components/ui/Button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/Card'
import { Input } from '../../components/ui/Input'
import { StatusDot } from '../../components/ui/StatusDot'
import type { ApprovalDecision, ApprovalRequest } from './protocol'
import { getApprovalTitle } from './protocol'
import { DiffViewer } from './DiffViewer'

export interface ApprovalCardProps {
  request: ApprovalRequest
  onRespond: (decision: ApprovalDecision) => void
  onCancel: () => void
}

function CommandExecutionApproval({ request, onRespond, onCancel }: ApprovalCardProps) {
  if (request.kind !== 'commandExecution') {
    return null
  }

  const command = request.params.command ?? 'Unknown command'
  const cwd = request.params.cwd ?? 'Unknown directory'
  const reason = request.params.reason

  return (
    <Card>
      <CardHeader>
        <CardTitle>{getApprovalTitle(request)}</CardTitle>
        <CardDescription>
          <StatusDot tone="warning" label="Pending approval" />
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-2">
          <p className="text-sm font-medium">Command</p>
          <pre className="overflow-x-auto rounded-[var(--radius-md)] bg-surface-muted p-3 font-mono text-sm">
            {command}
          </pre>
        </div>
        <div className="space-y-2">
          <p className="text-sm font-medium">Working directory</p>
          <p className="text-sm text-foreground-muted">{cwd}</p>
        </div>
        {reason ? (
          <div className="space-y-2">
            <p className="text-sm font-medium">Reason</p>
            <p className="text-sm text-foreground-muted">{reason}</p>
          </div>
        ) : null}
        <div className="flex flex-wrap gap-2">
          <Button
            variant="primary"
            onClick={() => onRespond({ kind: 'commandExecution', decision: 'accept' })}
          >
            Approve
          </Button>
          <Button
            variant="secondary"
            onClick={() => onRespond({ kind: 'commandExecution', decision: 'acceptForSession' })}
          >
            Approve for session
          </Button>
          <Button
            variant="danger"
            onClick={() => onRespond({ kind: 'commandExecution', decision: 'decline' })}
          >
            Deny
          </Button>
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}

function FileChangeApproval({ request, onRespond, onCancel }: ApprovalCardProps) {
  if (request.kind !== 'fileChange') {
    return null
  }

  const reason = request.params.reason
  const grantRoot = request.params.grantRoot

  return (
    <Card>
      <CardHeader>
        <CardTitle>{getApprovalTitle(request)}</CardTitle>
        <CardDescription>
          <StatusDot tone="warning" label="Pending approval" />
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {reason ? (
          <div className="space-y-2">
            <p className="text-sm font-medium">Reason</p>
            <p className="text-sm text-foreground-muted">{reason}</p>
          </div>
        ) : null}
        {grantRoot ? (
          <div className="space-y-2">
            <p className="text-sm font-medium">Grant root</p>
            <p className="text-sm text-foreground-muted">{grantRoot}</p>
          </div>
        ) : null}
        <div className="space-y-2">
          <p className="text-sm font-medium">Changes</p>
          <DiffViewer changes={request.changes} />
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            variant="primary"
            onClick={() => onRespond({ kind: 'fileChange', decision: 'accept' })}
          >
            Approve
          </Button>
          <Button
            variant="secondary"
            onClick={() => onRespond({ kind: 'fileChange', decision: 'acceptForSession' })}
          >
            Approve for session
          </Button>
          <Button
            variant="danger"
            onClick={() => onRespond({ kind: 'fileChange', decision: 'decline' })}
          >
            Deny
          </Button>
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}

function ToolUserInputApproval({ request, onRespond, onCancel }: ApprovalCardProps) {
  const [answers, setAnswers] = useState<Record<string, string[]>>({})

  if (request.kind !== 'toolUserInput') {
    return null
  }

  const questions = request.params.questions

  function handleSubmit(event: FormEvent<HTMLFormElement>): void {
    event.preventDefault()
    const response: Record<string, { answers: string[] }> = {}
    for (const question of questions) {
      const answer = answers[question.id]
      if (answer && answer.length > 0) {
        response[question.id] = { answers: answer }
      }
    }
    onRespond({ kind: 'toolUserInput', answers: response })
  }

  function updateAnswer(questionId: string, value: string): void {
    setAnswers((previous) => ({
      ...previous,
      [questionId]: [value],
    }))
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>{getApprovalTitle(request)}</CardTitle>
        <CardDescription>
          <StatusDot tone="warning" label="Input requested" />
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form className="space-y-4" onSubmit={handleSubmit}>
          {questions.map((question) => (
            <div key={question.id} className="space-y-2">
              <label className="text-sm font-medium" htmlFor={`question-${question.id}`}>
                {question.header}
              </label>
              <p className="text-sm text-foreground-muted">{question.question}</p>
              {question.options ? (
                <div className="space-y-2">
                  {question.options.map((option) => (
                    <label key={option.label} className="flex items-center gap-2 text-sm">
                      <input
                        type="radio"
                        name={`question-${question.id}`}
                        value={option.label}
                        checked={answers[question.id]?.[0] === option.label}
                        onChange={() => updateAnswer(question.id, option.label)}
                      />
                      <span>{option.label}</span>
                      <span className="text-foreground-muted">{option.description}</span>
                    </label>
                  ))}
                </div>
              ) : (
                <Input
                  id={`question-${question.id}`}
                  type={question.isSecret ? 'password' : 'text'}
                  value={answers[question.id]?.[0] ?? ''}
                  onChange={(event) => updateAnswer(question.id, event.target.value)}
                  placeholder={question.isOther ? 'Enter your answer' : ''}
                />
              )}
            </div>
          ))}
          <div className="flex flex-wrap gap-2">
            <Button type="submit" variant="primary">
              Submit
            </Button>
            <Button variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  )
}

function McpElicitationApproval({ request, onRespond, onCancel }: ApprovalCardProps) {
  const [content, setContent] = useState('')
  const [error, setError] = useState<string | null>(null)
  if (request.kind !== 'mcpElicitation') {
    return null
  }
  const params = request.params

  function handleSubmit(event: FormEvent<HTMLFormElement>): void {
    event.preventDefault()
    if (params.mode === 'url') {
      onRespond({
        kind: 'mcpElicitation',
        action: 'accept',
        content: { url: params.url },
      })
      return
    }
    if (content.trim().length === 0) {
      setError('Enter valid JSON before accepting this elicitation.')
      return
    }
    try {
      onRespond({ kind: 'mcpElicitation', action: 'accept', content: JSON.parse(content) })
    } catch {
      setError('Enter valid JSON before accepting this elicitation.')
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>{getApprovalTitle(request)}</CardTitle>
        <CardDescription>
          <StatusDot tone="warning" label="Input requested" />
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form className="space-y-4" onSubmit={handleSubmit}>
          <div className="space-y-1">
            <p className="text-sm font-medium">{params.serverName}</p>
            <p className="text-sm text-foreground-muted">{params.message}</p>
            {params.mode === 'url' ? (
              <p className="break-all font-mono text-xs text-foreground-muted">{params.url}</p>
            ) : null}
          </div>
          {params.mode !== 'url' ? (
            <div className="space-y-2">
              <label className="text-sm font-medium" htmlFor="mcp-elicitation-content">
                JSON response
              </label>
              <textarea
                id="mcp-elicitation-content"
                className="min-h-24 w-full rounded-[var(--radius-md)] border border-border bg-surface p-3 font-mono text-sm"
                value={content}
                onChange={(event) => setContent(event.target.value)}
                aria-describedby={error ? 'mcp-elicitation-error' : undefined}
              />
            </div>
          ) : null}
          {error ? (
            <p id="mcp-elicitation-error" role="alert" className="text-sm text-danger">
              {error}
            </p>
          ) : null}
          <div className="flex flex-wrap gap-2">
            <Button type="submit" variant="primary">
              Accept
            </Button>
            <Button
              variant="danger"
              onClick={() =>
                onRespond({ kind: 'mcpElicitation', action: 'decline', content: null })
              }
            >
              Decline
            </Button>
            <Button variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  )
}

export function ApprovalCard({ request, onRespond, onCancel }: ApprovalCardProps) {
  switch (request.kind) {
    case 'commandExecution':
      return (
        <CommandExecutionApproval request={request} onRespond={onRespond} onCancel={onCancel} />
      )
    case 'fileChange':
      return <FileChangeApproval request={request} onRespond={onRespond} onCancel={onCancel} />
    case 'toolUserInput':
      return <ToolUserInputApproval request={request} onRespond={onRespond} onCancel={onCancel} />
    case 'mcpElicitation':
      return <McpElicitationApproval request={request} onRespond={onRespond} onCancel={onCancel} />
  }
}
