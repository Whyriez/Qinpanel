import { Children, cloneElement, isValidElement, useCallback, useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Copy, Plus, RefreshCw, X } from 'lucide-react';
import { useAuth } from '../../context/AuthContext';
import { RouteLink } from '../../lib/router';
import { ModalOverlay } from '../ui/ModalOverlay';
import { PanelUrlSettings } from './PanelUrlSettings';
import { PanelTlsSettings } from './PanelTlsSettings';
import { ConfirmDangerDialog } from '../ui/ConfirmDangerDialog';
import {
  availableTransports, createEndpoint, deleteEndpoint, diagnoseEndpoint, endpointAuthority, fetchConnectivity, fetchEndpoint,
  isCurrentEvidence, newCreateKey, presetConfig, replaceEndpoint, serviceName, services, transportName,
  type ConnectivityOverview, type Endpoint, type EndpointConfig, type Evidence, type Preset, type Service,
} from '../../api/connectivity';

const input = 'w-full min-w-0 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] px-3 py-2 text-sm text-[var(--ink)]';
const message = (error: unknown) => error instanceof Error ? error.message : 'Permintaan gagal. Coba kembali.';
const initial = (): EndpointConfig => ({ service: 'panel', scope: { kind: 'host' }, host: '', port: 17890, transport: 'direct', tls_mode: 'none', certificate_id: null, primary: true, enabled: true });
const defaultPort: Record<Service, number> = { panel: 17890, website: 443, ssh: 22, database: 3306, imap: 993, submission: 587, smtp_inbound: 25 };
const reasonNames: Record<string, [string, string]> = {
  policy_blocked: ['Target dibatasi kebijakan probe', 'Target restricted by probe policy'], family_unavailable: ['Alamat keluarga IP ini belum ditemukan', 'No address found for this IP family'],
  connection_refused: ['Koneksi TCP ditolak', 'TCP connection refused'], agent_unavailable: ['Diagnosis agent belum tersedia', 'Agent diagnosis unavailable'],
  local_guard_only: ['Guard loopback terdeteksi; izin inbound belum diketahui', 'Loopback guard detected; inbound access unknown'], network_unavailable: ['Jaringan dari server belum terbukti', 'Network from server unverified'],
  verified: ['Terverifikasi', 'Verified'], not_tested: ['Belum diuji', 'Not tested'], unsupported: ['Belum didukung', 'Unsupported'], timeout: ['Batas waktu tercapai', 'Timed out'],
  service_missing: ['Layanan belum terpasang', 'Service not installed'], service_stopped: ['Layanan berhenti', 'Service stopped'], dns_failure: ['DNS gagal', 'DNS failure'],
  connector_unavailable: ['Konektor tidak tersedia', 'Connector unavailable'], tls_failure: ['TLS gagal', 'TLS failure'], authentication_denied: ['Autentikasi ditolak', 'Authentication denied'],
  listener_unavailable: ['Listener tidak tersedia', 'Listener unavailable'], port_blocked: ['Port terhalang', 'Port blocked'], provider_pending: ['Provider belum siap', 'Provider pending'],
};
const englishActions: Record<string, string> = {
  policy_blocked: 'Public probes cannot use this target. Use local diagnosis or a client on the appropriate network.', family_unavailable: 'The server resolver returned no address for this family. Review A/AAAA records.',
  connection_refused: 'The target refused TCP from this server. Review listeners and network rules.', agent_unavailable: 'Review the agent connection and retry.',
  local_guard_only: 'The loopback guard does not prove inbound access. Review host, provider and router firewalls separately.', network_unavailable: 'Review server routing and test from an external client.',
  verified: 'No action is required for this check.', not_tested: 'Run a check for the selected connection route.', unsupported: 'Choose a supported route or use browser access.',
  timeout: 'Review the connection route and retry the check.', service_missing: 'Install the service using its manager.', service_stopped: 'Review service logs and start the service.',
  dns_failure: 'Review the endpoint DNS records.', connector_unavailable: 'Review the connector and Tunnel route.', tls_failure: 'Review the endpoint certificate and TLS policy.',
  authentication_denied: 'Review service permissions and authentication.', listener_unavailable: 'Ask the administrator to review the service listener.', port_blocked: 'Ask the administrator to review port and network rules.', provider_pending: 'Complete provider configuration and verification.',
};
function useCopy() {
  const { i18n } = useTranslation();
  const en = i18n.language.startsWith('en');
  return { en, text: (id: string, english: string) => en ? english : id };
}
function ConnectivityField({ label, children }: { label: string; children: ReactNode }) {
  const id = useId();
  return <div className="connectivity-field"><label htmlFor={id}>{label}</label>{Children.toArray(children).map((child, index) => index === 0 && isValidElement<{ id?: string }>(child) ? cloneElement(child, { id }) : child)}</div>;
}
const checkNames: Record<Evidence['kind'], [string, string]> = { installation: ['Pemasangan', 'Installation'], service: ['Layanan lokal', 'Local service'], route: ['TCP dari server', 'TCP from server'], listener: ['Listener lokal (termasuk loopback)', 'Local listener (including loopback)'], firewall: ['Firewall milik panel', 'Panel-managed firewall'], provider: ['Konfigurasi provider', 'Provider configuration'], dns: ['DNS', 'DNS'], tls: ['TLS', 'TLS'], authentication: ['Autentikasi', 'Authentication'], external_reachability: ['Koneksi dari luar', 'External connection'] };

export function ConnectivitySummary({ data, now = Date.now(), manage = false, disabled = false, filter = '', checkingId = '', onCheck, onEdit, onDelete }: {
  data: ConnectivityOverview; now?: number; manage?: boolean; disabled?: boolean; filter?: Service | ''; checkingId?: string; onCheck?: (e: Endpoint) => void; onEdit?: (e: Endpoint) => void; onDelete?: (e: Endpoint) => void;
}) {
  const { en, text } = useCopy();
  const [copied, setCopied] = useState('');
  const groups = (manage ? services : services.filter(s => (data.service_counts[s] || 0) > 0 || ['website', 'ssh', 'imap', 'submission', 'smtp_inbound'].includes(s))).filter(s => !filter || s === filter);
  const scopeName = (e: Endpoint) => e.scope.kind === 'host' ? text('Server', 'Server') : e.scope.kind === 'engine' ? (e.scope.engine === 'mariadb' ? 'MariaDB' : 'PostgreSQL') : data.projects.find(p => e.scope.kind === 'project' && p.id === e.scope.site_id)?.name || text('Project tidak tersedia', 'Project unavailable');
  const copy = async (e: Endpoint) => {
    try { await navigator.clipboard.writeText(endpointAuthority(e)); setCopied(text('Alamat disalin.', 'Address copied.')); }
    catch { setCopied(text('Alamat gagal disalin. Pilih teks alamat untuk menyalinnya.', 'Copy failed. Select the address text to copy it.')); }
  };
  return <div className="connectivity-summary">
    {copied && <p role="status" className="text-sm">{copied}</p>}
    {groups.map(service => {
      const rows = data.endpoints.filter(row => row.endpoint.service === service);
      return <section key={service} className="connectivity-service" aria-label={serviceName(service, en)}>
        <h3 className="font-semibold text-sm">{serviceName(service, en)}</h3>
        {!rows.length && <p className="text-sm text-[var(--ink-muted)]">{data.service_counts[service] ? `${data.service_counts[service]} ${text('alamat terdaftar pada halaman lain. Gunakan filter layanan untuk melihatnya.', 'addresses listed on other pages. Use the service filter to view them.')}` : text('Belum diatur', 'Not configured')}</p>}
        {rows.map(({ endpoint: e, evidence }) => {
          const current = evidence.filter(r => isCurrentEvidence(e, r, now));
          const external = current.filter(r => r.kind === 'external_reachability');
          return <article key={e.id} className="connectivity-endpoint" data-testid={`endpoint-${e.id}`}>
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="min-w-0 space-y-1">
                <p className="text-xs text-[var(--ink-muted)]">{scopeName(e)} · {e.primary ? text('Utama', 'Primary') : text('Alternatif', 'Alternative')}{!e.enabled && ` · ${text('Dinonaktifkan', 'Disabled')}`}</p>
                <div className="flex items-start gap-2"><code className="break-all text-sm">{endpointAuthority(e)}</code><button type="button" className="shrink-0 p-1.5" aria-label={`${text('Salin alamat', 'Copy address')} ${e.host}`} onClick={() => void copy(e)}><Copy size={15} aria-hidden="true" /></button></div>
                <p className="text-sm text-[var(--ink-muted)]">{transportName(e.transport, en)} · {e.tls_mode === 'none' ? text('Tanpa TLS', 'No TLS') : e.tls_mode.toUpperCase()}</p>
                <p className="text-xs">{e.client_requirement === 'cloudflared' ? text('Memerlukan cloudflared pada perangkat client.', 'Requires cloudflared on the client device.') : e.client_requirement === 'cloudflare_one' ? text('Memerlukan Cloudflare One Client yang terdaftar pada organisasi.', 'Requires an organization-enrolled Cloudflare One Client.') : text('Tidak memerlukan aplikasi pendamping Tunnel.', 'No Tunnel companion application required.')}</p>
              </div>
              {manage && <div className="flex flex-wrap gap-2"><button type="button" className="button-secondary" disabled={disabled} onClick={() => onEdit?.(e)}>{text('Ubah', 'Edit')}<span className="sr-only"> {e.host}</span></button><button type="button" className="button-secondary" disabled={disabled} onClick={() => onDelete?.(e)}>{text('Hapus', 'Delete')}<span className="sr-only"> {e.host}</span></button></div>}
            </div>
            <div className="flex flex-wrap gap-x-5 gap-y-2 text-xs" aria-label={text('Ringkasan pemeriksaan', 'Check summary')}>
              {(['installation', 'service', 'route', 'listener', 'firewall', 'provider', 'dns', 'tls'] as const).map(kind => {
                const reports = current.filter(r => r.kind === kind);
                return <span key={kind}>{checkNames[kind][en ? 1 : 0]}: {reports.length ? reports.some(r => r.outcome === 'fail') ? text('Perlu tindakan', 'Needs attention') : reports.every(r => r.outcome === 'pass') ? ({ installation: text('Terpasang', 'Installed'), service: text('Berjalan', 'Running'), route: text('TCP dari server berhasil', 'TCP from server succeeded'), listener: text('Terdeteksi; cakupan bind belum diverifikasi', 'Detected; bind scope unverified'), firewall: text('Bukti tersedia', 'Evidence available'), provider: text('Bukti tersedia', 'Evidence available'), dns: text('Alamat ditemukan', 'Address found'), tls: text('Sertifikat tepercaya untuk host ini', 'Trusted certificate for this host') })[kind] : text('Belum diketahui', 'Unknown') : text('Belum diperiksa', 'Not checked')}</span>;
              })}
              {!external.length && <span>{text('Belum diuji dari luar', 'Not tested externally')}</span>}
              {external.map(r => <span key={r.id}>{r.address_family?.toUpperCase()} · {r.observer_id}: {r.outcome === 'pass' ? text('Terjangkau dari pengamat ini', 'Reachable from this observer') : r.outcome === 'fail' ? text('Perlu tindakan', 'Needs attention') : text('Belum diketahui', 'Unknown')}</span>)}
            </div>
            {manage && <button type="button" className="button-secondary" disabled={disabled || !e.enabled || !onCheck} aria-busy={checkingId === e.id} aria-describedby={`diagnosis-${e.id}`} onClick={() => onCheck?.(e)}>{checkingId === e.id ? text('Memeriksa…', 'Checking…') : text('Periksa', 'Check')}<span className="sr-only"> {e.host}</span></button>}
            <p id={`diagnosis-${e.id}`} className="text-xs text-[var(--ink-muted)]">{text('Pemeriksaan dari server ini tidak membuktikan akses internet. Autentikasi, STARTTLS dan TLS native database perlu pengujian client terpisah.', 'Checks from this server do not prove internet access. Authentication, STARTTLS and native database TLS require separate client testing.')}{!manage && ` ${text('Administrator dapat menjalankan pemeriksaan.', 'An administrator can run checks.')}`}</p>
            {evidence.length > 0 && <details><summary className="cursor-pointer text-sm">{text('Bukti pemeriksaan', 'Check evidence')} ({evidence.length})</summary><ul className="mt-3 space-y-3">{evidence.map(r => {
              const current = isCurrentEvidence(e, r, now);
              const stale = r.expires_at * 1000 <= now || r.expired ? text('Kedaluwarsa', 'Expired') : r.endpoint_revision !== e.revision || !r.revision_matches ? text('Konfigurasi telah berubah', 'Configuration changed') : !e.enabled ? text('Endpoint dinonaktifkan', 'Endpoint disabled') : text('Digantikan pemeriksaan lain', 'Superseded by another check');
              const point = r.observation_point === 'local_server' ? text('Server ini', 'This server') : r.observation_point === 'user_client' ? text('Perangkat pengguna', 'User device') : text('Probe eksternal', 'External probe');
              return <li key={r.id} className="border-t border-[var(--line)] pt-3 text-xs space-y-1 break-words">
                <p className="font-medium">{checkNames[r.kind][en ? 1 : 0]}{r.address_family && ` · ${r.address_family.toUpperCase()}`} · {current ? (reasonNames[r.reason]?.[en ? 1 : 0] || r.reason) : stale}</p>
                <p>{point}: {r.observer_id} · {text('Hasil tercatat', 'Recorded result')}: {r.outcome === 'pass' ? text('Lulus', 'Pass') : r.outcome === 'fail' ? text('Gagal', 'Fail') : text('Belum diketahui', 'Unknown')}</p>
                <p>{text('Diperiksa', 'Checked')}: {new Date(r.observed_at * 1000).toLocaleString(en ? 'en-GB' : 'id-ID')} · {text('Berlaku sampai', 'Valid until')}: {new Date(r.expires_at * 1000).toLocaleString(en ? 'en-GB' : 'id-ID')}</p>
                {current && <p>{en ? englishActions[r.reason] || r.reason : r.suggested_action}</p>}
              </li>;
            })}</ul></details>}
          </article>;
        })}
      </section>;
    })}
  </div>;
}

export function ConnectivitySettings() {
  const { user } = useAuth();
  return user ? <ConnectivityWorkspace key={`${user.id}:${user.role}`} admin={user.role === 'admin'} /> : null;
}
function ConnectivityWorkspace({ admin }: { admin: boolean }) {
  const { en, text } = useCopy();
  const [data, setData] = useState<ConnectivityOverview | null>(null);
  const [loading, setLoading] = useState(true), [busy, setBusy] = useState(false);
  const [error, setError] = useState(''), [success, setSuccess] = useState('');
  const [editor, setEditor] = useState<{ endpoint: Endpoint | null; config: EndpointConfig } | null>(null);
  const [formError, setFormError] = useState(''), [conflict, setConflict] = useState(false);
  const [checkingId, setCheckingId] = useState('');
  const [removing, setRemoving] = useState<Endpoint | null>(null);
  const [now, setNow] = useState(Date.now);
  const [page, setPage] = useState(1), [serviceFilter, setServiceFilter] = useState<Service | ''>('');
  const active = useRef(false), sequence = useRef(0), pending = useRef(false);
  const createKey = useRef('');
  const load = useCallback(async () => {
    const seq = ++sequence.current;
    setLoading(true); setError('');
    try {
      const result = await fetchConnectivity(page, serviceFilter);
      if (!active.current || seq !== sequence.current) return null;
      if (page > Math.max(1, result.total_pages)) { setPage(Math.max(1, result.total_pages)); return null; }
      setData(result); setNow(Date.now()); return result;
    } catch (error) { if (active.current && seq === sequence.current) { setData(null); setError(message(error)); } return null; }
    finally { if (active.current && seq === sequence.current) setLoading(false); }
  }, [page, serviceFilter]);
  useEffect(() => { active.current = true; void load(); return () => { active.current = false; ++sequence.current; }; }, [load]);
  useEffect(() => {
    const refreshClock = () => setNow(Date.now());
    window.addEventListener('focus', refreshClock); document.addEventListener('visibilitychange', refreshClock);
    return () => { window.removeEventListener('focus', refreshClock); document.removeEventListener('visibilitychange', refreshClock); };
  }, []);
  useEffect(() => {
    const expiries = data?.endpoints.flatMap(e => e.evidence.map(r => r.expires_at * 1000)).filter(t => t > now) || [];
    if (!expiries.length) return;
    const timer = window.setTimeout(() => setNow(Date.now()), Math.max(1, Math.min(30000, expiries.reduce((a, b) => Math.min(a, b)) - Date.now())));
    return () => window.clearTimeout(timer);
  }, [data, now]);
  const manage = admin && data?.can_manage === true;
  const change = (patch: Partial<EndpointConfig>) => {
    createKey.current = newCreateKey();
    setEditor(old => old ? { ...old, config: { ...old.config, ...patch } } : old);
  };
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!editor || !manage || pending.current) return;
    pending.current = true; setBusy(true); setFormError(''); setSuccess('');
    try {
      if (editor.endpoint) await replaceEndpoint(editor.endpoint, editor.config); else await createEndpoint(editor.config, createKey.current);
      if (active.current) { setEditor(null); setSuccess(text('Alamat disimpan. Periksa jalur koneksi sebelum digunakan.', 'Address saved. Verify the connection route before use.')); await load(); }
    } catch (error) { if (active.current) { setFormError(message(error)); setConflict((error as { status?: number }).status === 409); } }
    finally { pending.current = false; if (active.current) setBusy(false); }
  };
  const remove = async () => {
    if (!removing || !manage || pending.current) return;
    pending.current = true; setBusy(true); setError(''); setSuccess('');
    try { await deleteEndpoint(removing); if (active.current) { setRemoving(null); setSuccess(text('Alamat dihapus dari daftar koneksi.', 'Address removed from the connection list.')); await load(); } }
    catch (error) { if (active.current) { setRemoving(null); setError(message(error)); } }
    finally { pending.current = false; if (active.current) setBusy(false); }
  };
  const checkEndpoint = async (endpoint: Endpoint) => {
    if (!manage || pending.current || !endpoint.enabled) return;
    pending.current = true; setBusy(true); setCheckingId(endpoint.id); setError(''); setSuccess('');
    try {
      const result = await diagnoseEndpoint(endpoint);
      if (active.current) {
        setData(old => old ? { ...old, endpoints: old.endpoints.map(row => row.endpoint.id === result.endpoint.id ? result : row) } : old);
        setNow(Date.now());
        setSuccess(text('Pemeriksaan selesai. Tinjau hasil setiap lapisan; koneksi dari luar tetap memerlukan bukti terpisah. Tunggu 60 detik sebelum memeriksa alamat yang sama.', 'Check complete. Review each layer; external access still needs separate evidence. Wait 60 seconds before checking the same address.'));
      }
    } catch (error) { if (active.current) setError(message(error)); }
    finally { pending.current = false; if (active.current) { setBusy(false); setCheckingId(''); } }
  };
  const reloadEditor = async () => {
    if (busy || !editor?.endpoint) return;
    const id = editor.endpoint.id;
    setBusy(true);
    try { const latest = await fetchEndpoint(id); if (active.current) { setEditor({ endpoint: latest, config: { ...latest } }); setConflict(false); setFormError(''); } }
    catch (error) { if (active.current) setFormError(message(error)); }
    finally { if (active.current) setBusy(false); }
  };
  const configureService = (service: Service) => {
    const scope = service === 'panel' || service === 'ssh' ? { kind: 'host' as const } : service === 'database' ? { kind: 'engine' as const, engine: 'mariadb' as const } : { kind: 'project' as const, site_id: data?.projects[0]?.id || 0 };
    change({ service, scope, host: '', port: defaultPort[service], ...presetConfig(service, 'direct'), tls_mode: service === 'panel' ? 'none' : service === 'ssh' ? 'ssh' : service === 'submission' || service === 'smtp_inbound' ? 'starttls' : 'tls' });
  };
  const cfg = editor?.config;
  const fieldsId = 'connectivity-editor';
  return <div className="space-y-5" data-testid="connectivity-settings">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div className="space-y-1"><h2 className="text-base font-semibold">{text('Alamat per layanan', 'Service addresses')}</h2><p className="text-sm text-[var(--ink-muted)]">{text('Lihat alamat koneksi, jalur, dan bukti pemeriksaan setiap layanan.', 'Review the connection address, route and check evidence for each service.')}</p></div>
      <div className="flex flex-wrap gap-2"><button type="button" className="button-secondary" disabled={loading || busy} onClick={() => void load()}><RefreshCw size={15} aria-hidden="true" />{text('Muat ulang', 'Reload')}</button>{manage && <button type="button" className="button-primary" disabled={loading || busy} onClick={() => { createKey.current = newCreateKey(); setEditor({ endpoint: null, config: initial() }); setFormError(''); setConflict(false); }}><Plus size={15} aria-hidden="true" />{text('Tambah alamat', 'Add address')}</button>}</div>
    </div>
    {manage && data && <PanelUrlSettings endpoints={data.endpoints.map(row => row.endpoint)} disabled={busy || loading} onReloadEndpoints={() => void load()} />}
    {manage && data && <PanelTlsSettings endpoints={data.endpoints.map(row => row.endpoint)} disabled={busy || loading} />}
    {admin && <p className="text-sm text-[var(--ink-muted)]">{text('Menyimpan alamat tidak mengubah alamat panel saat ini, menerbitkan rute, atau membuka port.', 'Saving an address does not change the current panel address, publish routes, or open ports.')} <RouteLink href="/settings?section=network" className="underline">{text('Kelola DNS dan Cloudflare', 'Manage DNS and Cloudflare')}</RouteLink></p>}
    {!admin && <p className="text-sm text-[var(--ink-muted)]">{text('Alamat hanya ditampilkan untuk project Anda. Hubungi administrator untuk perubahan.', 'Only your project addresses are shown. Contact your administrator for changes.')}</p>}
    <p className="text-sm"><RouteLink href="/terminal" className="underline">{text('Buka terminal browser', 'Open browser terminal')}</RouteLink> · <RouteLink href="/files" className="underline">{text('Buka File Manager', 'Open File Manager')}</RouteLink></p>
    {success && <p role="status" className="text-sm">{success}</p>}
    <div className="max-w-xs space-y-1"><label htmlFor="connectivity-service-filter" className="text-sm">{text('Filter layanan', 'Service filter')}</label><select id="connectivity-service-filter" className={input} value={serviceFilter} disabled={busy} onChange={e => { setPage(1); setServiceFilter(e.target.value as Service | ''); }}><option value="">{text('Semua layanan', 'All services')}</option>{services.filter(s => admin || (data?.service_counts[s] || 0) > 0 || ['website', 'ssh', 'imap', 'submission', 'smtp_inbound'].includes(s)).map(s => <option key={s} value={s}>{serviceName(s, en)}</option>)}</select></div>
    {error && <div role="alert" className="space-y-2 text-sm text-[var(--danger)]"><p>{error}</p><button type="button" className="button-secondary" disabled={loading || busy} onClick={() => void load()}>{text('Coba lagi', 'Retry')}</button></div>}
    {loading ? <p role="status" className="text-sm text-[var(--ink-muted)]">{text('Memuat konektivitas…', 'Loading connectivity…')}</p> : data && <>
      {!data.total && <p className="border-y border-[var(--line)] py-4 text-sm">{serviceFilter ? text('Belum ada alamat untuk layanan ini.', 'No addresses configured for this service.') : text('Belum ada alamat koneksi yang diatur.', 'No connection addresses have been configured.')}{manage ? ` ${text('Tambahkan alamat untuk layanan yang ingin digunakan.', 'Add an address for a service you want to use.')}` : ` ${text('Terminal dan File Manager tetap dapat dibuka melalui panel sesuai izin akun.', 'Terminal and File Manager remain available through the panel according to your account permissions.')}`}</p>}
      <ConnectivitySummary data={data} filter={serviceFilter} manage={manage} now={now} disabled={busy || loading} checkingId={checkingId} onCheck={e => void checkEndpoint(e)} onEdit={e => { setEditor({ endpoint: e, config: { ...e } }); setFormError(''); setConflict(false); }} onDelete={setRemoving} />
      {data.total_pages > 1 && <nav aria-label={text('Halaman alamat koneksi', 'Connection address pages')} className="flex flex-wrap items-center justify-end gap-3 text-sm"><span>{data.total} {text('alamat', 'addresses')} · {text('Halaman', 'Page')} {data.page} {text('dari', 'of')} {data.total_pages}</span><button type="button" className="button-secondary" disabled={busy || loading || page <= 1} onClick={() => setPage(p => p - 1)}>{text('Sebelumnya', 'Previous')}</button><button type="button" className="button-secondary" disabled={busy || loading || page >= data.total_pages} onClick={() => setPage(p => p + 1)}>{text('Berikutnya', 'Next')}</button></nav>}
    </>}
    {editor && cfg && <ModalOverlay className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4" role="dialog" aria-modal="true" aria-labelledby={`${fieldsId}-title`} onDismiss={() => setEditor(null)} dismissDisabled={busy}>
      <div className="dialog-panel max-w-2xl border border-[var(--line)] rounded-[var(--radius-lg)] bg-[var(--surface)] p-5">
        <form onSubmit={event => void submit(event)} className="dialog-form connectivity-editor">
          <div className="dialog-header flex justify-between pb-4"><h2 id={`${fieldsId}-title`} className="font-semibold">{editor.endpoint ? text('Ubah alamat koneksi', 'Edit connection address') : text('Tambah alamat koneksi', 'Add connection address')}</h2><button type="button" disabled={busy} onClick={() => setEditor(null)} className="p-1" aria-label={text('Tutup', 'Close')}><X size={20} /></button></div>
          <div className="dialog-body space-y-4 text-sm">
            <p className="text-[var(--ink-muted)]">{text('Pilihan awal hanya mengisi formulir ini. Alamat disimpan setelah Anda memilih Simpan alamat.', 'Presets only fill this form. The address is saved when you choose Save address.')}</p>
            <fieldset disabled={busy} className="connectivity-fields">
              <ConnectivityField label={text('Layanan', 'Service')}><select className={input} value={cfg.service} disabled={Boolean(editor.endpoint)} onChange={e => configureService(e.target.value as Service)}>{services.map(s => <option key={s} value={s}>{serviceName(s, en)}</option>)}</select></ConnectivityField>
              <ConnectivityField label={text('Profil awal', 'Preset')}><select className={input} defaultValue="" key={`${cfg.service}-${editor.endpoint?.id || 'new'}`} onChange={e => { if (e.target.value) change(presetConfig(cfg.service, e.target.value as Preset)); }}><option value="">{text('Pilih untuk mengisi jalur', 'Choose to fill the route')}</option><option value="direct">{text('IP publik langsung', 'Direct public IP')}</option><option value="tunnel">{text('Tunnel / NAT', 'Tunnel / NAT')}</option><option value="mixed">{text('Gabungan', 'Mixed')}</option></select></ConnectivityField>
              <ConnectivityField label={text('Lingkup alamat', 'Address scope')}><select className={input} disabled={Boolean(editor.endpoint)} value={cfg.scope.kind} onChange={e => change({ certificate_id: null, scope: e.target.value === 'host' ? { kind: 'host' } : { kind: 'project', site_id: data?.projects[0]?.id || 0 } })}>{['panel', 'ssh'].includes(cfg.service) && <option value="host">{text('Server', 'Server')}</option>}{cfg.service !== 'panel' && cfg.service !== 'database' && <option value="project">{text('Project', 'Project')}</option>}{cfg.service === 'database' && <option value="engine">{text('Mesin database', 'Database engine')}</option>}</select></ConnectivityField>
              {cfg.scope.kind === 'project' && <ConnectivityField label={text('Project', 'Project')}><select required className={input} disabled={Boolean(editor.endpoint)} value={cfg.scope.site_id || ''} onChange={e => change({ scope: { kind: 'project', site_id: Number(e.target.value) }, certificate_id: null })}><option value="" disabled>{text('Pilih project', 'Choose a project')}</option>{data?.projects.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}</select></ConnectivityField>}
              {cfg.scope.kind === 'engine' && <ConnectivityField label={text('Mesin database', 'Database engine')}><select className={input} disabled={Boolean(editor.endpoint)} value={cfg.scope.engine} onChange={e => change({ scope: { kind: 'engine', engine: e.target.value as 'mariadb' | 'postgresql' }, port: e.target.value === 'mariadb' ? 3306 : 5432 })}><option value="mariadb">MariaDB</option><option value="postgresql">PostgreSQL</option></select></ConnectivityField>}
              <ConnectivityField label={text('Jalur koneksi', 'Connection route')}><select className={input} value={cfg.transport} onChange={e => change({ transport: e.target.value as EndpointConfig['transport'], certificate_id: null, tls_mode: cfg.service === 'ssh' ? 'ssh' : e.target.value === 'cloudflare_http' ? 'tls' : cfg.tls_mode })}>{availableTransports(cfg.service).map(t => <option key={t} value={t}>{transportName(t, en)}</option>)}</select></ConnectivityField>
              <ConnectivityField label={text('Hostname atau IP', 'Hostname or IP')}><input required className={input} value={cfg.host} autoComplete="off" placeholder="panel.example.com" onChange={e => change({ host: e.target.value })} /><span className="text-xs text-[var(--ink-muted)]">{text('Tanpa URL atau port. IPv6 tanpa bracket.', 'No URL or port. IPv6 without brackets.')}</span></ConnectivityField>
              <ConnectivityField label={text('Port', 'Port')}><input required type="number" min="1" max="65535" step="1" className={input} value={cfg.port || ''} onChange={e => change({ port: Number(e.target.value) })} /></ConnectivityField>
              <ConnectivityField label={text('Keamanan koneksi', 'Connection security')}><select className={input} value={cfg.tls_mode} disabled={cfg.service === 'ssh' || cfg.transport === 'cloudflare_http'} onChange={e => change({ tls_mode: e.target.value as EndpointConfig['tls_mode'], certificate_id: e.target.value === 'none' ? null : cfg.certificate_id })}>{cfg.service === 'ssh' ? <option value="ssh">SSH</option> : <><option value="none">{text('Tanpa TLS', 'No TLS')}</option><option value="tls">TLS</option>{!['panel', 'website'].includes(cfg.service) && <option value="starttls">STARTTLS</option>}</>}</select></ConnectivityField>
              {cfg.scope.kind === 'project' && ['tls', 'starttls'].includes(cfg.tls_mode) && !['cloudflare_http', 'provider'].includes(cfg.transport) && <ConnectivityField label={text('Sertifikat lokal', 'Local certificate')}><select className={input} value={cfg.certificate_id || ''} onChange={e => change({ certificate_id: e.target.value ? Number(e.target.value) : null })}><option value="">{text('Belum ditautkan', 'Not linked')}</option>{data?.certificates.filter(c => cfg.scope.kind === 'project' && c.site_id === cfg.scope.site_id).map(c => <option key={c.id} value={c.id}>{c.name} (#{c.id})</option>)}</select></ConnectivityField>}
              <label className="connectivity-check"><input type="checkbox" checked={cfg.enabled} onChange={e => change({ enabled: e.target.checked, primary: e.target.checked ? cfg.primary : false })} />{text('Aktifkan alamat dalam daftar', 'Enable address in the list')}</label>
              <label className="connectivity-check"><input type="checkbox" checked={cfg.primary} disabled={!cfg.enabled} onChange={e => change({ primary: e.target.checked })} />{text('Alamat utama untuk layanan dan lingkup ini', 'Primary address for this service and scope')}</label>
            </fieldset>
            {cfg.scope.kind === 'project' && !data?.projects.length && <p role="status">{text('Belum ada project aktif. Buat project sebelum menambahkan alamat layanan ini.', 'No active project. Create a project before adding this service address.')}</p>}
            {formError && <div role="alert" className="text-[var(--danger)] space-y-2"><p>{formError}</p>{conflict && editor.endpoint && <button type="button" className="button-secondary" disabled={busy || loading} onClick={() => void reloadEditor()}>{text('Muat versi terbaru dan ganti isian', 'Load latest version and replace fields')}</button>}</div>}
          </div>
          <div className="dialog-footer flex flex-wrap justify-end gap-2 pt-4"><button type="button" className="button-secondary" disabled={busy} onClick={() => setEditor(null)}>{text('Batal', 'Cancel')}</button><button type="submit" className="button-primary" disabled={busy || loading || !manage || (cfg.scope.kind === 'project' && !cfg.scope.site_id)}>{busy ? text('Menyimpan…', 'Saving…') : text('Simpan alamat', 'Save address')}</button></div>
        </form>
      </div>
    </ModalOverlay>}
    <ConfirmDangerDialog isOpen={Boolean(removing)} onClose={() => setRemoving(null)} onConfirm={() => void remove()} isLoading={busy} title={text('Hapus alamat koneksi', 'Delete connection address')} description={text('Alamat dan riwayat pemeriksaannya akan dihapus dari daftar. Layanan, rute provider, dan port tidak diubah.', 'The address and its check history will be removed from the list. Services, provider routes and ports are not changed.')} confirmMatchText={removing?.host || ''} confirmButtonLabel={text('Hapus alamat', 'Delete address')} />
  </div>;
}
