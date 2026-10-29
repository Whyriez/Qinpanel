import { cloneElement, useCallback, useEffect, useId, useRef, useState } from 'react';
import type { FormEvent, ReactElement } from 'react';
import { RefreshCw } from 'lucide-react';
import { checkProjectDns, cloudflareCommand, fetchCloudflareStatus, fetchNetwork, installCloudflare, savePublicAddresses } from '../../api/network';
import type { CloudflareCommand, CloudflarePage, CloudflareStatus, DnsRecord, NetworkOverview, RecordDraft, Tunnel, Zone } from '../../api/network';
import type { JobItem } from '../../types/api';
import { useRuntimeJobs } from '../runtime/useRuntimeJobs';
import { Dns01Settings } from './Dns01Settings';
import { RuntimeJobProgress } from '../runtime/RuntimeJobProgress';

const jobNames = new Set(['cloudflare.change', 'cloudflare.install']);
const inputStyle = 'w-full rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] px-3 py-2 text-sm text-[var(--ink)] focus-visible:outline-2 focus-visible:outline-[var(--primary)]';
const buttonStyle = 'rounded-[var(--radius-sm)] border border-[var(--line)] px-3 py-2 text-sm font-medium hover:bg-[var(--raised)] disabled:opacity-50 disabled:cursor-not-allowed';
const primaryStyle = `${buttonStyle} bg-[var(--primary)] text-white dark:text-[var(--bg)] hover:bg-[var(--primary-hover)]`;
const emptyRecord = (): RecordDraft => ({ kind: 'A', name: '@', content: '', ttl: 1, proxied: false, priority: 10 });
const errorText = (error: unknown) => error instanceof Error ? error.message : 'Permintaan gagal. Coba kembali.';

function Field({ label, children }: { label: string; children: ReactElement<{ id?: string }> }) {
  const id = useId();
  return <div className="min-w-0 space-y-1.5 text-sm"><label htmlFor={id} className="block font-medium">{label}</label>{cloneElement(children, { id })}</div>;
}
function Pager({ page, total, onPage, disabled }: { page: number; total: number; onPage: (page: number) => void; disabled: boolean }) {
  return <div className="flex items-center justify-end gap-3 text-sm"><button className={buttonStyle} disabled={disabled || page <= 1} onClick={() => onPage(page - 1)}>Sebelumnya</button><span>Halaman {page} dari {Math.max(1, total)}</span><button className={buttonStyle} disabled={disabled || page >= total} onClick={() => onPage(page + 1)}>Berikutnya</button></div>;
}

export function NetworkSettings({ initialTab = 'public' }: { initialTab?: 'public' | 'dns' | 'tunnels' | 'certificates' } = {}) {
  const [tab, setTab] = useState(initialTab);
  const [network, setNetwork] = useState<NetworkOverview | null>(null);
  const [status, setStatus] = useState<CloudflareStatus | null>(null);
  const [ipv4, setIpv4] = useState(''); const [ipv6, setIpv6] = useState('');
  const [account, setAccount] = useState(''); const [token, setToken] = useState('');
  const [busy, setBusy] = useState(false); const submitting = useRef(false);
  const [error, setError] = useState(''); const [message, setMessage] = useState('');
  const [statusError, setStatusError] = useState(''); const [statusLoading, setStatusLoading] = useState(true);
  const [zones, setZones] = useState<CloudflarePage<Zone> | null>(null);
  const [zoneId, setZoneId] = useState(''); const [newZone, setNewZone] = useState('');
  const [records, setRecords] = useState<CloudflarePage<DnsRecord> | null>(null);
  const [draft, setDraft] = useState(emptyRecord); const [editing, setEditing] = useState<string | null>(null);
  const [tunnels, setTunnels] = useState<CloudflarePage<Tunnel> | null>(null);
  const [tunnelName, setTunnelName] = useState(''); const [selectedTunnel, setSelectedTunnel] = useState('');
  const [projectDomain, setProjectDomain] = useState(''); const [dnsResult, setDnsResult] = useState('');
  const [deleteRecord, setDeleteRecord] = useState<DnsRecord | null>(null);
  const [removeTunnel, setRemoveTunnel] = useState<Tunnel | null>(null);
  const [confirmName, setConfirmName] = useState('');
  const mounted = useRef(true);
  const zoneRequest = useRef(0); const recordRequest = useRef(0); const tunnelRequest = useRef(0);
  const statusRequest = useRef(0);
  const statusFlight = useRef<Promise<boolean> | null>(null); const statusAbort = useRef<AbortController | null>(null);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; statusAbort.current?.abort(); }; }, []);

  const loadZones = useCallback(async (page = 1) => {
    const seq = ++zoneRequest.current;
    const value = await cloudflareCommand<CloudflarePage<Zone>>({ action: 'zones', page });
    if (mounted.current && seq === zoneRequest.current) setZones(value);
  }, []);
  const loadRecords = useCallback(async (id: string, page = 1) => {
    const seq = ++recordRequest.current;
    try {
      const value = await cloudflareCommand<CloudflarePage<DnsRecord>>({ action: 'records', zone_id: id, page });
      if (mounted.current && seq === recordRequest.current) setRecords(value);
    } catch (cause) { if (mounted.current && seq === recordRequest.current) throw cause; }
  }, []);
  const loadTunnels = useCallback(async (page = 1) => {
    const seq = ++tunnelRequest.current;
    const value = await cloudflareCommand<CloudflarePage<Tunnel>>({ action: 'tunnels', page });
    if (mounted.current && seq === tunnelRequest.current) setTunnels(value);
  }, []);
  const refreshStatus = useCallback(() => {
    if (statusFlight.current && !statusAbort.current?.signal.aborted) return statusFlight.current;
    const seq = ++statusRequest.current;
    const controller = new AbortController(); statusAbort.current = controller;
    setStatusLoading(true);
    const pending = (async () => {
      try {
        const current = await fetchCloudflareStatus(controller.signal);
        if (mounted.current && seq === statusRequest.current) { setStatus(current); setStatusError(''); setAccount(value => value || current.account_id || ''); }
        return true;
      } catch (cause) { if (mounted.current && seq === statusRequest.current) setStatusError(errorText(cause)); return false; }
      finally { if (seq === statusRequest.current) { statusFlight.current = null; if (mounted.current) setStatusLoading(false); } }
    })();
    statusFlight.current = pending;
    return pending;
  }, []);
  const finished = useCallback(async (job: JobItem) => {
    const okay = await refreshStatus();
    // Lists may require separate token permissions; failure must be visible.
    try { if (okay) await loadTunnels(); } catch (cause) { if (mounted.current) setError(errorText(cause)); }
    if (mounted.current) {
      if (job.status === 'succeeded') setMessage(okay ? 'Operasi Cloudflare selesai. Status server telah diperbarui.' : 'Tugas selesai. Menunggu verifikasi status server.');
      else { setMessage(''); if (job.status === 'failed') setError(job.error_message || 'Operasi Cloudflare gagal. Lihat rincian pada halaman Tugas.'); }
    }
    return okay;
  }, [loadTunnels, refreshStatus]);
  const runtime = useRuntimeJobs(finished, jobNames);
  const disabled = busy || runtime.loading || !!runtime.error || runtime.jobs.length > 0;

  useEffect(() => {
    if (tab !== 'tunnels' || !status?.configured || busy || runtime.jobs.length > 0) return;
    let active = true; let checking = false;
    const timer = setInterval(() => {
      if (checking) return;
      checking = true;
      void (async () => {
        try { const okay = await refreshStatus(); if (active && okay) await loadTunnels(); }
        catch (cause) { if (active && mounted.current) setError(errorText(cause)); }
        finally { checking = false; }
      })();
    }, 10000);
    return () => { active = false; clearInterval(timer); };
  }, [tab, status?.configured, busy, runtime.jobs.length, refreshStatus, loadTunnels]);

  useEffect(() => {
    let active = true;
    void fetchNetwork().then(net => {
      if (active) { setNetwork(net); setIpv4(net.ipv4 || ''); setIpv6(net.ipv6 || ''); }
    }).catch(cause => { if (active) setError(errorText(cause)); });
    void refreshStatus();
    return () => { active = false; };
  }, [refreshStatus]);
  useEffect(() => {
    if (!status?.configured || tab === 'public' || tab === 'certificates') return;
    void loadZones().catch(cause => { if (mounted.current) setError(errorText(cause)); });
    if (tab === 'tunnels') void loadTunnels().catch(cause => { if (mounted.current) setError(errorText(cause)); });
  }, [status?.configured, tab, loadZones, loadTunnels]);
  useEffect(() => {
    setRecords(null); setEditing(null); setDraft(emptyRecord()); setDeleteRecord(null); setProjectDomain('');
    recordRequest.current++;
    if (zoneId && tab === 'dns') void loadRecords(zoneId).catch(cause => { if (mounted.current) setError(errorText(cause)); });
  }, [zoneId, tab, loadRecords]);

  const run = async (operation: () => Promise<void>) => {
    if (submitting.current || disabled) return;
    submitting.current = true; setBusy(true); setError(''); setMessage('');
    try { await operation(); } catch (cause) { if (mounted.current) setError(errorText(cause)); }
    finally { submitting.current = false; if (mounted.current) setBusy(false); }
  };
  const queue = (command: CloudflareCommand) => run(async () => {
    const job = await cloudflareCommand<JobItem>(command);
    runtime.track(job); setMessage('Perubahan masuk antrean. Progres tetap tersedia saat Anda berpindah halaman.');
  });
  const query = (operation: () => Promise<unknown>) => { void run(async () => { await operation(); }); };
  const selectedZone = zones?.items.find(zone => zone.id === zoneId);

  return <div className="space-y-6 text-[var(--ink)]" data-testid="network-settings">
    <div><h2 className="text-lg font-semibold">DNS dan Cloudflare</h2><p className="mt-1 text-sm text-[var(--ink-muted)]">Hubungkan domain ke IP publik server atau terbitkan project melalui Cloudflare Tunnel.</p></div>
    <nav aria-label="Pengaturan koneksi domain" className="settings-section-tabs settings-network-tabs flex flex-wrap gap-2 border-b border-[var(--line)] pb-3">
      {([['public', 'IP publik'], ['dns', 'Cloudflare DNS'], ['tunnels', 'Cloudflare Tunnel'], ['certificates', 'Sertifikat DNS-01']] as const).map(([key, label]) => <button key={key} className={tab === key ? primaryStyle : buttonStyle} aria-pressed={tab === key} onClick={() => { setTab(key); setError(''); setMessage(''); }}>{label}</button>)}
    </nav>
    {(error || statusError || runtime.error) && <div role="alert" className="border-l-2 border-[var(--danger)] pl-3 text-sm text-[var(--danger)]">{error || statusError || runtime.error}<button className={`${buttonStyle} ml-3`} disabled={busy || statusLoading} onClick={() => { void refreshStatus(); }}>{statusLoading ? 'Memuat status…' : 'Muat ulang status'}</button></div>}
    {statusLoading && <p role="status" className="text-sm text-[var(--ink-muted)]">Memuat status Cloudflare…</p>}
    {message && <p role="status" className="text-sm text-[var(--ink-muted)]">{message}</p>}
    {runtime.jobs.map(job => <div key={job.id} className="space-y-2 border border-[var(--line)] rounded-[var(--radius-md)] p-4"><p className="text-sm font-medium">{job.name === 'cloudflare.install' ? 'Pemasangan cloudflared' : 'Perubahan tunnel'} · {job.status === 'queued' ? 'Dalam antrean' : job.status === 'succeeded' ? 'Memverifikasi status' : 'Sedang berjalan'}</p><RuntimeJobProgress job={job} runtime="Cloudflare" /></div>)}
    {busy && <p role="status" className="flex items-center gap-2 text-sm"><RefreshCw className="h-4 w-4 animate-spin" />Memproses permintaan…</p>}

    {tab === 'certificates' && <Dns01Settings />}
    {tab === 'public' && <section className="space-y-5" aria-label="DNS IP publik">
      <div><h3 className="font-semibold">Alamat publik server</h3><p className="mt-1 text-sm text-[var(--ink-muted)]">Masukkan IP publik atau IP NAT yang mengarah ke server ini. Nilai ini digunakan untuk panduan record dan pemeriksaan DNS.</p></div>
      {!network ? <p role="status">Konfigurasi jaringan belum berhasil dimuat.</p> : <>
        <form className="space-y-4" onSubmit={e => { e.preventDefault(); void run(async () => { await savePublicAddresses(ipv4, ipv6); setNetwork(await fetchNetwork()); setMessage('IP publik disimpan.'); }); }}>
          <div className="grid gap-4 md:grid-cols-2"><Field label="IPv4 publik"><input className={inputStyle} value={ipv4} onChange={e => setIpv4(e.target.value)} placeholder="Contoh: 203.0.113.10" autoComplete="off" /></Field><Field label="IPv6 publik (opsional)"><input className={inputStyle} value={ipv6} onChange={e => setIpv6(e.target.value)} placeholder="Alamat IPv6 server" autoComplete="off" /></Field></div>
          <button className={primaryStyle} disabled={disabled}>Simpan IP publik</button>
        </form>
        <div className="space-y-3 border-t border-[var(--line)] pt-5"><h3 className="font-semibold">Record DNS project</h3><p className="text-sm text-[var(--ink-muted)]">Tambahkan record berikut di penyedia DNS domain. Jika domain memakai Cloudflare, record dapat dikelola pada tab Cloudflare DNS. Pemeriksaan menggunakan resolver server; proxy Cloudflare akan menampilkan IP Cloudflare.</p>
          {network.projects.length === 0 ? <p className="text-sm">Belum ada domain project. Tambahkan domain pada menu Situs.</p> : <div className="overflow-x-auto"><table className="w-full text-left text-sm"><thead><tr className="border-b border-[var(--line)]"><th className="p-3">Hostname</th><th className="p-3">Record yang diperlukan</th><th className="p-3"><span className="sr-only">Pemeriksaan</span></th></tr></thead><tbody>{network.projects.map(project => <tr key={project.domain_id} className="border-b border-[var(--line)]"><td className="p-3 break-all"><a href={`/sites/${project.site_id}`} className="text-[var(--primary)] underline">{project.hostname}</a></td><td className="p-3">{network.ipv4 && <div>A → <code>{network.ipv4}</code></div>}{network.ipv6 && <div>AAAA → <code>{network.ipv6}</code></div>}{!network.ipv4 && !network.ipv6 && 'Simpan IP publik terlebih dahulu.'}</td><td className="p-3"><button className={buttonStyle} disabled={disabled || (!network.ipv4 && !network.ipv6)} onClick={() => { void run(async () => { const result = await checkProjectDns(project.domain_id); setDnsResult(`${result.hostname}: ${result.matches ? 'Sesuai dengan IP server' : 'Belum sesuai dengan IP server'}. Hasil resolver: ${result.addresses.join(', ') || 'tidak ada'}.`); }); }}>Periksa DNS</button></td></tr>)}</tbody></table></div>}
          {dnsResult && <p role="status" className="text-sm break-words">{dnsResult}</p>}
        </div>
      </>}
    </section>}

    {(tab === 'dns' || tab === 'tunnels') && <>
      <section className="space-y-4 border-b border-[var(--line)] pb-5" aria-label="Koneksi akun Cloudflare">
        <div><h3 className="font-semibold">Akun Cloudflare · {status?.configured ? 'Terhubung' : status ? 'Belum terhubung' : 'Status belum tersedia'}</h3><p className="mt-1 text-sm text-[var(--ink-muted)]">Gunakan API token dengan izin Account Settings Read, Zone Read, dan DNS Edit. Tambahkan Zone Edit untuk domain baru, serta Cloudflare Tunnel Edit untuk tunnel. Token dibuat sekali di akun Cloudflare dan disimpan terenkripsi di server.</p></div>
        <form onSubmit={(e: FormEvent) => { e.preventDefault(); void run(async () => { await cloudflareCommand({ action: 'configure', account_id: account.trim(), api_token: token.trim() }); setToken(''); await refreshStatus(); await loadZones(); setMessage('Akun Cloudflare tersambung. Token tidak ditampilkan kembali.'); }); }} className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2"><Field label="Account ID"><input required pattern="[a-fA-F0-9]{32}" className={inputStyle} value={account} onChange={e => setAccount(e.target.value)} autoComplete="off" /></Field><Field label={status?.configured ? 'API token baru' : 'API token'}><input required type="password" className={inputStyle} value={token} onChange={e => setToken(e.target.value)} autoComplete="new-password" /></Field></div>
          <button className={buttonStyle} disabled={disabled || !status}>{status?.configured ? 'Perbarui koneksi' : 'Hubungkan akun'}</button>
        </form>
      </section>
      {status?.configured && <>
        <section className="space-y-4" aria-label="Domain Cloudflare"><div className="flex flex-wrap items-center justify-between gap-3"><h3 className="font-semibold">Domain Cloudflare</h3><button className={buttonStyle} disabled={disabled} onClick={() => query(() => loadZones(zones?.pagination.page || 1))}>Muat ulang domain</button></div>
          <div className="grid gap-4 md:grid-cols-2"><Field label="Pilih domain"><select className={inputStyle} value={zoneId} onChange={e => setZoneId(e.target.value)}><option value="">Pilih domain Cloudflare</option>{zones?.items.map(zone => <option value={zone.id} key={zone.id}>{zone.name} · {zone.status}</option>)}</select></Field>
            {tab === 'dns' && <form className="space-y-2" onSubmit={e => { e.preventDefault(); void run(async () => { const zone = await cloudflareCommand<Zone>({ action: 'create_zone', name: newZone.trim() }); setNewZone(''); await loadZones(); setZoneId(zone.id); setMessage(`Domain ditambahkan. Atur nameserver di registrar: ${(zone.name_servers || []).join(', ')}.`); }); }}><Field label="Tambahkan domain"><input required className={inputStyle} value={newZone} onChange={e => setNewZone(e.target.value)} placeholder="example.com" /></Field><button className={buttonStyle} disabled={disabled || !zones}>Tambah domain</button></form>}
          </div>
          {zones && <Pager page={zones.pagination.page} total={zones.pagination.total_pages} disabled={disabled} onPage={page => query(() => loadZones(page))} />}
          {zones?.items.length === 0 && <p className="text-sm">Belum ada domain yang dapat diakses token ini.</p>}
          {selectedZone && <p className="text-sm text-[var(--ink-muted)] break-words">Status {selectedZone.status}. Nameserver: {selectedZone.name_servers?.join(', ') || 'Belum tersedia'}. Penggantian nameserver dilakukan di registrar domain.</p>}
        </section>

        {tab === 'dns' && zoneId && <section className="space-y-4 border-t border-[var(--line)] pt-5" aria-label="Record Cloudflare">
          <div className="flex flex-wrap justify-between gap-3"><h3 className="font-semibold">Record DNS</h3><button className={buttonStyle} disabled={disabled} onClick={() => query(() => loadRecords(zoneId, records?.pagination.page || 1))}>Muat ulang record</button></div>
          {!records ? <p role="status">Record belum dimuat.</p> : <>
            <div className="overflow-x-auto"><table className="w-full text-sm text-left"><thead><tr className="border-b border-[var(--line)]">{['Jenis', 'Nama', 'Isi', 'TTL / proxy', 'Tindakan'].map(label => <th className="p-3" key={label}>{label}</th>)}</tr></thead><tbody>{records.items.map(record => <tr key={record.id} className="border-b border-[var(--line)]"><td className="p-3">{record.type}</td><td className="p-3 break-all">{record.name}</td><td className="p-3 break-all min-w-40">{record.content}</td><td className="p-3">{record.ttl === 1 ? 'Otomatis' : `${record.ttl} dtk`} · {record.proxied ? 'Proxy aktif' : 'DNS saja'}</td><td className="p-3"><div className="flex gap-2"><button className={buttonStyle} disabled={disabled || !['A','AAAA','CNAME','TXT','MX','NS'].includes(record.type)} onClick={() => { setEditing(record.id); setDraft({ kind: record.type, name: record.name, content: record.content, ttl: record.ttl, proxied: record.proxied, priority: record.priority || 10 }); }}>Ubah<span className="sr-only"> {record.name}</span></button><button className={buttonStyle} disabled={disabled} onClick={() => setDeleteRecord(record)}>Hapus<span className="sr-only"> {record.name}</span></button></div></td></tr>)}</tbody></table></div>
            {records.items.length === 0 && <p className="text-sm">Belum ada record pada domain ini.</p>}
            <Pager page={records.pagination.page} total={records.pagination.total_pages} disabled={disabled} onPage={page => query(() => loadRecords(zoneId, page))} />
            {deleteRecord && <div role="group" aria-label="Konfirmasi hapus record" className="space-y-3 border border-[var(--danger)] p-4"><p className="text-sm">Hapus record {deleteRecord.type} {deleteRecord.name}? Layanan yang menggunakannya dapat terputus.</p><div className="flex gap-2"><button className={buttonStyle} disabled={disabled} onClick={() => { void run(async () => { await cloudflareCommand({ action:'delete_record',zone_id:zoneId,record_id:deleteRecord.id }); setDeleteRecord(null); await loadRecords(zoneId); setMessage('Record dihapus.'); }); }}>Ya, hapus record</button><button className={buttonStyle} onClick={() => setDeleteRecord(null)}>Batal</button></div></div>}
            <form className="space-y-4" onSubmit={e => { e.preventDefault(); void run(async () => { await cloudflareCommand({ action:'save_record',zone_id:zoneId,record_id:editing,record:draft }); setEditing(null); setDraft(emptyRecord()); await loadRecords(zoneId); setMessage('Record disimpan. Perubahan DNS dapat memerlukan waktu untuk tersebar.'); }); }}>
              <h4 className="font-medium">{editing ? 'Ubah record' : 'Tambah record'}</h4>
              <div className="grid gap-4 md:grid-cols-2"><Field label="Jenis record"><select className={inputStyle} value={draft.kind} onChange={e => setDraft({ ...draft, kind:e.target.value, proxied:false })}>{['A','AAAA','CNAME','TXT','MX','NS'].map(type => <option key={type}>{type}</option>)}</select></Field><Field label="Nama lengkap atau @"><input required className={inputStyle} value={draft.name} onChange={e => setDraft({ ...draft,name:e.target.value })} placeholder="app.example.com" /></Field><Field label="Isi record"><textarea required rows={2} className={inputStyle} value={draft.content} onChange={e => setDraft({ ...draft,content:e.target.value })} /></Field><Field label="TTL (1 = otomatis)"><input required type="number" min={1} max={86400} className={inputStyle} value={draft.ttl} onChange={e => setDraft({ ...draft,ttl:Number(e.target.value) })} /></Field>{draft.kind === 'MX' && <Field label="Prioritas"><input required type="number" min={0} max={65535} className={inputStyle} value={draft.priority} onChange={e => setDraft({ ...draft,priority:Number(e.target.value) })} /></Field>}</div>
              {['A','AAAA','CNAME'].includes(draft.kind) && <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={draft.proxied} onChange={e => setDraft({ ...draft,proxied:e.target.checked })} />Aktifkan proxy Cloudflare</label>}
              <div className="flex gap-2"><button className={primaryStyle} disabled={disabled}>Simpan record</button>{editing && <button type="button" className={buttonStyle} onClick={() => { setEditing(null); setDraft(emptyRecord()); }}>Batal ubah</button>}</div>
            </form>
          </>}
        </section>}

        {tab === 'tunnels' && <section className="space-y-5 border-t border-[var(--line)] pt-5" aria-label="Pengelolaan tunnel">
          <div className="flex flex-wrap items-center justify-between gap-3"><div><h3 className="font-semibold">Konektor server · {status.connector_installed ? 'Terpasang' : 'Belum terpasang'}</h3><p className="mt-1 text-sm text-[var(--ink-muted)]">Konektor berjalan sebagai layanan server. Status lokal dan kesehatan koneksi Cloudflare diperiksa secara terpisah.</p></div>{!status.connector_installed && <button className={primaryStyle} disabled={disabled} onClick={() => { void run(async () => { runtime.track(await installCloudflare()); }); }}>Pasang cloudflared</button>}</div>
          <form className="flex flex-wrap items-end gap-3" onSubmit={e => { e.preventDefault(); void queue({ action:'create_tunnel',name:tunnelName.trim() }); }}><div className="w-full min-w-0 sm:w-auto sm:flex-1"><Field label="Nama tunnel baru"><input required pattern="[a-zA-Z0-9_-]{1,80}" className={inputStyle} value={tunnelName} onChange={e => setTunnelName(e.target.value)} placeholder="project-server" /></Field></div><button className={buttonStyle} disabled={disabled}>Buat tunnel</button><button type="button" className={buttonStyle} disabled={disabled} onClick={() => query(async () => { if (await refreshStatus()) await loadTunnels(tunnels?.pagination.page || 1); })}>Muat ulang tunnel</button></form>
          {!tunnels ? <p role="status">Daftar tunnel belum dimuat.</p> : <>
            <div className="divide-y divide-[var(--line)]">{tunnels.items.map(tunnel => { const local = status.managed_tunnels.find(t => t.id === tunnel.id); return <article key={tunnel.id} className="space-y-3 py-4"><div className="flex flex-wrap items-start justify-between gap-3"><div className="min-w-0"><h4 className="font-medium break-all">{tunnel.name}</h4><p className="mt-1 text-sm text-[var(--ink-muted)]">Cloudflare: {tunnel.status || 'Belum diketahui'} · Lokal: {local?.local_status || 'Belum terhubung ke panel'}</p></div><div className="flex flex-wrap gap-2">{!tunnel.managed ? <button className={buttonStyle} disabled={disabled || !tunnel.remote_config} onClick={() => { void queue({action:'tunnel_action',tunnel_id:tunnel.id,operation:'import'}); }}>Hubungkan ke panel<span className="sr-only"> {tunnel.name}</span></button> : <><button className={buttonStyle} disabled={disabled || !status.connector_installed || local?.local_status === 'active'} onClick={() => { void queue({action:'tunnel_action',tunnel_id:tunnel.id,operation:'start'}); }}>Mulai<span className="sr-only"> {tunnel.name}</span></button><button className={buttonStyle} disabled={disabled || local?.local_status !== 'active'} onClick={() => { void queue({action:'tunnel_action',tunnel_id:tunnel.id,operation:'stop'}); }}>Hentikan<span className="sr-only"> {tunnel.name}</span></button><button className={buttonStyle} disabled={disabled || !!local?.routes.length} onClick={() => { setRemoveTunnel(tunnel); setConfirmName(''); }}>Hapus tunnel<span className="sr-only"> {tunnel.name}</span></button></>}</div></div>{local?.routes.map(route => <div key={route.hostname} className="flex flex-wrap justify-between items-center gap-3 text-sm"><a className="text-[var(--primary)] underline break-all" href={`https://${route.hostname}`} target="_blank" rel="noreferrer">{route.hostname}</a><button className={buttonStyle} disabled={disabled} onClick={() => { void queue({action:'unpublish',tunnel_id:tunnel.id,zone_id:route.zone_id,hostname:route.hostname}); }}>Lepaskan hostname<span className="sr-only"> {route.hostname}</span></button></div>)}</article>; })}</div>
            {tunnels.items.length === 0 && <p className="text-sm">Belum ada tunnel pada akun ini.</p>}
            <Pager page={tunnels.pagination.page} total={tunnels.pagination.total_pages} disabled={disabled} onPage={page => query(() => loadTunnels(page))} />
          </>}
          {removeTunnel && <div role="group" aria-label="Konfirmasi hapus tunnel" className="space-y-3 border border-[var(--danger)] p-4"><p className="text-sm">Tunnel akan dihapus dari akun Cloudflare dan layanan lokal. Ketik <strong>{removeTunnel.name}</strong> untuk melanjutkan.</p><Field label="Konfirmasi nama tunnel"><input className={inputStyle} value={confirmName} onChange={e => setConfirmName(e.target.value)} /></Field><div className="flex gap-2"><button className={buttonStyle} disabled={disabled || confirmName !== removeTunnel.name} onClick={() => { void queue({action:'tunnel_action',tunnel_id:removeTunnel.id,operation:'remove'}); setRemoveTunnel(null); }}>Hapus permanen</button><button className={buttonStyle} onClick={() => setRemoveTunnel(null)}>Batal</button></div></div>}
          <form className="space-y-4 border-t border-[var(--line)] pt-5" onSubmit={e => { e.preventDefault(); void queue({action:'publish',zone_id:zoneId,tunnel_id:selectedTunnel,hostname:projectDomain,tls:false}); }}>
            <div><h3 className="font-semibold">Terbitkan hostname project</h3><p className="mt-1 text-sm text-[var(--ink-muted)]">Pilih domain Cloudflare di atas. Panel membuat rute tunnel dan CNAME, lalu mengarahkan trafik melalui Nginx project. Hostname harus belum memiliki record DNS. Mulai konektor untuk membuka akses publik.</p></div>
            <div className="grid gap-4 md:grid-cols-2"><Field label="Tunnel tujuan"><select required className={inputStyle} value={selectedTunnel} onChange={e => setSelectedTunnel(e.target.value)}><option value="">Pilih tunnel yang dikelola</option>{tunnels?.items.filter(t => t.managed).map(t => <option key={t.id} value={t.id}>{t.name}</option>)}</select></Field><Field label="Hostname project"><select required className={inputStyle} value={projectDomain} onChange={e => setProjectDomain(e.target.value)}><option value="">Pilih domain project</option>{network?.projects.filter(p => p.status === 'active' && selectedZone && (p.hostname === selectedZone.name || p.hostname.endsWith(`.${selectedZone.name}`))).map(p => <option key={p.domain_id} value={p.hostname}>{p.hostname}</option>)}</select></Field></div>
            <button className={primaryStyle} disabled={disabled || !zoneId || !selectedTunnel || !projectDomain}>Terbitkan project</button>
          </form>
        </section>}
      </>}
    </>}
  </div>;
}
