import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { RefreshCw } from 'lucide-react';
import { applyUpdate, canUpdate, checkUpdate, isUpdating, updateLogs, updateStatus, type UpdateRequest, type UpdateStatus } from '../../api/panelUpdate';
import { ModalOverlay } from '../ui/ModalOverlay';

const pendingKey = 'qpanel.update.pending';
function readPending(): UpdateRequest | null {
  try { const value = JSON.parse(sessionStorage.getItem(pendingKey) || 'null'); return value && typeof value.id === 'string' && typeof value.version === 'string' && typeof value.digest === 'string' ? value : null; } catch { return null; }
}
function persistPending(value: UpdateRequest | null) { try { if (value) sessionStorage.setItem(pendingKey, JSON.stringify(value)); else sessionStorage.removeItem(pendingKey); } catch { /* Server status remains authoritative when browser storage is unavailable. */ } }
export function PanelUpdateSummary({ data, reconnecting = false }: { data: UpdateStatus; reconnecting?: boolean }) {
  const { i18n } = useTranslation(); const en = i18n.language.startsWith('en');
  const phase = data.update?.phase;
  const phases = { queued: en ? 'Update queued' : 'Update dijadwalkan', running: en ? 'Applying update' : 'Menerapkan pembaruan', succeeded: en ? 'Update completed' : 'Pembaruan selesai', failed: en ? 'Update failed' : 'Pembaruan gagal', interrupted: en ? 'Update interrupted' : 'Pembaruan terputus' };
  return <div className="space-y-4">
    <dl className="grid grid-cols-1 gap-4 sm:grid-cols-2"><div><dt className="text-sm text-[var(--ink-muted)]">{en ? 'Installed version' : 'Versi terpasang'}</dt><dd className="mt-1 font-semibold">{data.current_version}</dd></div><div><dt className="text-sm text-[var(--ink-muted)]">{en ? 'Latest stable release' : 'Rilis stabil terbaru'}</dt><dd className="mt-1 font-semibold">{data.available?.version || (en ? 'Not checked' : 'Belum diperiksa')}</dd></div></dl>
    {!data.supported && <p role="status" className="text-sm text-[var(--ink-muted)]">{en ? 'Install the complete panel package once to enable updates from this page.' : 'Pasang paket panel lengkap satu kali untuk mengaktifkan pembaruan dari halaman ini.'}</p>}
    {data.available?.digest === data.installed_digest && <p className="text-sm">{en ? 'This release package is already installed.' : 'Paket rilis ini sudah terpasang.'}</p>}
    {data.available?.version === data.current_version && data.available.digest !== data.installed_digest && data.current_version === 'v0.1.0' && <p className="text-sm text-[var(--ink-muted)]">{en ? 'This development release uses the same version number. Apply it to get the latest package.' : 'Rilis development ini memakai nomor versi yang sama. Terapkan untuk mengambil paket terbaru.'}</p>}
    {data.check_error && <p role="alert" className="text-sm text-[var(--danger)]">{data.check_error}</p>}
    {phase && <div role="status" aria-live="polite" className="border-t border-[var(--line)] pt-4 text-sm space-y-2">
      <p className="font-medium">{phases[phase]} · {data.update?.version}</p>
      {isUpdating(data.update) && <p className="text-[var(--ink-muted)]">{en ? 'The panel may reconnect while services restart. You can leave this page and return to check progress.' : 'Panel dapat tersambung ulang saat layanan dimulai kembali. Anda dapat meninggalkan halaman ini lalu kembali untuk memantau progres.'}</p>}
      {(phase === 'failed' || phase === 'interrupted') && <p className="text-[var(--ink-muted)]">{en ? 'The update did not complete. The installer attempts to restore its snapshot on failure. Check diagnostics before trying again.' : 'Update belum selesai. Installer mencoba memulihkan snapshot jika pemasangan gagal. Periksa diagnostik sebelum mencoba kembali.'}</p>}
    </div>}
    {reconnecting && <p role="status" className="text-sm text-[var(--ink-muted)]">{en ? 'Waiting for confirmation from the server. Reconnecting to the panel…' : 'Menunggu konfirmasi server. Menyambungkan kembali ke panel…'}</p>}
    {data.available?.notes && <details className="border-t border-[var(--line)] pt-3 text-sm"><summary className="cursor-pointer font-medium">{en ? 'Release notes' : 'Catatan rilis'}</summary><p className="mt-3 whitespace-pre-wrap break-words text-[var(--ink-muted)]">{data.available.notes}</p></details>}
  </div>;
}
export function PanelUpdateCard() {
  const { i18n } = useTranslation(); const en = i18n.language.startsWith('en');
  const [data, setData] = useState<UpdateStatus | null>(null), [error, setError] = useState(''), [loading, setLoading] = useState(true), [working, setWorking] = useState(false), [confirm, setConfirm] = useState(false), [reconnecting, setReconnecting] = useState(false);
  const [pending, setPending] = useState<UpdateRequest | null>(readPending);
  const [diagnostics, setDiagnostics] = useState<string | null>(null);
  const current = useRef(data), pendingRef = useRef(pending), mounted = useRef(true), inFlight = useRef(false);
  const epoch = useRef(0), action = useRef(false);
  current.current = data; pendingRef.current = pending;
  const accept = useCallback((value: UpdateStatus) => {
    if (!mounted.current) return;
    setData(value); setError(''); setReconnecting(false);
    if (value.update?.id === pendingRef.current?.id) { setPending(null); persistPending(null); }
  }, []);
  const refresh = useCallback(async (signal?: AbortSignal) => {
    if (inFlight.current || action.current) return; inFlight.current = true;
    const revision = epoch.current;
    try { const result = await updateStatus(signal); if (revision === epoch.current) accept(result); }
    catch (e) { if (mounted.current && !signal?.aborted && revision === epoch.current) { if (isUpdating(current.current?.update) || pendingRef.current) setReconnecting(true); else setError(e instanceof Error ? e.message : 'Gagal memuat status update.'); } }
    finally { inFlight.current = false; if (mounted.current) setLoading(false); }
  }, [accept]);
  useEffect(() => {
    mounted.current = true; const controller = new AbortController(); void refresh(controller.signal);
    const timer = window.setInterval(() => { if (!document.hidden) void refresh(controller.signal); }, 5000);
    const focus = () => { void refresh(controller.signal); }; window.addEventListener('focus', focus);
    return () => { mounted.current = false; controller.abort(); window.clearInterval(timer); window.removeEventListener('focus', focus); };
  }, [refresh]);
  const check = async () => { epoch.current++; action.current = true; setWorking(true); setError(''); try { accept(await checkUpdate()); } catch (e) { if (mounted.current) setError(e instanceof Error ? e.message : 'Gagal memeriksa update.'); } finally { action.current = false; if (mounted.current) setWorking(false); } };
  const logs = async () => { try { const result = await updateLogs(); if (mounted.current) setDiagnostics(result.diagnostics || (en ? 'No log entries yet.' : 'Belum ada log.')); } catch (e) { if (mounted.current) setError(e instanceof Error ? e.message : 'Gagal memuat diagnostik.'); } };
  const apply = async (retry?: UpdateRequest) => {
    if (!data?.available && !retry) return;
    epoch.current++; action.current = true;
    const request = retry || { id: crypto.randomUUID(), version: data!.available!.version, digest: data!.available!.digest };
    persistPending(request); pendingRef.current = request; setPending(request); setWorking(true); setConfirm(false); setError('');
    try { accept(await applyUpdate(request)); }
    catch (e) {
      if (!mounted.current) return;
      const status = (e as { status?: number }).status;
      if (status && status >= 400 && status < 500) { setPending(null); persistPending(null); setError(e instanceof Error ? e.message : 'Update ditolak.'); }
      else setReconnecting(true);
    } finally { action.current = false; if (mounted.current) setWorking(false); }
  };
  const busy = working || isUpdating(data?.update) || !!pending;
  return <section aria-labelledby="panel-update-title" className="border border-[var(--line)] bg-[var(--surface)] rounded-[var(--radius-md)] p-5 sm:p-6 space-y-5">
    <div><h2 id="panel-update-title" className="text-base font-semibold">{en ? 'Panel updates' : 'Pembaruan panel'}</h2><p className="mt-2 text-sm text-[var(--ink-muted)]">{en ? 'Apply new releases without reinstalling. Accounts, websites, configuration and databases are retained.' : 'Terapkan rilis baru tanpa instalasi ulang. Akun, website, konfigurasi, dan database tetap dipertahankan.'}</p></div>
    {loading && !data && <p role="status" className="text-sm">{en ? 'Loading update status…' : 'Memuat status pembaruan…'}</p>}
    {error && <p role="alert" className="text-sm text-[var(--danger)]">{error}</p>}
    {data && <PanelUpdateSummary data={data} reconnecting={reconnecting || !!pending} />}
    {!data && reconnecting && <p role="status">{en ? 'Reconnecting to the panel…' : 'Menyambungkan kembali ke panel…'}</p>}
    <div className="flex flex-wrap gap-3">
      <button type="button" className="button-secondary" disabled={busy || loading} onClick={() => void check()}><RefreshCw size={16} aria-hidden="true" />{en ? 'Check for updates' : 'Periksa pembaruan'}</button>
      <button type="button" className="button-primary" disabled={!data || !canUpdate(data) || busy || !!error} onClick={() => setConfirm(true)}>{en ? 'Apply update' : 'Terapkan pembaruan'}</button>
      {(error || reconnecting || pending) && <button type="button" className="button-secondary" onClick={() => void refresh()}>{en ? 'Refresh status' : 'Muat ulang status'}</button>}
      {pending && data && !isUpdating(data.update) && <button type="button" disabled={working} className="button-secondary" onClick={() => void apply(pending)}>{en ? 'Retry this request' : 'Kirim ulang permintaan ini'}</button>}
      {data?.update?.phase === 'succeeded' && <button type="button" className="button-secondary" onClick={() => window.location.reload()}>{en ? 'Reload panel' : 'Muat ulang panel'}</button>}
      {data?.update && <button type="button" className="button-secondary" onClick={() => void logs()}>{en ? 'View diagnostics' : 'Lihat diagnostik'}</button>}
    </div>
    {diagnostics !== null && <pre tabIndex={0} className="max-h-80 overflow-auto whitespace-pre-wrap break-words border border-[var(--line)] rounded-[var(--radius-md)] p-4 text-xs focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--primary)]" aria-label={en ? 'Update diagnostics' : 'Diagnostik pembaruan'}>{diagnostics}</pre>}
    {confirm && data?.available && <ModalOverlay onDismiss={() => setConfirm(false)}><div className="dialog-panel max-w-lg"><div className="dialog-header"><h2>{en ? 'Apply panel update' : 'Terapkan pembaruan panel'}</h2></div><div className="dialog-body space-y-3 text-sm"><p>{data.current_version} → {data.available.version}</p><p>{en ? 'A snapshot is created before migration. The panel will briefly reconnect during the update. Your websites and data are retained.' : 'Snapshot dibuat sebelum migrasi. Panel akan tersambung ulang saat update. Website dan data Anda tetap dipertahankan.'}</p></div><div className="dialog-footer flex flex-wrap gap-3"><button type="button" className="button-secondary" onClick={() => setConfirm(false)}>{en ? 'Cancel' : 'Batal'}</button><button type="button" className="button-primary" onClick={() => void apply()}>{en ? 'Apply this version' : 'Terapkan versi ini'}</button></div></div></ModalOverlay>}
  </section>;
}
