import { ModalOverlay } from '../components/ui/ModalOverlay';
import React, { useEffect, useState, useCallback, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import {
  RefreshCw,
  Ban,
  Radio,
  X,
  AlertCircle,
  ChevronLeft,
  ChevronRight,
} from 'lucide-react';
import { JobItem, JobProgressEvent } from '../types/api';
import { fetchJobs, fetchJobDetail, fetchJobLogs, cancelJob, subscribeJobEvents } from '../api/client';
import { JobQueueNotice } from '../components/jobs/JobQueueNotice';
import { JobLogEntry } from '../components/jobs/JobLogEntry';
import { jobProgressMessage, jobSnapshotEvent, mergeJobEvents, canCancelJob, jobTitle, jobProgressLabel, isServerBackupJob, serverBackupJobDetail } from '../lib/jobProgress';
import { StatusPill, StatusVariant } from '../components/ui/StatusPill';
import { EmptyState } from '../components/ui/EmptyState';

export const JobsPage: React.FC = () => {
  const { t } = useTranslation();
  const [jobs, setJobs] = useState<JobItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Filters & Pagination
  const [page, setPage] = useState(1);
  const [perPage] = useState(15);
  const [totalPages, setTotalPages] = useState(1);
  const [totalItems, setTotalItems] = useState(0);
  const [selectedStatus, setSelectedStatus] = useState<string>('');

  // Live SSE streaming inspect modal
  const [activeStreamJob, setActiveStreamJob] = useState<JobItem | null>(null);
  const [streamEvents, setStreamEvents] = useState<JobProgressEvent[]>([]);
  const [streamError, setStreamError] = useState('');
  const [followLog, setFollowLog] = useState(true);
  const logView = useRef<HTMLDivElement>(null);
  const listRevision = useRef(0);
  const listPending = useRef<number | null>(null);

  const loadJobs = useCallback(async (quiet = false) => {
    if (quiet && listPending.current !== null) return;
    const revision = ++listRevision.current;
    listPending.current = revision;
    if (!quiet) setLoading(true);
    try {
      const res = await fetchJobs({
        page,
        per_page: perPage,
        status: selectedStatus || undefined,
      });
      if (revision !== listRevision.current) return;
      setError(null);
      setJobs(res.items);
      setTotalPages(res.total_pages || 1);
      setTotalItems(res.total);
    } catch (err: unknown) {
      if (revision === listRevision.current) setError(err instanceof Error ? err.message : 'Gagal memuat daftar tugas.');
    } finally {
      if (listPending.current === revision) {
        listPending.current = null;
        setLoading(false);
      }
    }
  }, [page, perPage, selectedStatus]);

  useEffect(() => {
    void loadJobs();
    const timer = setInterval(() => { void loadJobs(true); }, 3000);
    return () => { listRevision.current++; listPending.current = null; clearInterval(timer); };
  }, [loadJobs]);

  useEffect(() => {
    const id = new URLSearchParams(window.location.search).get('job');
    if (!id) return;
    let active = true;
    void fetchJobDetail(id).then(job => {
      if (active) { setActiveStreamJob(job); setStreamEvents([jobSnapshotEvent(job)]); }
    }).catch(() => { if (active) setError('Tugas yang ditautkan tidak dapat dimuat.'); });
    return () => { active = false; };
  }, []);

  const handleCancel = async (id: string) => {
    try {
      const updated = await cancelJob(id);
      listRevision.current++;
      setJobs((prev) => prev.map((j) => (j.id === id ? updated : j)));
      if (activeStreamJob && activeStreamJob.id === id) {
        setActiveStreamJob(updated);
      }
    } catch (err: unknown) {
      alert(err instanceof Error ? err.message : 'Gagal membatalkan tugas');
    }
  };

  const handleOpenStreamModal = (job: JobItem) => {
    setFollowLog(true);
    setActiveStreamJob(job);
    setStreamError('');
    setStreamEvents([jobSnapshotEvent(job)]);
  };

  useEffect(() => {
    if (followLog && logView.current) logView.current.scrollTop = logView.current.scrollHeight;
  }, [streamEvents, followLog]);

  // SSE subscription effect
  useEffect(() => {
    if (!activeStreamJob) return;
    let active = true;
    const jobId = activeStreamJob.id;
    let revision = 0;
    let terminal = false;
    let checking = false;
    let logCursor = 0;
    const apply = (incoming: JobProgressEvent, snapshot: JobItem | null = null) => {
        const event = { ...incoming, message: jobProgressMessage(incoming.status, incoming.message) };
        if (!active || event.job_id !== jobId) return;
        if (terminal && !['succeeded', 'failed', 'cancelled'].includes(event.status)) return;
        terminal = ['succeeded', 'failed', 'cancelled'].includes(event.status);
        setStreamError('');
        setStreamEvents((prev) => {
          const last = prev.at(-1);
          // Polling only adds state changes; the opening SSE snapshot is deduplicated.
          if (event.sequence === undefined && last && last.job_id === event.job_id && last.status === event.status &&
            last.progress === event.progress && (snapshot ||
              (last.timestamp === event.timestamp && last.message === event.message))) return prev;
          return mergeJobEvents(prev, [event]);
        });
        const update = (current: JobItem) => ({
          ...current, status: event.status, progress: event.progress, updated_at: event.timestamp,
          result_json: snapshot?.result_json ?? current.result_json,
          error_message: ['queued', 'failed'].includes(event.status) ? snapshot ? snapshot.error_message : incoming.message : null,
        });
        setActiveStreamJob((current: JobItem | null) =>
          current?.id === event.job_id ? update(current) : current
        );
        setJobs((prev) =>
          prev.map((job) => job.id === event.job_id ? update(job) : job)
        );
    };
    const unsubscribe = subscribeJobEvents(jobId, event => {
      revision++;
      listRevision.current++;
      apply(event);
    });
    const check = async () => {
      if (checking) return;
      checking = true;
      const before = revision;
      try {
        const [snapshot, history] = await Promise.allSettled([
          terminal ? Promise.resolve(null) : fetchJobDetail(jobId),
          fetchJobLogs(jobId, logCursor),
        ]);
        if (!active) return;
        if (history.status === 'fulfilled') {
          setStreamEvents(prev => mergeJobEvents(prev, history.value));
          logCursor = Math.max(logCursor, ...history.value.map(event => event.sequence ?? 0));
        }
        // A slow snapshot must not overwrite a newer event or a closed modal.
        if (snapshot.status === 'fulfilled' && snapshot.value && revision === before) apply(jobSnapshotEvent(snapshot.value), snapshot.value);
        setStreamError(history.status === 'rejected' ? 'Riwayat log belum dapat dimuat. Pemantauan akan dicoba kembali.' : snapshot.status === 'rejected' ? 'Pembaruan status belum dapat dimuat. Pemantauan akan dicoba kembali.' : '');
      } catch {
        if (active && revision === before) setStreamError('Pembaruan status belum dapat dimuat. Pemantauan akan dicoba kembali.');
      } finally { checking = false; }
    };
    void check();
    const timer = setInterval(() => { void check(); }, 3000);

    return () => {
      active = false;
      clearInterval(timer);
      unsubscribe();
    };
  }, [activeStreamJob?.id]);

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

  const formatTimestamp = (ts: number) => {
    return new Date(ts * 1000).toLocaleString('id-ID', {
      dateStyle: 'medium',
      timeStyle: 'medium',
    });
  };

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h1 className="text-xl font-bold tracking-tight text-[var(--ink)]">
            {t('jobs.title')}
          </h1>
          <p className="text-xs text-[var(--ink-muted)] mt-1">
            {t('jobs.subtitle')}
          </p>
        </div>

        <button
          onClick={() => { void loadJobs(); }}
          disabled={loading}
          className="self-start sm:self-auto py-1.5 px-3 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] text-xs font-medium flex items-center gap-1.5 cursor-pointer"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
          <span>{t('common.refresh')}</span>
        </button>
      </div>

      {error && (
        <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--danger)]/10 border border-[var(--danger)]/30 flex items-center gap-2 text-xs text-[var(--danger)]">
          <AlertCircle className="w-4 h-4 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {/* Filter Bar */}
      <div className="flex items-center justify-between bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-md)] p-3">
        <div className="flex items-center gap-2">
          <select
            value={selectedStatus}
            onChange={(e) => {
              setSelectedStatus(e.target.value);
              setPage(1);
            }}
            className="bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] px-2.5 py-1.5 text-xs text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          >
            <option value="">Semua Status</option>
            <option value="queued">Queued (Antre)</option>
            <option value="running">Running (Berjalan)</option>
            <option value="succeeded">Succeeded (Berhasil)</option>
            <option value="failed">Failed (Gagal)</option>
            <option value="cancelled">Cancelled (Dibatalkan)</option>
          </select>
        </div>

        <div className="text-xs text-[var(--ink-muted)]">
          Total: <span className="font-semibold text-[var(--ink)]">{totalItems}</span> tugas
        </div>
      </div>

      {/* Jobs Table */}
      <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-md)] overflow-hidden">
        <div className="overflow-x-auto">
          <table className="w-full text-left border-collapse text-xs">
            <thead>
              <tr className="border-b border-[var(--line)] bg-[var(--raised)]/50 text-[var(--ink-muted)] font-medium">
                <th className="py-2.5 px-4">Nama Tugas</th>
                <th className="py-2.5 px-4 w-44">Progres</th>
                <th className="py-2.5 px-4">Status</th>
                <th className="py-2.5 px-4">Percobaan</th>
                <th className="py-2.5 px-4">Waktu Mulai</th>
                <th className="py-2.5 px-4 text-right">Aksi</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[var(--line)] text-[var(--ink)]">
              {jobs.length === 0 ? (
                <tr>
                  <td colSpan={6} className="py-12 text-center text-xs text-[var(--ink-muted)]">
                    <EmptyState
                      title="Tidak ada tugas"
                      description="Belum ada operasi latar belakang yang dijalankan atau cocok dengan filter."
                    />
                  </td>
                </tr>
              ) : (
                jobs.map((job) => (
                  <tr key={job.id} className="hover:bg-[var(--raised)]/40 transition-colors">
                    <td className="py-2.5 px-4">
                      <div className="flex flex-col">
                        <span className="font-semibold text-xs text-[var(--ink)]">
                          {jobTitle(job.name)}
                        </span>
                        <span className="font-mono text-[10px] text-[var(--ink-muted)]">
                          {job.id}
                        </span>
                      </div>
                    </td>
                    <td className="py-2.5 px-4">
                      <div className="space-y-1">
                        <div className="flex justify-between text-[10px] font-mono text-[var(--ink-muted)]">
                          <span>{jobProgressLabel(job)}</span>
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
                    </td>
                    <td className="py-2.5 px-4 whitespace-nowrap">
                      <StatusPill
                        label={job.status}
                        variant={getStatusVariant(job.status)}
                        pulse={job.status === 'running'}
                      />
                    </td>
                    <td className="py-2.5 px-4 font-mono text-[11px] text-[var(--ink-muted)] whitespace-nowrap">
                      {job.retry_count} / {job.max_retries}
                    </td>
                    <td className="py-2.5 px-4 font-mono text-[11px] text-[var(--ink-muted)] whitespace-nowrap">
                      {formatTimestamp(job.created_at)}
                    </td>
                    <td className="py-2.5 px-4 text-right whitespace-nowrap">
                      <div className="flex items-center justify-end gap-2">
                        <button
                          onClick={() => handleOpenStreamModal(job)}
                          className="p-1 rounded text-[var(--primary)] hover:bg-[var(--primary)]/10 cursor-pointer"
                          title="Monitor Progres Real-Time (SSE)"
                        >
                          <Radio className="w-3.5 h-3.5" />
                        </button>
                        {canCancelJob(job) && (
                          <button
                            onClick={() => handleCancel(job.id)}
                            className="p-1 rounded text-[var(--danger)] hover:bg-[var(--danger)]/10 cursor-pointer"
                            title="Batalkan Tugas"
                          >
                            <Ban className="w-3.5 h-3.5" />
                          </button>
                        )}
                      </div>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>

        {/* Pagination Footer */}
        {totalPages > 1 && (
          <div className="flex items-center justify-between px-4 py-3 border-t border-[var(--line)] bg-[var(--raised)]/30 text-xs">
            <span className="text-[var(--ink-muted)]">
              Halaman {page} dari {totalPages}
            </span>
            <div className="flex items-center gap-1.5">
              <button
                disabled={page <= 1}
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                className="p-1 rounded border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] disabled:opacity-40 cursor-pointer"
              >
                <ChevronLeft className="w-3.5 h-3.5" />
              </button>
              <button
                disabled={page >= totalPages}
                onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                className="p-1 rounded border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] disabled:opacity-40 cursor-pointer"
              >
                <ChevronRight className="w-3.5 h-3.5" />
              </button>
            </div>
          </div>
        )}
      </div>

      {/* SSE Live Streaming Modal */}
      {activeStreamJob && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm"
          role="dialog"
          aria-modal="true"
          aria-labelledby="job-monitor-title"
          onClick={() => setActiveStreamJob(null)}
          onDismiss={() => setActiveStreamJob(null)}
        >
          <div
            className="w-full max-w-4xl bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] shadow-[var(--shadow-floating)] p-5 space-y-4 dialog-panel"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between border-b border-[var(--line)] pb-3 dialog-header">
              <div className="flex items-center gap-2">
                <Radio className="w-4 h-4 text-[var(--primary)] animate-pulse" />
                <h3 id="job-monitor-title" className="font-semibold text-xs text-[var(--ink)]">Live Monitor: {jobTitle(activeStreamJob.name)}</h3>
              </div>
              <button
                onClick={() => setActiveStreamJob(null)}
                aria-label="Tutup monitor"
                className="p-1 rounded text-[var(--ink-muted)] hover:text-[var(--ink)] cursor-pointer"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
            <div className="dialog-body space-y-4">
              <div className="space-y-2">
                <div className="flex items-center justify-between text-xs">
                  <span className="text-[var(--ink-muted)]">Status Terkini:</span>
                  <StatusPill
                    label={activeStreamJob.status}
                    variant={getStatusVariant(activeStreamJob.status)}
                    pulse={activeStreamJob.status === 'running'}
                  />
                </div>

                <div className="space-y-1">
                  <div className="flex justify-between text-xs font-mono text-[var(--ink-muted)]">
                    <span>Progres:</span>
                    <span>{jobProgressLabel(activeStreamJob)}</span>
                  </div>
                  <div className="h-2 w-full bg-[var(--line)] rounded-full overflow-hidden">
                    <div
                      className="h-full bg-[var(--primary)] transition-all duration-300"
                      style={{ width: `${activeStreamJob.progress}%` }}
                    />
                  </div>
                </div>
              </div>

              {activeStreamJob.status === 'queued' && !isServerBackupJob(activeStreamJob.name) && <JobQueueNotice jobId={activeStreamJob.id} />}
              {isServerBackupJob(activeStreamJob.name)&&<p className="text-xs text-[var(--ink-muted)]">Worker server menjalankan tugas ini secara terpisah. Panel dapat terputus selama snapshot; status dilanjutkan setelah layanan kembali. Tugas ini tidak dapat dibatalkan dari antrean.</p>}
              {isServerBackupJob(activeStreamJob.name)&&<p role="status" className="text-sm text-[var(--ink)]">{serverBackupJobDetail(activeStreamJob).message}</p>}
              {streamError && <p role="alert" className="text-xs text-[var(--ink-muted)]">{streamError}</p>}

              <div className="space-y-1.5">
                <div className="flex flex-wrap items-center justify-between gap-2 text-[11px] text-[var(--ink-muted)]">
                  <span className="font-medium">Log instalasi dan progres:</span>
                  <label className="inline-flex items-center gap-2 py-2 cursor-pointer"><input type="checkbox" checked={followLog} onChange={event => setFollowLog(event.target.checked)} />Ikuti log terbaru</label>
                </div>
                <div ref={logView} role="log" aria-label="Log progres tugas" tabIndex={0} onScroll={event => {
                  const view = event.currentTarget;
                  if (view.scrollHeight - view.scrollTop - view.clientHeight >= 40) setFollowLog(false);
                }} className="min-w-0 max-h-[45vh] overflow-y-auto p-3 bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] font-mono text-xs leading-[18px] text-[var(--ink)] focus-visible:outline-2 focus-visible:outline-[var(--primary)]">
                  {streamEvents.map((ev, i) => (
                    <JobLogEntry key={i} event={ev} />
                  ))}
                </div>
              </div>

              {canCancelJob(activeStreamJob) && (
                <div className="pt-2 text-right">
                  <button
                    onClick={() => handleCancel(activeStreamJob.id)}
                    className="py-1 px-3 rounded bg-[var(--danger)] text-white text-xs font-semibold cursor-pointer"
                  >
                    Batalkan Tugas
                  </button>
                </div>
              )}
            </div>
          </div>
        </ModalOverlay>
      )}
    </div>
  );
};
