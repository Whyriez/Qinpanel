import React from 'react';

export type StatusVariant = 'success' | 'danger' | 'warning' | 'info' | 'neutral';

export interface StatusPillProps {
  variant: StatusVariant;
  label: string;
  size?: 'sm' | 'md';
  pulse?: boolean;
  className?: string;
}

export const StatusPill: React.FC<StatusPillProps> = ({
  variant,
  label,
  size = 'md',
  pulse = false,
  className = '',
}) => {
  const dotColorClass = {
    success: 'bg-[var(--success)]',
    danger: 'bg-[var(--danger)]',
    warning: 'bg-[var(--accent)]',
    info: 'bg-[var(--info)]',
    neutral: 'bg-[var(--ink-muted)]',
  }[variant];

  const bgColorClass = {
    success: 'bg-[var(--success)]/10 text-[var(--success)] border-[var(--success)]/20',
    danger: 'bg-[var(--danger)]/10 text-[var(--danger)] border-[var(--danger)]/20',
    warning: 'bg-[var(--accent)]/15 text-[var(--warning-ink)] border-[var(--accent)]/25',
    info: 'bg-[var(--info)]/10 text-[var(--info)] border-[var(--info)]/20',
    neutral: 'bg-[var(--line)]/50 text-[var(--ink-muted)] border-[var(--line)]',
  }[variant];

  const sizeClass = size === 'sm' ? 'px-2 py-0.5 text-xs' : 'px-2.5 py-1 text-xs';

  return (
    <span
      className={`inline-flex items-center gap-1.5 font-medium border rounded-[var(--radius-full)] ${sizeClass} ${bgColorClass} ${className}`}
    >
      <span className="relative flex h-2 w-2">
        {pulse && (
          <span
            className={`animate-ping absolute inline-flex h-full w-full rounded-full opacity-75 ${dotColorClass}`}
          />
        )}
        <span className={`relative inline-flex rounded-full h-2 w-2 ${dotColorClass}`} />
      </span>
      <span>{label}</span>
    </span>
  );
};
