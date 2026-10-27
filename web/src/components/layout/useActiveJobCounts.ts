import { useEffect, useState } from 'react';
import { fetchJobs } from '../../api/client';

export interface ActiveJobCounts { queued: number; running: number; total: number }

export function useActiveJobCounts(userId: number | null): ActiveJobCounts | null {
  const [snapshot, setSnapshot] = useState<{ userId: number; counts: ActiveJobCounts } | null>(null);
  useEffect(() => {
    if (userId === null) return;
    let active = true;
    let inFlight = false;
    const load = async () => {
      if (!active || inFlight || document.hidden) return;
      inFlight = true;
      try {
        const [queued, running] = await Promise.all([
          fetchJobs({ status: 'queued', page: 1, per_page: 1 }),
          fetchJobs({ status: 'running', page: 1, per_page: 1 }),
        ]);
        const total = queued.total + running.total;
        if (![queued.total, running.total, total].every(value => Number.isSafeInteger(value) && value >= 0)) {
          throw new Error('Jumlah tugas tidak valid');
        }
        if (active) setSnapshot({ userId, counts: { queued: queued.total, running: running.total, total } });
      } catch {
        // Unknown counts must not be presented as zero or an old successful snapshot.
        if (active) setSnapshot(null);
      } finally { inFlight = false; }
    };
    void load();
    const refresh = () => { void load(); };
    const timer = window.setInterval(refresh, 5000);
    window.addEventListener('focus', refresh);
    document.addEventListener('visibilitychange', refresh);
    return () => {
      active = false;
      window.clearInterval(timer);
      window.removeEventListener('focus', refresh);
      document.removeEventListener('visibilitychange', refresh);
    };
  }, [userId]);
  return snapshot?.userId === userId ? snapshot.counts : null;
}
