import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { request } from '../../api/client';
import type { JobItem } from '../../types/api';
import { RuntimeJobProgress } from '../runtime/RuntimeJobProgress';
import { useRuntimeJobs } from '../runtime/useRuntimeJobs';
import { ModalOverlay } from '../ui/ModalOverlay';
import { ConfirmDangerDialog } from '../ui/ConfirmDangerDialog';

interface DnsOrder { id: string; site_id: number; state: 'queued' | 'issuing' | 'candidate' | 'active' | 'staging' | 'failed'; staging: boolean; error: string | null; job_id: string }
interface Inventory {
  provider: { configured: boolean; zone_id?: string; zone_name?: string; revision?: string; pending: { id: string; hostname: string; created_at: number }[] };
  projects: { site_id: number; hostname: string }[];
  orders: DnsOrder[];
  bindings: { site_id: number; certificate_id: number; renew_at: number; not_after: number; error: string | null }[];
}
const jobsFilter = new Set(['ssl.dns01.issue']);
const input = 'mt-1 w-full min-w-0 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] px-3 py-2 text-sm text-[var(--ink)]';
const button = 'max-w-full whitespace-normal rounded-[var(--radius-sm)] border border-[var(--line)] px-3 py-2 text-sm font-medium hover:bg-[var(--raised)] disabled:opacity-50 disabled:cursor-not-allowed';
export function parseDns01Inventory(value: unknown): Inventory {
  const fail = () => { throw new Error('Inventaris DNS-01 tidak valid. Muat ulang untuk mencoba kembali.'); };
  if (!value || typeof value !== 'object') return fail();
  const v = value as Inventory;
  if (!v.provider || typeof v.provider.configured !== 'boolean' || !Array.isArray(v.provider.pending) || v.provider.pending.length > 40 || !Array.isArray(v.projects) || v.projects.length > 200 || !Array.isArray(v.orders) || v.orders.length > 20 || !Array.isArray(v.bindings) || v.bindings.length > 200) return fail();
  if (v.provider.configured && [v.provider.zone_name, v.provider.zone_id, v.provider.revision].some(x => typeof x !== 'string' || !x)) return fail();
  if (v.provider.pending.some(x => typeof x.id !== 'string' || typeof x.hostname !== 'string' || !Number.isSafeInteger(x.created_at))) return fail();
  if (v.projects.some(x => !Number.isSafeInteger(x.site_id) || x.site_id <= 0 || typeof x.hostname !== 'string')) return fail();
  if (v.orders.some(x => typeof x.id !== 'string' || !Number.isSafeInteger(x.site_id) || typeof x.job_id !== 'string' || typeof x.staging !== 'boolean' || !['queued', 'issuing', 'candidate', 'active', 'staging', 'failed'].includes(x.state) || (x.error !== null && typeof x.error !== 'string'))) return fail();
  if (v.bindings.some(x => !Number.isSafeInteger(x.site_id) || !Number.isSafeInteger(x.certificate_id) || [x.renew_at, x.not_after].some(n => !Number.isSafeInteger(n) || n < 0 || n > 253402300799) || (x.error !== null && typeof x.error !== 'string'))) return fail();
  return v;
}
export function dns01State(state: DnsOrder['state']) {
  return { queued: 'Dalam antrean', issuing: 'Memeriksa DNS dan menerbitkan', candidate: 'Kandidat perlu dipulihkan', active: 'Terpasang', staging: 'Uji staging selesai, tidak dipasang', failed: 'Gagal' }[state];
}
export function Dns01Settings({ onChanged }: { onChanged?: () => void }) {
  const id = useId(), mounted = useRef(false), sequence = useRef(0), submitting = useRef(false);
  const [inventory, setInventory] = useState<Inventory | null>(null), [loading, setLoading] = useState(true), [busy, setBusy] = useState(false), [error, setError] = useState(''), [message, setMessage] = useState('');
  const [dialog, setDialog] = useState<'token' | 'issue' | null>(null), [zone, setZone] = useState(''), [zoneId, setZoneId] = useState(''), [token, setToken] = useState('');
  const [site, setSite] = useState(''), [email, setEmail] = useState(''), [staging, setStaging] = useState(true), [confirmed, setConfirmed] = useState(false);
  const [discard, setDiscard] = useState<DnsOrder | null>(null), [revoke, setRevoke] = useState(false);
  const [clock, setClock] = useState(() => Date.now() / 1000);
  const load = useCallback(async () => {
    const seq = ++sequence.current;
    try {
      const value = parseDns01Inventory(await request<unknown>('/api/v1/network/dns01'));
      if (mounted.current && seq === sequence.current) { setInventory(value); setError(''); setClock(Date.now() / 1000); }
      return true;
    } catch (e) { if (mounted.current && seq === sequence.current) { setInventory(null); setError(e instanceof Error ? e.message : 'Inventaris DNS-01 belum tersedia.'); } return false; }
    finally { if (mounted.current && seq === sequence.current) setLoading(false); }
  }, []);
  const finished = useCallback(async () => { const okay = await load(); onChanged?.(); return okay; }, [load, onChanged]);
  const jobs = useRuntimeJobs(finished, jobsFilter);
  useEffect(() => { mounted.current = true; void load(); const tick = () => setClock(Date.now() / 1000); const timer = setInterval(tick, 60000); window.addEventListener('focus', tick); return () => { mounted.current = false; sequence.current++; clearInterval(timer); window.removeEventListener('focus', tick); }; }, [load]);
  const locked = busy || loading || jobs.loading || !!jobs.error || !inventory;
  const pending = inventory?.orders.some(o => ['queued', 'issuing', 'candidate'].includes(o.state)) || jobs.jobs.some(j => ['queued', 'running'].includes(j.status));
  const dirty = !!inventory?.provider.pending.length;
  const projectName = (siteId: number) => inventory?.projects.find(p => p.site_id === siteId)?.hostname || `Project ${siteId}`;
  const date = (n: number) => new Date(n * 1000).toLocaleString('id-ID');
  const mutate = async (path: string, method: 'POST' | 'PUT' | 'DELETE', body?: unknown) => {
    if (submitting.current) return;
    submitting.current = true; setBusy(true); setError(''); setMessage('');
    try {
      const result = await request<Partial<JobItem>>(`/api/v1/network/dns01${path}`, { method, ...(body ? { body: JSON.stringify(body) } : {}) });
      if (mounted.current) {
        if (result.id && result.name === 'ssl.dns01.issue') jobs.track(result as JobItem);
        setDialog(null); setToken(''); setDiscard(null); setRevoke(false);
        setMessage(result.name ? 'Tugas tersimpan. Proses tetap berjalan saat Anda berpindah halaman.' : 'Perubahan DNS-01 tersimpan.');
      }
      await load(); onChanged?.();
    } catch (e) { if (mounted.current) setError(e instanceof Error ? e.message : 'Operasi DNS-01 gagal.'); }
    finally { submitting.current = false; if (mounted.current) setBusy(false); }
  };
  return <section aria-labelledby={`${id}-title`} className="space-y-4 border-t border-[var(--line)] pt-6">
    <div className="flex flex-wrap items-center justify-between gap-3"><h3 id={`${id}-title`} className="font-semibold">Sertifikat hostname melalui DNS-01</h3><button className={button} disabled={busy || loading} onClick={() => { setLoading(true); void load(); }}>Muat ulang DNS-01</button></div>
    <p className="text-sm text-[var(--ink-muted)]">Hubungkan token DNS-01 untuk sertifikat project dan Storage Qinpanel, termasuk akses melalui Tunnel tanpa membuka port publik 80. Koneksi Tunnel tidak menghubungkan token DNS-01 ini secara otomatis. Penerbitan hostname di halaman ini memasang sertifikat pada Nginx situs; verifikasi TLS origin tetap aktif. Untuk storage, kembali ke Generate &amp; aktifkan setelah token tersimpan. Sertifikat panel dan IP menggunakan pengaturan konektivitas terpisah.</p>
    <p className="text-sm text-[var(--ink-muted)]">Gunakan token Cloudflare khusus dengan Zone DNS Edit dan Zone Read, dibatasi hanya satu zona. Token disimpan terenkripsi pada agent. Jangan gunakan token Tunnel atau Global API Key. Delegasi CNAME/NS dan wildcard belum didukung.</p>
    {loading && <p role="status" className="text-sm">Memuat DNS-01…</p>}
    {error && <p role="alert" className="text-sm text-[var(--danger)] break-words">{error}</p>}
    {message && <p role="status" className="text-sm">{message}</p>}
    {inventory && <>
      <p className="text-sm">Zona DNS-01: <strong>{inventory.provider.configured ? inventory.provider.zone_name : 'Belum dihubungkan'}</strong></p>
      <div className="flex flex-wrap gap-2">
        <button className={button} disabled={locked || pending} onClick={() => { setZone(inventory.provider.zone_name || ''); setZoneId(inventory.provider.zone_id || ''); setToken(''); setDialog('token'); }}>{inventory.provider.configured ? 'Ganti token DNS-01' : 'Hubungkan token DNS-01'}</button>
        {inventory.provider.configured && <><button className={button} disabled={locked || pending || dirty || !inventory.projects.length} onClick={() => { setSite(''); setStaging(true); setConfirmed(false); setDialog('issue'); }}>Terbitkan atau perbarui hostname</button><button className={button} disabled={locked || pending || dirty} onClick={() => setRevoke(true)}>Lepas token lokal</button></>}
      </div>
      <p className="text-xs text-[var(--ink-muted)]">Seluruh hostname project harus terdaftar di zona yang sama, maksimal lima hostname. Uji staging tidak dipasang. Produksi memakai renewal otomatis berdasarkan masa berlaku. Penerbitan tidak mengubah DNS A/CNAME Tunnel atau mempublikasikan rute.</p>
      {jobs.error && <p role="alert" className="text-sm text-[var(--danger)]">{jobs.error} Lihat rincian pada halaman Tugas.</p>}
      {jobs.jobs.map(j => <RuntimeJobProgress key={j.id} job={j} runtime="sertifikat DNS-01" />)}
      {dirty && <div className="space-y-2"><p className="text-sm">Challenge TXT tertunda. Cleanup mempertahankan record DNS milik pihak lain.</p>{inventory.provider.pending.map(p => <div key={p.id} className="flex flex-wrap items-center gap-3 text-sm"><span className="break-all">{p.hostname}</span><button className={button} disabled={locked || clock - p.created_at < 2400 || jobs.jobs.some(j => j.status === 'running')} onClick={() => void mutate('/cleanup', 'POST', { id: p.id })}>Bersihkan TXT tertunda</button><span className="text-xs text-[var(--ink-muted)]">Tersedia setelah 40 menit untuk melindungi order yang berjalan.</span></div>)}</div>}
      {inventory.bindings.map(b => <div key={b.site_id} className="space-y-1 border-t border-[var(--line)] pt-3 text-sm"><p className="font-medium">{projectName(b.site_id)} · Sertifikat produksi</p><p>{b.not_after <= clock ? 'Kedaluwarsa' : 'Berlaku hingga'}: {date(b.not_after)}</p><p>Jadwal renewal: {date(b.renew_at)}</p>{b.error && <p className="text-[var(--danger)] break-words">{b.error}</p>}</div>)}
      {inventory.orders.map(o => <div key={o.id} className="space-y-2 border-t border-[var(--line)] pt-3 text-sm"><p className="font-medium">{projectName(o.site_id)} · {dns01State(o.state)}</p>{o.error && <p className="text-[var(--danger)] break-words">{o.error}</p>}{o.state === 'candidate' && <div className="flex flex-wrap gap-2"><button className={button} disabled={locked} onClick={() => void mutate(`/${o.id}/retry`, 'POST')}>Pulihkan kandidat yang sama</button><button className={button} disabled={locked} onClick={() => setDiscard(o)}>Buang kandidat</button></div>}<a className="text-[var(--primary)] underline" href="/jobs">Lihat riwayat tugas</a></div>)}
    </>}
    {dialog && <ModalOverlay onDismiss={() => { setDialog(null); setToken(''); }} aria-labelledby={`${id}-dialog`} dismissDisabled={busy}>
      <form className="dialog-panel max-w-lg border border-[var(--line)] p-5 gap-4" onSubmit={e => { e.preventDefault(); if (dialog === 'token') void mutate('', 'PUT', { zone_id: zoneId, zone_name: zone, api_token: token }); else void mutate('/issue', 'POST', { site_id: Number(site), email, staging, confirmed }); }}>
        <h3 id={`${id}-dialog`} className="font-semibold">{dialog === 'token' ? 'Token khusus DNS-01' : 'Terbitkan sertifikat hostname'}</h3>
        <div className="dialog-body"><fieldset disabled={busy} className="min-w-0 space-y-4">{dialog === 'token' ? <>
          <label className="block text-sm">Nama zona<input required autoFocus className={input} placeholder="example.com" readOnly={dirty} value={zone} onChange={e => setZone(e.target.value)} /></label>
          <label className="block text-sm">ID zona<input required pattern="[0-9a-fA-F]{32}" readOnly={dirty} className={input} value={zoneId} onChange={e => setZoneId(e.target.value)} /></label>
          <label className="block text-sm">API token DNS<input required type="password" autoComplete="new-password" minLength={20} maxLength={256} className={input} value={token} onChange={e => setToken(e.target.value)} /></label>
          <p className="text-xs text-[var(--ink-muted)]">Batasi izin token pada zona ini: Zone DNS Edit dan Zone Read. Pemeriksaan awal membaca zona; izin menulis diuji ketika challenge dibuat. Token lama dipertahankan jika pemeriksaan gagal. Jika cleanup tertunda karena token kedaluwarsa, perbarui token pada zona yang sama.</p>
        </> : <>
          <label className="block text-sm">Hostname utama project<select autoFocus required className={input} value={site} onChange={e => setSite(e.target.value)}><option value="">Pilih project</option>{inventory?.projects.map(p => <option key={p.site_id} value={p.site_id}>{p.hostname}</option>)}</select></label>
          <label className="block text-sm">Email akun CA<input required type="email" maxLength={254} className={input} value={email} onChange={e => setEmail(e.target.value)} /></label>
          <label className="flex items-start gap-2 text-sm"><input type="checkbox" checked={staging} onChange={e => setStaging(e.target.checked)} className="mt-1 size-4 shrink-0" />Uji dahulu dengan CA staging. Sertifikat uji tidak dipasang pada origin.</label>
          <label className="flex items-start gap-2 text-sm"><input type="checkbox" required checked={confirmed} onChange={e => setConfirmed(e.target.checked)} className="mt-1 size-4 shrink-0" /><span>Saya mengontrol DNS seluruh hostname project dan menyetujui <a className="underline" href="https://letsencrypt.org/repository/" target="_blank" rel="noreferrer">ketentuan layanan CA</a>.</span></label>
          <p className="text-xs text-[var(--ink-muted)]">TXT sementara dibersihkan setelah validasi. Konflik/delegasi tidak ditimpa. Gagal renewal mempertahankan sertifikat aktif; penerbitan tidak menonaktifkan verifikasi TLS Tunnel.</p>
        </>}
        {error && <p role="alert" className="text-sm text-[var(--danger)]">{error}</p>}
        </fieldset></div><div className="dialog-footer flex flex-wrap justify-end gap-2"><button type="button" className={button} disabled={busy} onClick={() => { setDialog(null); setToken(''); }}>Batal</button><button className={button} disabled={locked || (dialog === 'issue' && (!confirmed || !site))}>{busy ? 'Menyimpan…' : dialog === 'token' ? 'Simpan token DNS' : 'Antrekan DNS-01'}</button></div>
      </form>
    </ModalOverlay>}
    <ConfirmDangerDialog isOpen={!!discard} onClose={() => setDiscard(null)} title="Buang kandidat DNS-01" description="Konfigurasi origin yang tersimpan diterapkan kembali sebelum kandidat dibuang. Sertifikat aktif dipertahankan." confirmMatchText={discard ? projectName(discard.site_id) : ''} confirmButtonLabel="Buang kandidat" isLoading={busy} onConfirm={() => { if (discard) void mutate(`/${discard.id}/discard`, 'POST', { hostname: projectName(discard.site_id) }); }} />
    <ConfirmDangerDialog isOpen={revoke} onClose={() => setRevoke(false)} title="Lepas token DNS lokal" description="Renewal DNS-01 berhenti sampai token dihubungkan kembali. Sertifikat aktif tetap terpasang. Cabut token di Cloudflare secara terpisah." confirmMatchText={inventory?.provider.zone_name || ''} confirmButtonLabel="Lepas token lokal" isLoading={busy} onConfirm={() => void mutate('', 'DELETE')} />
  </section>;
}
