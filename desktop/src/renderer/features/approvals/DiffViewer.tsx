import type { FileChangeDiff } from './protocol'

const MAX_DIFF_CHARS = 64 * 1024

function isBinaryDiff(diff: string): boolean {
  return diff.includes('\0') || diff.startsWith('Binary files')
}

export function DiffViewer({ changes }: { changes?: FileChangeDiff[] }) {
  if (!changes || changes.length === 0) {
    return (
      <p className="rounded-[var(--radius-md)] border border-dashed border-border bg-surface-muted p-3 text-sm text-foreground-muted">
        No file changes are available for this approval yet.
      </p>
    )
  }

  return (
    <section className="space-y-3" aria-label="File change diff">
      {changes.map((change) => {
        const large = change.diff.length > MAX_DIFF_CHARS
        const visible = large ? change.diff.slice(0, MAX_DIFF_CHARS) : change.diff
        return (
          <article
            key={`${change.kind}:${change.path}`}
            className="space-y-1 rounded-[var(--radius-md)] border border-border bg-surface p-3"
            aria-label={`Diff for ${change.path}`}
          >
            <p className="font-mono text-xs text-foreground-muted">
              {change.kind}: {change.path}
            </p>
            {isBinaryDiff(change.diff) ? (
              <p className="text-sm text-foreground-muted">
                Binary diff content is hidden for safety.
              </p>
            ) : (
              <pre className="max-h-64 overflow-auto rounded-[var(--radius-sm)] bg-surface-muted p-2 font-mono text-xs whitespace-pre-wrap">
                {visible}
              </pre>
            )}
            {large ? (
              <p className="text-xs text-warning">Diff truncated at {MAX_DIFF_CHARS} characters.</p>
            ) : null}
          </article>
        )
      })}
    </section>
  )
}
