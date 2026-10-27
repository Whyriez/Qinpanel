import { useCallback, useEffect, useRef, useState } from 'react';
import { Download, Loader2, RefreshCw, Trash2 } from 'lucide-react';
import { fetchPythonCatalog, installPythonVersion, refreshRuntimeCatalog, removePythonVersion } from '../../api/client';
import type { JobItem, PythonRuntimeListApiResponse } from '../../types/api';
import { runtimeJobVersion, useRuntimeJobs } from './useRuntimeJobs';
import { RuntimeJobProgress } from './RuntimeJobProgress';
import { ConfirmDangerDialog } from '../ui/ConfirmDangerDialog';
import { toast } from 'sonner';
import { PythonManualInstall } from './PythonManualInstall';

const names = new Set(['python.install']);
const secondaryButton = 'inline-flex min-h-10 items-center justify-center gap-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] px-3 py-2 text-xs font-medium text-[var(--ink)] hover:bg-[var(--bg)] disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--primary)]';
const primaryButton = 'inline-flex min-h-10 items-center justify-center gap-2 rounded-[var(--radius-md)] bg-[var(--primary)] px-3 py-2 text-xs font-semibold text-white hover:opacity-90 disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--primary)]';
export function PythonRuntimesTab({ onCatalogRefresh }: { onCatalogRefresh?: () => void }) {
  const [data, setData] = useState<PythonRuntimeListApiResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [remove, setRemove] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const refreshingRef = useRef(false);
  const submitting = useRef(false);
  const load = useCallback(async () => {
    setLoading(true); setError('');
    try { const result = await fetchPythonCatalog(); setData(result); return result; }
    catch (cause) { setError(cause instanceof Error ? cause.message : 'Gagal memuat Python.'); return null; }
    finally { setLoading(false); }
  }, []);
  const finished = useCallback(async (job: JobItem) => {
    const result = await load();
    return Boolean(result && (job.status !== 'succeeded' || result.versions.some(v => v.version === runtimeJobVersion(job) && v.is_installed)));
  }, [load]);
  const jobs = useRuntimeJobs(finished, names);
  useEffect(() => { void load(); }, [load]);
  const refreshVersions = async () => {
    if (refreshingRef.current) return;
    refreshingRef.current = true; setRefreshing(true);
    try {
      const result = await refreshRuntimeCatalog();
      onCatalogRefresh?.();
      const inventory = await load();
      const source = result.sources.find(item => item.runtime === 'python');
      if (source?.error || source?.stale) throw new Error(source.error || 'Katalog Python belum dapat diperbarui.');
      if (!inventory) throw new Error('Inventaris Python belum dapat dimuat. Coba muat ulang Python.');
      if (inventory.catalog_ready === false) throw new Error('Daftar versi Python belum tersedia. Anda dapat memasukkan versi lengkap.');
    } finally { refreshingRef.current = false; setRefreshing(false); }
  };
  const install = async (version: string) => {
    if (submitting.current || jobs.loading || jobs.error || jobs.jobs.some(job => runtimeJobVersion(job) === version) || data?.versions.some(item => item.version === version && item.is_installed)) throw new Error('Pemasangan belum dapat dimulai. Periksa versi dan status tugas Python.');
    submitting.current = true; setBusy(version);
    try {
      const job = await installPythonVersion({ version }); jobs.track(job);
      toast.success(`Pemasangan Python ${version} masuk antrean`, { description: 'Progres tetap tersedia di halaman Tugas.' });
    } finally { submitting.current = false; setBusy(null); }
  };
  const confirmRemove = async () => {
    if (!remove || submitting.current) return;
    const version = remove; setRemove(null); setBusy(version); submitting.current = true;
    try { await removePythonVersion(version); await load(); toast.success(`Python ${version} dihapus`); }
    catch (cause) { toast.error(cause instanceof Error ? cause.message : 'Gagal menghapus Python.'); }
    finally { submitting.current = false; setBusy(null); }
  };
  const versions = data?.versions.filter(v => v.is_installed || data.catalog_ready !== false) ?? [];
  return <section className="space-y-4" aria-label="Runtime Python">
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div><h2 className="text-sm font-semibold">Versi Python</h2><p className="text-xs text-[var(--ink-muted)]">Pasang CPython untuk dipilih situs Django, Flask, FastAPI, atau aplikasi sendiri.</p></div>
      <div className="flex flex-wrap gap-2">
        <button type="button" onClick={() => { void refreshVersions().catch(cause => toast.error(cause instanceof Error ? cause.message : 'Gagal memperbarui katalog Python.')); }} disabled={loading || refreshing || Boolean(busy)} className={secondaryButton}><RefreshCw size={14} className={refreshing ? 'animate-spin' : ''} />{refreshing ? 'Memperbarui katalog…' : 'Perbarui katalog Python'}</button>
        <button type="button" onClick={() => void load()} disabled={loading || refreshing} className={secondaryButton}><RefreshCw size={14} />Muat ulang Python</button>
      </div>
    </div>
    {(error || jobs.error) && <p role="alert" className="text-sm text-[var(--danger)]">{error || jobs.error}</p>}
    {data?.catalog_ready === false && <p role="status" className="text-sm text-[var(--ink-muted)]">Daftar otomatis belum tersedia atau sudah kedaluwarsa. Perbarui katalog atau gunakan pemasangan versi lengkap di bawah.</p>}
    {loading && !data && <p role="status">Memuat runtime Python…</p>}
    {!loading && data && versions.length === 0 && <div className="space-y-2 border-y border-[var(--line)] py-5"><h3 className="text-sm font-semibold">Daftar versi Python belum tersedia</h3><p className="text-sm text-[var(--ink-muted)]">Perbarui katalog untuk menampilkan versi yang dapat dipasang. Runtime yang selesai dipasang dapat dipilih pada pengaturan situs.</p></div>}
    <div className="divide-y divide-[var(--line)] rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)]">
      {versions.map(version => {
        const job = jobs.jobs.find(j => runtimeJobVersion(j) === version.version);
        const disabled = Boolean(busy || job || loading || jobs.loading || jobs.error || error);
        return <article key={version.version} className="p-4 space-y-3">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div><h3 className="text-sm font-semibold">Python {version.version}</h3><p className="text-xs text-[var(--ink-muted)]">{version.is_installed ? 'Terpasang' : 'Tersedia untuk dipasang'} · {version.description}</p>{version.install_path && <p className="mt-1 break-all text-xs font-mono text-[var(--ink-muted)]">{version.install_path}</p>}</div>
            {version.is_installed ? <button type="button" disabled={disabled} onClick={() => setRemove(version.version)} className={secondaryButton}><Trash2 size={14} />Hapus Python {version.version}</button>
              : <button type="button" disabled={disabled || refreshing} onClick={() => { void install(version.version).catch(cause => toast.error(cause instanceof Error ? cause.message : 'Gagal membuat tugas Python.')); }} className={primaryButton}>{busy === version.version || job ? <Loader2 size={14} className="animate-spin" /> : <Download size={14} />}{job ? 'Sedang dipasang' : `Pasang Python ${version.version}`}</button>}
          </div>
          {job && <RuntimeJobProgress job={job} runtime={`Python ${version.version}`} />}
        </article>;
      })}
    </div>
    {jobs.jobs.filter(job => !versions.some(v => v.version === runtimeJobVersion(job))).map(job => <div key={job.id} className="rounded border border-[var(--line)] p-4"><p className="mb-2 text-sm">Pemasangan Python {runtimeJobVersion(job)}</p><RuntimeJobProgress job={job} runtime={`Python ${runtimeJobVersion(job) ?? ''}`} /></div>)}
    <PythonManualInstall onInstall={install} installedVersions={data?.versions.filter(version => version.is_installed).map(version => version.version) ?? []}
      busyVersions={jobs.jobs.map(runtimeJobVersion).filter((version): version is string => Boolean(version))}
      unavailableReason={jobs.error || error || (jobs.loading ? 'Memuat status tugas Python…' : loading ? 'Memeriksa Python terpasang…' : refreshing ? 'Memperbarui katalog Python…' : busy ? 'Tunggu permintaan Python yang sedang diproses.' : '')} />
    <ConfirmDangerDialog isOpen={Boolean(remove)} title={`Hapus Python ${remove ?? ''}?`} description="Runtime yang masih dipakai situs tidak dapat dihapus. File proyek dan virtual environment situs tetap tersimpan." confirmMatchText={remove ?? ''} confirmButtonLabel="Hapus runtime" onConfirm={() => void confirmRemove()} onClose={() => setRemove(null)} />
  </section>;
}
