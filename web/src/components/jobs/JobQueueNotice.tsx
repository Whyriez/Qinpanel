import { useEffect, useState } from 'react';
import { fetchJobs } from '../../api/client';
import type { JobItem } from '../../types/api';

/** Read-only queue context. Installing packages remains serialized by the worker. */
export function JobQueueNotice({ jobId }: { jobId: string }) {
  const [running, setRunning] = useState<JobItem[]>([]);
  const [error, setError] = useState(false);

  useEffect(() => {
    let active = true;
    let checking = false;
    const check = async () => {
      if (checking) return;
      checking = true;
      try {
        const result = await fetchJobs({ status: 'running', page: 1, per_page: 100 });
        if (active) {
          setRunning(result.items.filter(job => job.status === 'running' && job.id !== jobId));
          setError(false);
        }
      } catch {
        if (active) { setRunning([]); setError(true); }
      } finally { checking = false; }
    };
    void check();
    const timer = setInterval(() => { void check(); }, 3000);
    return () => { active = false; clearInterval(timer); };
  }, [jobId]);

  return (
    <div role="status" aria-label="Status antrean tugas" className="space-y-2 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] p-3 text-xs">
      <p className="font-medium text-[var(--ink)]">Tugas ini masih antre. Eksekusi belum dimulai.</p>
      <p className="text-[var(--ink-muted)]">Worker memproses satu tugas pada satu waktu. Tugas ini menunggu giliran.</p>
      {running.length > 0 && (
        <div className="space-y-1 text-[var(--ink-muted)]">
          <p>Menunggu tugas yang sedang berjalan:</p>
          {running.map(job => <p key={job.id} className="[overflow-wrap:anywhere]"><strong>{job.name}</strong> — {job.progress}% <span className="font-mono">({job.id})</span></p>)}
        </div>
      )}
      {error && <p className="text-[var(--ink-muted)]">Status tugas lain belum dapat dimuat. Pemantauan akan dicoba kembali.</p>}
    </div>
  );
}
