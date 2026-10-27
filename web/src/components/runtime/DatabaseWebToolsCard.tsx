import { useCallback, useEffect, useRef, useState } from 'react';
import { Download, RefreshCw } from 'lucide-react';
import { fetchDatabaseWebTools, installDatabaseWebTools } from '../../api/client';
import { useRuntimeJobs } from './useRuntimeJobs';
import { RuntimeJobProgress } from './RuntimeJobProgress';
import { toast } from 'sonner';

const names = new Set(['database.web.install']);
const button = 'inline-flex min-h-10 items-center justify-center gap-2 rounded-[var(--radius-md)] border border-[var(--line)] px-3 py-2 text-xs font-medium hover:bg-[var(--bg)] disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--primary)]';
export function DatabaseWebToolsCard() {
  const [status, setStatus] = useState<{ installed: boolean; ready: boolean } | null>(null);
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState('');
  const inFlight = useRef(false);
  const load = useCallback(async () => {
    setLoading(true);
    try { const data = await fetchDatabaseWebTools(); setStatus(data); setError(''); return data; }
    catch (cause) { setError(cause instanceof Error ? cause.message : 'Gagal memuat pengelola database.'); return null; }
    finally { setLoading(false); }
  }, []);
  const finished = useCallback(async () => Boolean((await load())?.ready), [load]);
  const jobs = useRuntimeJobs(finished, names);
  useEffect(() => { void load(); }, [load]);
  const install = async () => {
    if (inFlight.current || jobs.loading || jobs.error || !status || error) return;
    inFlight.current = true; setSubmitting(true);
    try { const job = await installDatabaseWebTools(); jobs.track(job); }
    catch (cause) { toast.error(cause instanceof Error ? cause.message : 'Gagal memasang pengelola database.'); }
    finally { inFlight.current = false; setSubmitting(false); }
  };
  return <section aria-label="Pengelola database internal" className="space-y-3 rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-4 text-[var(--ink)]">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div className="space-y-1"><h3 className="text-sm font-semibold">Pengelola database internal</h3>
        <p className="max-w-2xl text-xs text-[var(--ink-muted)]">phpMyAdmin untuk MariaDB/MySQL dan Adminer untuk PostgreSQL. Tenant membuka dari tab database situs, langsung di tab baru tanpa mengaktifkan akses remote.</p>
        <p role="status" className="text-xs">{loading ? 'Memeriksa pengelola…' : status?.ready ? 'Siap digunakan' : status?.installed ? 'Terpasang, layanan belum siap' : status ? 'Belum dipasang' : 'Status belum tersedia'}</p>
      </div>
      <div className="flex flex-wrap gap-2">
        <button type="button" onClick={() => void load()} disabled={loading} className={button}><RefreshCw size={14} />Periksa pengelola</button>
        <button type="button" onClick={() => void install()} disabled={loading || submitting || jobs.loading || Boolean(jobs.error || error) || !status || jobs.jobs.length > 0} className={button}><Download size={14} />{jobs.jobs.length || submitting ? 'Sedang dipasang' : status?.ready ? 'Perbarui pengelola' : status?.installed ? 'Perbaiki pengelola' : 'Pasang pengelola database'}</button>
      </div>
    </div>
    {(error || jobs.error) && <p role="alert" className="text-xs text-[var(--danger)]">{error || jobs.error}</p>}
    {jobs.jobs.map(job => <RuntimeJobProgress key={job.id} job={job} runtime="Pengelola database" />)}
  </section>;
}
