import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { fetchJobDetail, request } from '../../api/client';
import type { Endpoint } from '../../api/connectivity';
import { endpointAuthority } from '../../api/connectivity';
import { useRuntimeJobs } from '../runtime/useRuntimeJobs';
import { RuntimeJobProgress } from '../runtime/RuntimeJobProgress';
import { ModalOverlay } from '../ui/ModalOverlay';
import { ConfirmDangerDialog } from '../ui/ConfirmDangerDialog';

export interface PanelCertificate { id: string; scope: 'panel'; ip: string; port: number; staging: boolean; state: 'queued' | 'issuing' | 'candidate' | 'staging' | 'active' | 'retired' | 'failed'; not_before: number | null; not_after: number | null; renew_at: number | null; expired: boolean; error: string | null; job_id: string | null }
interface Inventory { certificates: PanelCertificate[]; profile: string; challenge_port: number }
const jobNames = new Set(['panel.tls.issue']);
const input = 'w-full min-w-0 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] px-3 py-2 text-sm text-[var(--ink)] mt-1';
export function panelTlsCandidates(endpoints: Endpoint[]) {
  return endpoints.filter(e => e.service === 'panel' && e.scope.kind === 'host' && e.enabled && e.transport === 'direct' && e.tls_mode === 'tls' && e.port >= 1024 && ![7700, 17890].includes(e.port) && (/^\d+\.\d+\.\d+\.\d+$/.test(e.host) || e.host.includes(':')));
}
export function certificateHours(end: number, clock: number) { return Math.max(0, Math.ceil((end - clock) / 3600)); }
export function parsePanelTlsInventory(value: unknown): Inventory {
  if (!value || typeof value !== 'object' || !('certificates' in value) || !Array.isArray(value.certificates) || value.certificates.length > 20) throw new Error('Inventaris sertifikat panel tidak valid.');
  for (const row of value.certificates) {
    if (!row || row.scope !== 'panel' || typeof row.id !== 'string' || typeof row.ip !== 'string' || !Number.isInteger(row.port) || typeof row.staging !== 'boolean' || !['queued', 'issuing', 'candidate', 'staging', 'active', 'retired', 'failed'].includes(row.state) || [row.not_before, row.not_after, row.renew_at].some(n => n !== null && (!Number.isSafeInteger(n) || n < 0 || n > 253402300799)) || (row.error !== null && typeof row.error !== 'string') || (row.job_id !== null && typeof row.job_id !== 'string')) throw new Error('Inventaris sertifikat panel tidak valid.');
  }
  return value as Inventory;
}
export function PanelTlsSettings({ endpoints, disabled = false }: { endpoints: Endpoint[]; disabled?: boolean }) {
  const { i18n } = useTranslation(), fields = useId();
  const en = i18n.language.startsWith('en'), text = (id: string, english: string) => en ? english : id;
  const [inventory, setInventory] = useState<Inventory | null>(null), [error, setError] = useState(''), [message, setMessage] = useState('');
  const [loading, setLoading] = useState(true), [busy, setBusy] = useState(false), [clock, setClock] = useState(() => Date.now() / 1000);
  const [editor, setEditor] = useState(false), [selected, setSelected] = useState(''), [email, setEmail] = useState(''), [staging, setStaging] = useState(true), [confirmed, setConfirmed] = useState(false);
  const [discarding, setDiscarding] = useState<PanelCertificate | null>(null);
  const mounted = useRef(false), sequence = useRef(0), pending = useRef(false);
  const load = useCallback(async () => {
    const seq = ++sequence.current;
    try {
      const data = parsePanelTlsInventory(await request<unknown>('/api/v1/connectivity/panel-tls'));
      if (mounted.current && seq === sequence.current) { setInventory(data); setClock(Date.now() / 1000); setError(''); }
      return true;
    } catch (cause) { if (mounted.current && seq === sequence.current) { setInventory(null); setError(cause instanceof Error ? cause.message : 'Inventaris belum tersedia.'); } return false; }
    finally { if (mounted.current && seq === sequence.current) setLoading(false); }
  }, []);
  const jobs = useRuntimeJobs(load, jobNames);
  useEffect(() => { mounted.current = true; void load(); const tick = () => setClock(Date.now() / 1000); const timer = setInterval(tick, 60000); window.addEventListener('focus', tick); return () => { mounted.current = false; ++sequence.current; clearInterval(timer); window.removeEventListener('focus', tick); }; }, [load]);
  const candidates = panelTlsCandidates(endpoints), current = inventory?.certificates.find(c => c.state === 'active');
  const working = inventory?.certificates.some(c => ['queued', 'issuing', 'candidate'].includes(c.state)) || jobs.jobs.some(j => ['queued', 'running'].includes(j.status));
  const locked = disabled || busy || loading || jobs.loading || !inventory;
  const submit = async (path: string, body?: unknown) => {
    if (pending.current) return;
    pending.current = true; setBusy(true); setError(''); setMessage('');
    try {
      const result = await request<{ job_id: string }>(path, { method: 'POST', ...(body ? { body: JSON.stringify(body) } : {}) });
      if (mounted.current) { setEditor(false); setDiscarding(null); setMessage(text('Permintaan disimpan. Proses tetap berjalan saat Anda berpindah halaman.', 'Request saved. Processing continues when you leave this page.')); }
      if (result.job_id) {
        try { const job = await fetchJobDetail(result.job_id); if (mounted.current) jobs.track(job); } catch { /* inventory and the Tasks page retain the persisted job */ }
      }
      await load();
    } catch (cause) { if (mounted.current) setError(cause instanceof Error ? cause.message : 'Permintaan sertifikat gagal.'); }
    finally { pending.current = false; if (mounted.current) setBusy(false); }
  };
  const date = (value: number) => new Date(value * 1000).toLocaleString(en ? 'en-GB' : 'id-ID');
  return <section className="space-y-3 border-b border-[var(--line)] py-5" aria-label={text('Sertifikat IP panel', 'Panel IP certificates')}>
    <div className="flex flex-wrap items-center justify-between gap-2"><h3 className="text-sm font-semibold">{text('HTTPS panel dengan IP', 'Panel HTTPS with an IP address')}</h3><button type="button" className="button-secondary" disabled={busy} onClick={() => { setLoading(true); void load(); }}>{text('Muat ulang sertifikat panel', 'Reload panel certificates')}</button></div>
    <p className="text-sm text-[var(--ink-muted)]">{text('Sertifikat khusus panel, terpisah dari SSL situs. Penerbitan memerlukan IP publik milik server dan akses masuk port 80 untuk HTTP-01, meskipun HTTPS memakai port lain.', 'Panel certificates are separate from website SSL. Issuance requires a public IP belonging to this server and inbound port 80 for HTTP-01, even when HTTPS uses another port.')}</p>
    <p className="text-xs text-[var(--ink-muted)]">{text('Masa berlaku sekitar 160 jam. Renewal otomatis mengikuti ARI atau masa berlaku aktual. Proses ini tidak membuka firewall, mengubah DNS, atau mengaktifkan URL utama. HTTP panel yang ada tetap tersedia.', 'Certificates last about 160 hours. Automatic renewal uses ARI or the actual lifetime. This does not open firewall ports, change DNS, or activate the primary URL. Existing panel HTTP remains available.')}</p>
    {loading && <p role="status" className="text-sm">{text('Memuat sertifikat panel…', 'Loading panel certificates…')}</p>}
    {inventory && <>
      {!inventory.certificates.length && <p className="text-sm">{text('Belum ada sertifikat IP panel.', 'No panel IP certificate yet.')}</p>}
      <div className="flex flex-wrap gap-2"><button type="button" className="button-secondary" disabled={locked || working || !candidates.length} onClick={() => { setSelected(candidates[0]?.id || ''); setConfirmed(false); setStaging(true); setError(''); setEditor(true); }}>{text('Terbitkan sertifikat IP', 'Issue an IP certificate')}</button>{current && <button type="button" className="button-secondary" disabled={locked || working} onClick={() => void submit('/api/v1/connectivity/panel-tls/renew')}>{text('Perbarui sertifikat panel', 'Renew panel certificate')}</button>}</div>
      {!candidates.length && <p className="text-xs">{text('Tambahkan alamat Panel langsung dengan TLS, IP publik, dan port terpisah (misalnya 19443). Gunakan filter Panel untuk melihatnya.', 'Add a direct Panel endpoint with TLS, a public IP, and a separate port (such as 19443). Use the Panel filter to see it.')}</p>}
      <div className="space-y-3">{inventory.certificates.slice(0, 5).map(c => <div key={c.id} className="border-t border-[var(--line)] pt-3 space-y-1 text-xs">
        <p className="font-medium break-all">{c.ip.includes(':') ? `[${c.ip}]:${c.port}` : `${c.ip}:${c.port}`} · {c.not_after !== null && c.not_after <= clock ? text('Kedaluwarsa', 'Expired') : c.state === 'active' ? text('Inventaris aktif', 'Active inventory') : c.state === 'staging' ? text('Uji staging selesai, tidak dipasang', 'Staging test complete, not installed') : c.state === 'candidate' ? text('Kandidat perlu sinkronisasi', 'Candidate needs synchronization') : c.state === 'retired' ? text('Sertifikat sebelumnya', 'Previous certificate') : c.state === 'failed' ? text('Gagal', 'Failed') : text('Dalam proses', 'Processing')}</p>
        {c.not_after !== null && <p>{text('Berlaku hingga', 'Valid until')} <time dateTime={new Date(c.not_after * 1000).toISOString()}>{date(c.not_after)}</time> · {certificateHours(c.not_after, clock)} {text('jam tersisa', 'hours remaining')}</p>}
        {c.renew_at !== null && c.state === 'active' && <p>{text('Jadwal renewal', 'Renewal scheduled')}: {date(c.renew_at)}</p>}
        {c.error && <p className="text-[var(--danger)] break-words">{c.error}</p>}
        {c.state === 'candidate' && <div className="flex flex-wrap gap-2"><button type="button" className="button-secondary" disabled={locked} onClick={() => void submit(`/api/v1/connectivity/panel-tls/${encodeURIComponent(c.id)}/retry`)}>{text('Sinkronkan kandidat', 'Synchronize candidate')}</button><button type="button" className="button-secondary" disabled={locked} onClick={() => setDiscarding(c)}>{text('Buang kandidat', 'Discard candidate')}</button></div>}
      </div>)}</div>
    </>}
    {jobs.jobs.slice(0, 2).map(job => <RuntimeJobProgress key={job.id} job={job} runtime="sertifikat panel" />)}
    {jobs.error && <p role="alert" className="text-xs">{text('Progres belum tersedia; lihat halaman Tugas.', 'Progress is unavailable; see the Tasks page.')} {jobs.error}</p>}
    <a href="/jobs" className="text-xs underline">{text('Buka riwayat tugas', 'Open task history')}</a>
    {message && <p role="status" className="text-sm">{message}</p>}
    {error && !editor && <p role="alert" className="text-sm text-[var(--danger)]">{error}</p>}
    {editor && <ModalOverlay aria-labelledby={`${fields}-title`} onDismiss={() => setEditor(false)} dismissDisabled={busy}><form className="dialog-panel max-w-xl border border-[var(--line)] p-5 gap-4" onSubmit={event => { event.preventDefault(); const e = candidates.find(e => e.id === selected); if (e && confirmed) void submit('/api/v1/connectivity/panel-tls/issue', { endpoint_id: e.id, expected_revision: e.revision, email, staging, http01_confirmed: confirmed }); }}>
      <h3 id={`${fields}-title`} className="text-sm font-semibold">{text('Terbitkan sertifikat IP panel', 'Issue a panel IP certificate')}</h3>
      <div className="dialog-body space-y-4 text-sm"><div><label htmlFor={`${fields}-endpoint`}>{text('Alamat panel', 'Panel endpoint')}</label><select id={`${fields}-endpoint`} className={input} value={selected} disabled={busy} onChange={event => setSelected(event.target.value)}>{candidates.map(e => <option key={e.id} value={e.id}>{endpointAuthority(e)}</option>)}</select></div>
        <div><label htmlFor={`${fields}-email`}>{text('Email akun CA', 'CA account email')}</label><input id={`${fields}-email`} type="email" required maxLength={254} className={input} autoComplete="email" value={email} disabled={busy} onChange={event => setEmail(event.target.value)} /></div>
        <label className="flex items-start gap-2"><input className="mt-0.5 size-4 shrink-0" type="checkbox" checked={staging} disabled={busy} onChange={event => setStaging(event.target.checked)} /><span>{text('Uji dahulu dengan CA staging. Sertifikat uji tidak dipasang pada panel.', 'Test with the staging CA first. Test certificates are not installed on the panel.')}</span></label>
        <p className="text-xs text-[var(--ink-muted)]">{text('Gunakan port TLS terpisah yang belum dipakai. Jika listener TLS sudah aktif, IP dan port harus tetap sama agar akses tidak terputus. DNS-01 tidak mendukung identifier IP.', 'Use a separate, unused TLS port. An existing TLS listener must keep its IP and port to preserve access. DNS-01 does not support IP identifiers.')}</p>
        <label className="flex items-start gap-2"><input className="mt-0.5 size-4 shrink-0" type="checkbox" required checked={confirmed} disabled={busy} onChange={event => setConfirmed(event.target.checked)} /><span>{text('IP ini milik server saya, port publik 80 dapat dijangkau CA, dan saya menyetujui ketentuan layanan Let’s Encrypt.', 'This IP belongs to my server, public port 80 is reachable by the CA, and I accept the Let’s Encrypt terms of service.')}</span></label>
        <a href="https://letsencrypt.org/repository/" target="_blank" rel="noopener noreferrer" className="text-xs underline">{text('Ketentuan layanan CA', 'CA terms of service')}</a>
        {!staging && <p className="text-xs">{text('Sertifikat produksi yang valid akan dipasang setelah pengujian kandidat, syntax check, dan reload Nginx. Jika gagal, sertifikat aktif sebelumnya dipertahankan.', 'A valid production certificate is installed after candidate validation, syntax checks, and an Nginx reload. Failure preserves the previous active certificate.')}</p>}
        {error && <p role="alert" className="text-[var(--danger)]">{error}</p>}
      </div><div className="dialog-footer flex justify-end gap-2"><button type="button" className="button-secondary" disabled={busy} onClick={() => setEditor(false)}>{text('Batal', 'Cancel')}</button><button type="submit" className="button-primary" disabled={busy || !confirmed || !selected || !email}>{busy ? text('Menyimpan…', 'Saving…') : text('Antrekan penerbitan', 'Queue issuance')}</button></div>
    </form></ModalOverlay>}
    <ConfirmDangerDialog isOpen={Boolean(discarding)} onClose={() => setDiscarding(null)} onConfirm={() => { if (discarding) void submit(`/api/v1/connectivity/panel-tls/${encodeURIComponent(discarding.id)}/discard`); }} isLoading={busy} title={text('Buang kandidat sertifikat', 'Discard candidate certificate')} description={text('Private key kandidat di inventaris akan dihapus hanya jika agent memastikan kandidat belum aktif. Sertifikat aktif tidak dihapus.', 'The candidate private key is removed from inventory only if the agent confirms the candidate is not active. The active certificate is preserved.')} confirmMatchText={discarding?.ip || ''} confirmButtonLabel={text('Buang kandidat', 'Discard candidate')} />
  </section>;
}
