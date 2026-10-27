import type { JobItem } from '../../types/api';
import { useRef, useState } from 'react';
import { cancelJob } from '../../api/client';
import { canCancelJob } from '../../lib/jobProgress';

export function RuntimeJobProgress({ job, runtime }: { job: JobItem; runtime: string }) {
  const [pending, setPending] = useState(false);
  const [requested, setRequested] = useState(false);
  const [error, setError] = useState('');
  const submitting = useRef(false);
  const isRuntimeInstall = ['php.install', 'php.repo.setup', 'node.install', 'python.install'].includes(job.name);
  const cancellable = isRuntimeInstall && canCancelJob(job);
  const cancel = async () => {
    if (!cancellable || submitting.current || requested) return;
    submitting.current = true; setPending(true); setError('');
    try {
      const updated = await cancelJob(job.id);
      if (updated.status === 'cancelled') setRequested(true);
      else setError('Status tugas sudah berubah. Muat ulang progres untuk melihat hasilnya.');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'Gagal membatalkan pemasangan. Coba lagi.');
    } finally { submitting.current = false; setPending(false); }
  };
  return (
    <div className="space-y-2">
      {job.status === 'queued' && <p className="text-xs text-[var(--ink-muted)]">Menunggu giliran worker. Tugas belum dimulai.</p>}
      <div className="flex flex-wrap items-center gap-3">
      <progress
        max={100}
        value={job.progress}
        aria-label={`Progres tugas ${runtime}`}
        className="block h-1.5 w-40 max-w-full accent-[var(--primary)]"
      >
        {job.progress}%
      </progress>
      <a href={`/jobs?job=${encodeURIComponent(job.id)}`} className="text-xs text-[var(--primary)] underline underline-offset-2">
        Lihat progres
      </a>
      {cancellable && <button type="button" onClick={() => { void cancel(); }} disabled={pending || requested}
        aria-label={`Batalkan pemasangan ${runtime}`}
        className="min-h-9 rounded-[var(--radius-md)] border border-[var(--line)] px-3 py-1 text-xs text-[var(--danger)] disabled:opacity-50">
        {pending ? 'Membatalkan…' : requested ? 'Pembatalan diminta' : 'Batalkan'}
      </button>}
      </div>
      {requested && <p role="status" className="text-xs text-[var(--ink-muted)]">Permintaan pembatalan diterima. Memeriksa kembali hasil pemasangan.</p>}
      {isRuntimeInstall && job.status === 'running' && !cancellable && <p className="text-xs text-[var(--ink-muted)]">Pemasangan paket sedang berjalan dan tidak dapat dihentikan dengan aman. Pembatalan tersedia saat masih antre.</p>}
      {error && <p role="alert" className="text-xs text-[var(--danger)]">{error}</p>}
    </div>
  );
}
