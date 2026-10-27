import { useCallback, useEffect, useState } from 'react';
import { getRuntimeCatalog, refreshRuntimeCatalog, saveRuntimeCatalogSchedule, type RuntimeCatalogStatus } from '../../api/client';
import { toast } from 'sonner';
const buttonStyle = 'inline-flex min-h-10 items-center justify-center rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] px-3 py-2 text-xs font-medium text-[var(--ink)] hover:bg-[var(--bg)] disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--primary)]';

export function CatalogScheduleCard({ onRefresh }: { onRefresh: () => void }) {
  const [data, setData] = useState<RuntimeCatalogStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const load = useCallback(async () => {
    try { setData(await getRuntimeCatalog()); setError(''); }
    catch (cause) { setError(cause instanceof Error ? cause.message : 'Gagal memuat jadwal katalog.'); }
  }, []);
  useEffect(() => { void load(); }, [load]);
  const save = async (enabled: boolean, interval_hours: number) => {
    const previous = data;
    setData(current => current ? { ...current, enabled, interval_hours } : current);
    setBusy(true); setError('');
    try { setData(await saveRuntimeCatalogSchedule({ enabled, interval_hours })); toast.success('Jadwal katalog disimpan'); }
    catch (cause) { setData(previous); setError(cause instanceof Error ? cause.message : 'Gagal menyimpan jadwal.'); }
    finally { setBusy(false); }
  };
  const refresh = async () => {
    setBusy(true); setError('');
    try {
      const result = await refreshRuntimeCatalog(); setData(result); onRefresh();
      if (result.sources.some(s => s.error)) toast.error('Sebagian katalog belum dapat diperbarui');
      else toast.success('Katalog runtime diperbarui');
    } catch (cause) { setError(cause instanceof Error ? cause.message : 'Gagal memperbarui katalog.'); }
    finally { setBusy(false); }
  };
  return <section aria-label="Jadwal katalog runtime" className="space-y-3 rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-4">
    <div className="flex flex-wrap justify-between gap-3"><div><h2 className="text-sm font-semibold">Pembaruan katalog runtime</h2><p className="text-xs text-[var(--ink-muted)]">Memeriksa rilis resmi secara berkala. Instalasi dan versi situs tetap dipilih manual.</p></div>
      <button type="button" disabled={busy} onClick={() => void refresh()} className={buttonStyle}>{busy ? 'Sedang memproses…' : 'Perbarui katalog sekarang'}</button></div>
    {error && <p role="alert" className="text-sm text-[var(--danger)]">{error}</p>}
    {!data && error && <button type="button" onClick={() => void load()} className={buttonStyle}>Coba muat jadwal lagi</button>}
    {data && <>
      <div className="flex flex-wrap items-center gap-4 text-sm">
        <label className="inline-flex items-center gap-2"><input type="checkbox" checked={data.enabled} disabled={busy} onChange={e => void save(e.target.checked, data.interval_hours)} />Perbarui otomatis</label>
        <label className="inline-flex items-center gap-2">Periksa setiap<select aria-label="Interval pembaruan katalog" value={data.interval_hours} disabled={busy} onChange={e => void save(data.enabled, Number(e.target.value))} className="rounded border border-[var(--line)] bg-[var(--surface)] px-2 py-1"><option value={6}>6 jam</option><option value={12}>12 jam</option><option value={24}>24 jam</option><option value={168}>7 hari</option></select></label>
      </div>
      <ul className="grid gap-2 text-xs sm:grid-cols-3">{data.sources.map(source => <li key={source.runtime} className="break-words"><span className="font-semibold">{source.runtime === 'node' ? 'Node.js' : source.runtime === 'php' ? 'PHP' : 'Python'}</span><span className="block text-[var(--ink-muted)]">{source.refreshed_at ? `Terakhir diperbarui ${new Date(source.refreshed_at * 1000).toLocaleString()}` : 'Belum tersinkronisasi'}</span>{source.error ? <span role="status" className="text-[var(--danger)]">{source.error}</span> : source.stale && <span>Menunggu pembaruan katalog</span>}</li>)}</ul>
    </>}
  </section>;
}
