import React from 'react';
import { LucideIcon } from 'lucide-react';

export interface EmptyStateProps {
  icon?: LucideIcon;
  title: string;
  description: string;
  actionLabel?: string;
  onAction?: () => void;
  className?: string;
}

export const EmptyState: React.FC<EmptyStateProps> = ({
  icon: Icon,
  title,
  description,
  actionLabel,
  onAction,
  className = '',
}) => {
  return (
    <div
      className={`flex flex-col items-center justify-center p-8 text-center border border-dashed border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--surface)] ${className}`}
    >
      {Icon && (
        <div className="w-12 h-12 mb-3 rounded-[var(--radius-full)] bg-[var(--raised)] border border-[var(--line)] flex items-center justify-center text-[var(--ink-muted)]">
          <Icon className="w-6 h-6" />
        </div>
      )}
      <h3 className="text-sm font-semibold text-[var(--ink)] mb-1">{title}</h3>
      <p className="text-xs text-[var(--ink-muted)] max-w-sm mb-4">{description}</p>
      {actionLabel && onAction && (
        <button
          onClick={onAction}
          className="inline-flex items-center justify-center px-4 py-2 text-xs font-medium text-[var(--primary-foreground)] bg-[var(--primary)] hover:bg-[var(--primary-hover)] rounded-[var(--radius-sm)] transition-colors focus:outline-none focus:ring-2 focus:ring-[var(--primary)] focus:ring-offset-2"
        >
          {actionLabel}
        </button>
      )}
    </div>
  );
};
