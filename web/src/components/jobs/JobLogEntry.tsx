import type { JobProgressEvent } from '../../types/api';

/** Compact console entry; preserve original line breaks and indentation. */
export function JobLogEntry({ event }: { event: JobProgressEvent }) {
  return <div className="job-log-entry min-w-0 whitespace-pre-wrap [overflow-wrap:anywhere]">
    <time className="mr-2 text-[var(--ink-muted)]">[{new Date(event.timestamp * 1000).toLocaleTimeString()}]</time>
    <span className="mr-2 font-medium text-[var(--primary)]">{event.status}</span>
    <span data-log-message="">{event.message || `Progres ${event.progress}%`}</span>
  </div>;
}
