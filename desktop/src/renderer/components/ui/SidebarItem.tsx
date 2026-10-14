import { forwardRef } from 'react'
import { cn } from '../../lib/utils'

export interface SidebarItemProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  active?: boolean
}

export const SidebarItem = forwardRef<HTMLButtonElement, SidebarItemProps>(
  ({ className, active = false, 'aria-current': ariaCurrent = 'page', ...props }, ref) => (
    <button
      ref={ref}
      type="button"
      aria-current={active ? ariaCurrent : undefined}
      className={cn(
        'flex h-10 w-full items-center rounded-[var(--radius-md)] px-3 text-left text-sm font-medium transition-colors',
        active
          ? 'bg-primary/10 text-primary'
          : 'text-foreground-muted hover:bg-surface-muted hover:text-foreground',
        className,
      )}
      {...props}
    />
  ),
)
SidebarItem.displayName = 'SidebarItem'
