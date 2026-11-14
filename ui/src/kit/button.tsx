import type { ButtonHTMLAttributes, ReactNode } from 'react'
import { cx } from './cx'

export type ButtonVariant = 'default' | 'primary' | 'icon'

export interface ButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'type'> {
  /** `primary` is the one accent per screen; `icon` is the 28 × 28 square. */
  variant?: ButtonVariant
  /** Transparent until hovered (the ⋮ trigger, row buttons). */
  quiet?: boolean
  children?: ReactNode
}

export function Button({ variant = 'default', quiet, className, children, ...rest }: ButtonProps) {
  return (
    <button
      type="button"
      className={cx('m-btn', variant === 'primary' && 'm-btn-primary', variant === 'icon' && 'm-btn-icon', quiet && 'is-quiet', className)}
      {...rest}
    >
      {children}
    </button>
  )
}

/** The ⋮ trigger: every secondary action hides behind it (10 §2). */
export function MenuButton({ label = 'More', ...rest }: Omit<ButtonProps, 'variant' | 'children'> & { label?: string }) {
  return (
    <Button variant="icon" quiet aria-label={label} {...rest}>
      ⋮
    </Button>
  )
}

export function ButtonGroup({ children, className }: { children?: ReactNode; className?: string }) {
  return <div className={cx('m-btn-group', className)}>{children}</div>
}
