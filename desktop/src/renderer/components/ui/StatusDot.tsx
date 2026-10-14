import { cn } from '../../lib/utils'

const toneClassNames = {
  neutral: 'bg-foreground-muted',
  success: 'bg-success',
  warning: 'bg-warning',
  danger: 'bg-danger',
} as const

export function StatusDot({
  tone = 'neutral',
  label,
}: {
  tone?: keyof typeof toneClassNames
  label: string
}) {
  return (
    <span className="inline-flex items-center gap-2 text-sm text-foreground-muted">
      <span className={cn('h-2 w-2 rounded-full', toneClassNames[tone])} aria-hidden="true" />
      <span>{label}</span>
    </span>
  )
}
