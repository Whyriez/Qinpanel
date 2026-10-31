import React, { useEffect, useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import {
  ListTodo,
  ChevronDown,
  ChevronUp,
  X,
  ExternalLink,
  Ban,
} from 'lucide-react';
import { JobItem, JobProgressEvent } from '../../types/api';
import { fetchJobs, cancelJob, subscribeJobEvents } from '../../api/client';
import { StatusPill, StatusVariant } from '../ui/StatusPill';
import { useAuth } from '../../context/AuthContext';
import { canCancelJob } from '../../lib/jobProgress';

export interface JobTrayProps {
  onNavigateToJobs?: () => void;
}

export const JobTray: React.FC<JobTrayProps> = ({ onNavigateToJobs }) => {
  const { t } = useTranslation();
  const { user } = useAuth();
  const tenant = user?.role !== 'admin';
  const [isOpen, setIsOpen] = useState(false);
  const [jobs, setJobs] = useState<JobItem[]>([]);
  const [loading, setLoading] = useState(false);

  const activeCount = jobs.filter((j) =>
    ['queued', 'running'].includes(j.status)
  ).length;

  const loadRecentJobs = useCallback(async () => {
    try {
      setLoading(true);
      const res = await fetchJobs({ page: 1, per_page: 6 });
      setJobs(res.items);
    } catch {
      // ignore in tray
    } finally {
      setLoading(false);
    }
  }, []);

  // Poll jobs when tray is open or when there are active jobs
  useEffect(() => {
    loadRecentJobs();
    const interval = setInterval(() => {
      loadRecentJobs();
    }, activeCount > 0 ? 3000 : 10000);

    return () => clearInterval(interval);
  }, [loadRecentJobs, activeCount]);

  // Subscribe to SSE for the top active job if running
  useEffect(() => {
    const runningJob = jobs.find((j) => j.status === 'running' || j.status === 'queued');
    if (!runningJob) return;

    const unsubscribe = subscribeJobEvents(
      runningJob.id,
      (event: JobProgressEvent) => {
        setJobs((prev) =>
          prev.map((j) => {
            if (j.id === event.job_id) {
              return {
                ...j,
                status: event.status,
                progress: event.progress,
                error_message: event.status === 'failed' ? event.message : j.error_message,
              };
            }
            return j;
          })
        );
      }
    );

    return () => {
      unsubscribe();
    };
  }, [jobs]);

  const handleCancelJob = async (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      const updated = await cancelJob(id);
      setJobs((prev) => prev.map((j) => (j.id === id ? updated : j)));
    } catch {
      // ignore
    }
  };

  const getStatusVariant = (status: string): StatusVariant => {
    switch (status) {
      case 'succeeded':
        return 'success';
      case 'failed':
        return 'danger';
      case 'running':
        return 'info';
      case 'cancelled':
        return 'neutral';
      default:
        return 'warning';
    }
  };

  if (activeCount === 0 && !isOpen) return null;

  return (
    <div className="hosting-job-tray fixed bottom-4 right-6 z-40 flex flex-col items-end">
      {/* Expanded Popover */}
      {isOpen && (
        <div className="mb-3 w-84 sm:w-96 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] shadow-[var(--shadow-floating)] overflow-hidden transition-all duration-200">
          <div className="px-4 py-3 border-b border-[var(--line)] flex items-center justify-between bg-[var(--raised)]">
            <div className="flex items-center gap-2">
              <ListTodo className="w-4 h-4 text-[var(--primary)]" />
              <span className="font-semibold text-xs text-[var(--ink)]">
                {t(tenant ? 'activity.trayTitle' : 'jobs.trayTitle')}
              </span>
              {activeCount > 0 && (
                <span className="px-1.5 py-0.2 text-[10px] font-bold rounded-full bg-[var(--primary)] text-white">
                  {activeCount}
                </span>
              )}
            </div>
            <div className="flex items-center gap-1">
              <button
                onClick={() => setIsOpen(false)}
                className="p-1 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface)]"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>

          <div className="max-h-72 overflow-y-auto divide-y divide-[var(--line)] p-1">
            {jobs.length === 0 ? (
              <div className="py-8 text-center text-xs text-[var(--ink-muted)]">
                {loading ? t('common.loading') : t(tenant ? 'activity.noJobs' : 'jobs.noJobs')}
              </div>
            ) : (
              jobs.map((job) => (
                <div key={job.id} className="p-3 space-y-1.5 hover:bg-[var(--raised)]/50 rounded-[var(--radius-sm)] transition-colors">
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-mono text-xs font-semibold text-[var(--ink)] truncate max-w-[200px]">
                      {job.name}
                    </span>
                    <StatusPill
                      label={job.status}
                      variant={getStatusVariant(job.status)}
                      pulse={job.status === 'running'}
                    />
                  </div>

                  {/* Progress bar */}
                  <div className="space-y-1">
                    <div className="flex items-center justify-between text-[11px] text-[var(--ink-muted)] font-mono">
                      <span>{job.progress}%</span>
                      {canCancelJob(job) && (
                        <button
                          onClick={(e) => handleCancelJob(job.id, e)}
                          className="flex items-center gap-1 text-[10px] text-[var(--danger)] hover:underline"
                          title="Batalkan proses ini"
                        >
                          <Ban className="w-3 h-3" />
                          <span>Batal</span>
                        </button>
                      )}
                    </div>
                    <div className="h-1.5 w-full bg-[var(--line)] rounded-full overflow-hidden">
                      <div
                        className={`h-full transition-all duration-300 ${
                          job.status === 'succeeded'
                            ? 'bg-[var(--success)]'
                            : job.status === 'failed'
                            ? 'bg-[var(--danger)]'
                            : 'bg-[var(--primary)]'
                        }`}
                        style={{ width: `${job.progress}%` }}
                      />
                    </div>
                  </div>

                  {job.error_message && (
                    <p className="text-[11px] text-[var(--danger)] leading-tight">
                      {job.error_message}
                    </p>
                  )}
                </div>
              ))
            )}
          </div>

          {onNavigateToJobs && (
            <div className="p-2 border-t border-[var(--line)] bg-[var(--raised)] text-center">
              <button
                onClick={() => {
                  setIsOpen(false);
                  onNavigateToJobs();
                }}
                className="w-full flex items-center justify-center gap-1.5 py-1 text-xs font-medium text-[var(--primary)] hover:underline"
              >
                <span>{t(tenant ? 'activity.viewAll' : 'jobs.viewAll')}</span>
                <ExternalLink className="w-3 h-3" />
              </button>
            </div>
          )}
        </div>
      )}

      {/* Floating Pill Trigger */}
      <button
        onClick={() => setIsOpen(!isOpen)}
        className="flex items-center gap-2.5 px-3.5 py-2 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-full)] shadow-[var(--shadow-floating)] text-xs text-[var(--ink-muted)] hover:border-[var(--primary)]/40 hover:text-[var(--ink)] transition-all"
        title={t(tenant ? 'activity.trayHint' : 'jobs.trayHint')}
      >
        <span
          className={`w-2 h-2 rounded-full ${
            activeCount > 0
              ? 'bg-[var(--primary)] animate-pulse'
              : 'bg-[var(--success)]'
          }`}
        />
        <span className="font-semibold text-[var(--ink)]">
          {t('jobs.trayActiveCount', { label: t(tenant ? 'nav.activity' : 'nav.jobs'), count: activeCount })}
        </span>
        {isOpen ? (
          <ChevronDown className="w-3.5 h-3.5 text-[var(--ink-muted)]" />
        ) : (
          <ChevronUp className="w-3.5 h-3.5 text-[var(--ink-muted)]" />
        )}
      </button>
    </div>
  );
};
