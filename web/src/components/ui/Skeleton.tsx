import React from 'react';

export interface SkeletonProps extends React.HTMLAttributes<HTMLDivElement> {
  className?: string;
}

export const Skeleton: React.FC<SkeletonProps> = ({ className = '', ...props }) => {
  return (
    <div
      className={`animate-pulse rounded-[var(--radius-sm)] bg-[var(--line)]/50 ${className}`}
      {...props}
    />
  );
};
