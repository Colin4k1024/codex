import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '../../components/ui/Card'
import { SidebarItem } from '../../components/ui/SidebarItem'
import { StatusDot } from '../../components/ui/StatusDot'
import type { ApprovalEntry, ApprovalWorkspaceState } from './state'
import { getApprovalDescription, getApprovalTitle } from './protocol'
import { ApprovalCard } from './ApprovalCard'
import type { ApprovalDecision } from './protocol'

export interface ApprovalWorkspaceProps {
  state: ApprovalWorkspaceState
  onRespond: (request: ApprovalEntry['request'], decision: ApprovalDecision) => void
  onCancel: (requestId: ApprovalEntry['request']['id']) => void
  onSelect: (requestId: ApprovalEntry['request']['id'] | null) => void
}

function ApprovalListItem({
  entry,
  selected,
  onSelect,
}: {
  entry: ApprovalEntry
  selected: boolean
  onSelect: () => void
}) {
  const request = entry.request
  const title = getApprovalTitle(request)
  const description = getApprovalDescription(request)

  return (
    <SidebarItem active={selected} onClick={onSelect}>
      <span className="flex min-w-0 flex-col gap-1">
        <span className="truncate font-medium">{title}</span>
        <span className="truncate text-xs text-foreground-muted">{description}</span>
        <StatusDot
          tone={
            entry.state === 'Pending'
              ? 'warning'
              : entry.state === 'Responded'
                ? 'success'
                : 'neutral'
          }
          label={entry.state}
        />
      </span>
    </SidebarItem>
  )
}

export function ApprovalWorkspace({
  state,
  onRespond,
  onCancel,
  onSelect,
}: ApprovalWorkspaceProps) {
  const pendingEntries = state.entries.filter((entry) => entry.state === 'Pending')
  const selectedEntry = state.selectedId
    ? state.entries.find((entry) => entry.request.id === state.selectedId)
    : undefined

  return (
    <div className="grid gap-6 lg:grid-cols-[20rem_minmax(0,1fr)]">
      <Card className="min-w-0">
        <CardHeader>
          <CardTitle>Approvals</CardTitle>
          <CardDescription>
            {pendingEntries.length === 0
              ? 'No pending approvals'
              : `${pendingEntries.length} pending approval${pendingEntries.length === 1 ? '' : 's'}`}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {state.error ? (
            <p role="alert" className="text-sm text-danger">
              {state.error}
            </p>
          ) : null}

          <ul className="space-y-1" aria-label="Approval list">
            {state.entries.map((entry) => (
              <li key={entry.request.id}>
                <ApprovalListItem
                  entry={entry}
                  selected={entry.request.id === state.selectedId}
                  onSelect={() => onSelect(entry.request.id)}
                />
              </li>
            ))}
          </ul>

          {state.entries.length === 0 ? (
            <p className="rounded-[var(--radius-md)] border border-dashed border-border bg-surface-muted p-4 text-sm text-foreground-muted">
              No approval requests received yet.
            </p>
          ) : null}
        </CardContent>
      </Card>

      <div className="min-w-0">
        {selectedEntry && selectedEntry.state === 'Pending' ? (
          <ApprovalCard
            request={selectedEntry.request}
            onRespond={(decision) => onRespond(selectedEntry.request, decision)}
            onCancel={() => onCancel(selectedEntry.request.id)}
          />
        ) : (
          <Card>
            <CardContent className="flex h-64 items-center justify-center">
              <p className="text-sm text-foreground-muted">
                {selectedEntry
                  ? `This approval has been ${selectedEntry.state.toLowerCase()}.`
                  : 'Select an approval request from the list.'}
              </p>
            </CardContent>
          </Card>
        )}
      </div>
    </div>
  )
}
