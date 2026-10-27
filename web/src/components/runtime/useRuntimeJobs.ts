import { useCallback, useEffect, useRef, useState } from 'react';
import { fetchJobDetail, fetchJobs } from '../../api/client';
import type { JobItem } from '../../types/api';
import { toast } from 'sonner';

const runtimeNames = new Set(['php.install', 'php.remove', 'php.repo.setup', 'node.install', 'python.install']);
const isActive = (job: JobItem) => job.status === 'queued' || job.status === 'running';

export function runtimeJobVersion(job: JobItem): string | undefined {
  try {
    const payload: unknown = JSON.parse(job.payload_json);
    if (typeof payload !== 'object' || payload === null || !('version' in payload)) return;
    return typeof payload.version === 'string' ? payload.version.trim().replace(/^v/, '') : undefined;
  } catch { return; }
}

/** Recover server jobs on entry; retain accepted jobs while a list request is in flight. */
export function useRuntimeJobs(onFinished: (job: JobItem) => Promise<boolean>, names: ReadonlySet<string> = runtimeNames) {
  const tracked = useRef(new Map<string, JobItem>());
  const notified = useRef(new Set<string>());
  const [jobs, setJobs] = useState<JobItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');

  const track = useCallback((job: JobItem) => {
    tracked.current.set(job.id, job);
    setJobs([...tracked.current.values()]);
  }, []);

  useEffect(() => {
    let active = true;
    let checking = false;
    const check = async () => {
      if (checking) return;
      checking = true;
      const previous = new Map(tracked.current);
      try {
        const lists = await Promise.all(['queued', 'running'].map(async status => {
          const items: JobItem[] = [];
          let page = 1;
          let totalPages = 1;
          do {
            const result = await fetchJobs({ status, page, per_page: 100 });
            items.push(...result.items.filter(job => names.has(job.name) && isActive(job)));
            totalPages = result.total_pages;
            page++;
          } while (page <= totalPages);
          return items;
        }));
        const current = new Map(lists.flat().map(job => [job.id, job]));
        const missing = [...previous.values()].filter(job => !current.has(job.id));
        const details = await Promise.all(missing.map(async job => {
          if (!isActive(job)) return job;
          try { return await fetchJobDetail(job.id); } catch { return job; }
        }));
        if (!active) return;
        for (const job of details) {
          if (isActive(job)) current.set(job.id, job);
          else {
            if (!notified.current.has(job.id)) {
              notified.current.add(job.id);
              const label = job.name === 'storage.tls.issue' ? 'Sertifikat storage' : job.name === 'ssl.dns01.issue' ? 'Sertifikat DNS-01' : job.name === 'panel.tls.issue' ? 'Sertifikat panel' : job.name.startsWith('email.') ? 'Tugas email' : job.name.startsWith('cloudflare.') ? 'Tugas Cloudflare' : job.name === 'database.web.install' ? 'Pemasangan pengelola database' : job.name === 'database.server.install' ? 'Pemasangan database' : job.name === 'python.install' ? 'Pemasangan Python' : job.name === 'node.install' ? 'Pemasangan Node.js' : 'Tugas PHP';
              if (job.status === 'failed') toast.error(`${label} gagal`, { description: job.error_message || 'Lihat rincian pada halaman Tugas.' });
              if (job.status === 'succeeded') toast.success(`${label} selesai`);
              if (job.status === 'cancelled') toast.info(`${label} dibatalkan`, { description: 'Daftar runtime diperbarui dari hasil pemeriksaan server.' });
            }
            if (job.status === 'succeeded') {
              tracked.current.set(job.id, job);
              setJobs([...tracked.current.values()]);
            }
            const refreshed = await onFinished(job);
            if (!active) return;
            // Keep a successful installation blocked until inventory confirms
            // the result; a slow/failed refresh must not expose Install again.
            if (job.status === 'succeeded' && !refreshed) current.set(job.id, job);
          }
        }
        for (const [id, job] of tracked.current) {
          if (!previous.has(id)) current.set(id, job);
        }
        tracked.current = current;
        setJobs([...current.values()]);
        setError('');
      } catch (cause: unknown) {
        if (active) setError(cause instanceof Error ? cause.message : 'Gagal memuat tugas runtime.');
      } finally {
        checking = false;
        if (active) setLoading(false);
      }
    };
    void check();
    const timer = setInterval(() => { void check(); }, 3000);
    return () => { active = false; clearInterval(timer); };
  }, [onFinished, names]);

  return { jobs, loading, error, track };
}
