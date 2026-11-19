import { useState } from 'react';
import { ClipboardCheck, Plus, RefreshCw, FileText, X } from 'lucide-react';
import { Badge, Button, ScrollArea } from '../../../shared/view/ui';
import type { HarnessNodeView, HarnessTaskDetail } from '../../../services/desktop-client';
import { useHarnessTasks } from '../hooks/useHarnessTasks';

type HarnessTasksPanelProps = {
  isVisible: boolean;
};

const STATE_BADGE: Record<string, string> = {
  succeeded: 'bg-emerald-500/15 text-emerald-600 dark:text-emerald-400 border-emerald-500/30',
  failed: 'bg-red-500/15 text-red-600 dark:text-red-400 border-red-500/30',
  blocked: 'bg-red-500/10 text-red-500 border-red-500/20',
  waiting_input: 'bg-amber-500/15 text-amber-600 dark:text-amber-400 border-amber-500/30',
  running: 'bg-blue-500/15 text-blue-600 dark:text-blue-400 border-blue-500/30',
  verifying: 'bg-blue-500/10 text-blue-500 border-blue-500/20',
  ready: 'bg-secondary text-secondary-foreground border-transparent',
  pending: 'bg-secondary text-secondary-foreground border-transparent',
  uncertain: 'bg-amber-500/10 text-amber-500 border-amber-500/20',
  cancelling: 'bg-secondary text-secondary-foreground border-transparent',
};

function stateClass(state: string): string {
  return STATE_BADGE[state] ?? 'bg-secondary text-secondary-foreground border-transparent';
}

function NodeRow({ node }: { node: HarnessNodeView }) {
  return (
    <div className="rounded-md border border-border p-3">
      <div className="flex items-center gap-2">
        <span className="font-medium text-foreground">{node.id}</span>
        <Badge variant="outline" className="text-[10px] uppercase">{node.kind}</Badge>
        <Badge variant="outline" className={`text-[10px] ${stateClass(node.state)}`}>{node.state}</Badge>
        <span className="ml-auto text-xs text-muted-foreground">attempts {node.attempts}</span>
      </div>
      {node.reason && <p className="mt-1 text-xs text-muted-foreground">{node.reason}</p>}
      {node.gates.length > 0 && (
        <div className="mt-2 space-y-1">
          {node.gates.map((g) => (
            <div key={g.id} className="flex flex-wrap items-center gap-2 text-xs">
              <span className={g.proven ? 'text-emerald-600 dark:text-emerald-400' : 'text-muted-foreground'}>
                {g.proven ? '✓ proven' : '· unproven'}
              </span>
              <span className="font-mono text-foreground">{g.id}</span>
              {g.exit_code !== null && <span className="text-muted-foreground">exit {g.exit_code}</span>}
              {g.evidence_ref && (
                <span className="truncate font-mono text-muted-foreground" title={g.evidence_ref}>
                  {g.evidence_ref}
                </span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function TaskDetailView({
  detail,
  artifactView,
  onReadArtifact,
  onCloseArtifact,
}: {
  detail: HarnessTaskDetail;
  artifactView: { id: string; content: string } | null;
  onReadArtifact: (artifactId: string) => void;
  onCloseArtifact: () => void;
}) {
  const ordered = detail.order
    .map((id) => detail.nodes.find((n) => n.id === id))
    .filter((n): n is HarnessNodeView => Boolean(n));
  const nodes = ordered.length > 0 ? ordered : detail.nodes;

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 border-b border-border px-4 py-3">
        <ClipboardCheck className="h-4 w-4 text-muted-foreground" />
        <span className="font-medium text-foreground">{detail.title}</span>
        <Badge variant="outline" className={`text-[10px] ${stateClass(detail.state)}`}>{detail.state}</Badge>
        <span className="ml-auto text-xs text-muted-foreground">
          {detail.tokens_used} tokens · {detail.wall_secs}s
        </span>
      </div>

      <ScrollArea className="flex-1">
        <div className="space-y-4 p-4">
          <section>
            <h3 className="mb-2 text-xs font-semibold uppercase text-muted-foreground">Nodes</h3>
            <div className="space-y-2">
              {nodes.map((n) => <NodeRow key={n.id} node={n} />)}
            </div>
          </section>

          {detail.artifacts.length > 0 && (
            <section>
              <h3 className="mb-2 text-xs font-semibold uppercase text-muted-foreground">Artifacts</h3>
              <div className="flex flex-wrap gap-2">
                {detail.artifacts.map((a) => (
                  <Button
                    key={a.id}
                    variant="outline"
                    size="sm"
                    className="h-7 gap-1 text-xs"
                    onClick={() => onReadArtifact(a.id)}
                  >
                    <FileText className="h-3 w-3" />
                    {a.name}
                  </Button>
                ))}
              </div>
            </section>
          )}

          {artifactView && (
            <section className="rounded-md border border-border">
              <div className="flex items-center gap-2 border-b border-border px-3 py-2">
                <span className="font-mono text-xs text-foreground">{artifactView.id}</span>
                <Button variant="ghost" size="icon" className="ml-auto h-6 w-6" onClick={onCloseArtifact}>
                  <X className="h-3 w-3" />
                </Button>
              </div>
              <pre className="max-h-64 overflow-auto whitespace-pre-wrap p-3 text-xs text-foreground">
                {artifactView.content}
              </pre>
            </section>
          )}

          <section>
            <h3 className="mb-2 text-xs font-semibold uppercase text-muted-foreground">Report</h3>
            <pre className="whitespace-pre-wrap rounded-md border border-border bg-muted/30 p-3 text-xs text-foreground">
              {detail.report}
            </pre>
          </section>
        </div>
      </ScrollArea>
    </div>
  );
}

export default function HarnessTasksPanel({ isVisible }: HarnessTasksPanelProps) {
  const {
    tasks, selected, isLoading, isCreating, error, artifactView,
    clearError, clearArtifact, refreshList, selectTask, createTask, readArtifact,
  } = useHarnessTasks(isVisible);
  const [mode, setMode] = useState('success');

  if (!isVisible) return null;

  return (
    <div className="flex h-full bg-background">
      <div className="flex w-64 shrink-0 flex-col border-r border-border">
        <div className="flex items-center gap-2 border-b border-border px-3 py-2">
          <span className="text-sm font-medium text-foreground">Harness Tasks</span>
          <Button variant="ghost" size="icon" className="ml-auto h-7 w-7" onClick={() => void refreshList()}>
            <RefreshCw className="h-3.5 w-3.5" />
          </Button>
        </div>

        <div className="space-y-2 border-b border-border p-3">
          <select
            value={mode}
            onChange={(e) => setMode(e.target.value)}
            className="w-full rounded-md border border-input bg-background px-2 py-1 text-xs text-foreground"
          >
            <option value="success">Fixture: success</option>
            <option value="verify_fails">Fixture: verify fails</option>
            <option value="needs_input">Fixture: needs input</option>
          </select>
          <Button
            size="sm"
            className="w-full gap-1"
            disabled={isCreating}
            onClick={() => void createTask('Team delivery', mode)}
          >
            <Plus className="h-3.5 w-3.5" />
            {isCreating ? 'Running…' : 'New Task'}
          </Button>
        </div>

        <ScrollArea className="flex-1">
          <div className="space-y-1 p-2">
            {tasks.length === 0 && (
              <p className="px-2 py-4 text-center text-xs text-muted-foreground">
                No business runs yet. Create a task to see node status and gate evidence.
              </p>
            )}
            {tasks.map((t) => (
              <button
                key={t.id}
                onClick={() => void selectTask(t.id)}
                className={`w-full rounded-md border px-3 py-2 text-left transition-colors ${
                  selected?.id === t.id ? 'border-primary bg-accent' : 'border-transparent hover:bg-accent/50'
                }`}
              >
                <div className="flex items-center gap-2">
                  <span className="truncate text-sm text-foreground">{t.title}</span>
                  <Badge variant="outline" className={`ml-auto shrink-0 text-[10px] ${stateClass(t.state)}`}>
                    {t.state}
                  </Badge>
                </div>
                <span className="text-xs text-muted-foreground">
                  {t.succeeded_count}/{t.node_count} nodes · {t.workflow_id}
                </span>
              </button>
            ))}
          </div>
        </ScrollArea>
      </div>

      <div className="min-w-0 flex-1">
        {error && (
          <div className="flex items-center gap-2 border-b border-destructive/30 bg-destructive/10 px-4 py-2 text-xs text-destructive">
            <span className="flex-1">{error}</span>
            <Button variant="ghost" size="icon" className="h-6 w-6" onClick={clearError}>
              <X className="h-3 w-3" />
            </Button>
          </div>
        )}
        {isLoading && (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">Loading…</div>
        )}
        {!isLoading && selected && (
          <TaskDetailView
            detail={selected}
            artifactView={artifactView}
            onReadArtifact={(id) => void readArtifact(selected.id, id)}
            onCloseArtifact={clearArtifact}
          />
        )}
        {!isLoading && !selected && (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-muted-foreground">
            <ClipboardCheck className="h-8 w-8" />
            <p className="text-sm">Select a task or create a new business run.</p>
            <p className="max-w-sm text-center text-xs">
              Node success is proven only by gate evidence (real exit codes / artifacts),
              managed by the Rust harness — never by chat or TaskMaster identifiers.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}