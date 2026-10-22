/**
 * A panel's first load: a light shimmer in place of its rows, so an empty
 * state is never shown before the data has had its say.
 */
import React from 'react';
import { cn } from '../lib/cn';

/** One shimmering bar. */
export const SkeletonBar: React.FC<{ className?: string }> = ({ className }) => (
  <div className={cn('rounded bg-slate-100 dark:bg-white/5 animate-pulse', className)} />
);

/** `rows` shimmering rows, each `rowClass` tall/round. */
export const SkeletonRows: React.FC<{ rows?: number; rowClass?: string; className?: string; testId?: string }> = ({
  rows = 3, rowClass = 'h-9 rounded-lg', className, testId,
}) => (
  <div className={cn('space-y-1.5', className)} data-testid={testId} aria-busy="true">
    {Array.from({ length: rows }, (_, i) => (
      <SkeletonBar key={i} className={cn(rowClass, i === rows - 1 && 'w-2/3')} />
    ))}
  </div>
);
