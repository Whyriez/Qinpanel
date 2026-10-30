import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { Database, KeyRound, Plus, RefreshCw, X } from 'lucide-react';
import { ModalOverlay } from '../ui/ModalOverlay';
import { cloudflareCommand, fetchCloudflareStatus, type CloudflarePage, type CloudflareStatus, type Zone } from '../../api/network';
import { allocateManagedBucket, configureManagedStorage, fetchManagedStorage, fetchStorageTls, generateStorageTls, issueStorageKey, storageAllocationId, type ManagedStorageStatus, type StorageConnection, type StorageContext, type StorageKeys, type StorageTlsInventory } from '../../api/storage';

const errorMessage = (error: unknown) => error instanceof Error ? error.message : 'Storage request failed';
const capacity = (value: number | null) => value === null ? '—' : `${(value / 1073741824).toLocaleString(undefined, { maximumFractionDigits: 1 })} GiB`;
type Localized = { english: boolean };
const language = (english: boolean) => (id: string, en: string) => english ? en : id;

function Dialog({ english, title, description, busy, error, children, footer, onClose }: { english: boolean; title: string; description: string; busy: boolean; error: string; children: ReactNode; footer: ReactNode; onClose: () => void }) {
  const id = useId(); const alert = useRef<HTMLParagraphElement>(null);
  useEffect(() => { if (error) alert.current?.focus(); }, [error]);
  return <ModalOverlay className="storage-modal" aria-labelledby={`${id}-title`} aria-describedby={`${id}-description`} onDismiss={onClose} dismissDisabled={busy}>
    <section className="dialog-panel storage-dialog"><header className="dialog-header storage-dialog-header"><div><h2 id={`${id}-title`}>{title}</h2><p id={`${id}-description`}>{description}</p></div><button type="button" aria-label={english ? 'Close' : 'Tutup'} disabled={busy} onClick={onClose}><X size={18} /></button></header>
      <div className="dialog-body storage-dialog-body">{error && <p className="storage-error" role="alert" tabIndex={-1} ref={alert}>{error}</p>}{children}</div><footer className="dialog-footer storage-dialog-footer">{footer}</footer>
    </section>
  </ModalOverlay>;
}

type BucketContext = { context: StorageContext; contextLoading?: boolean; contextError?: string; onRefreshContext?: () => void };

export function ManagedStorageActions({ english, status, loading, disabled, modalOpen, onConfigure, onBucket, onManage, onRefresh }: Localized & { status: ManagedStorageStatus | null; loading: boolean; disabled: boolean; modalOpen: boolean; onConfigure: () => void; onBucket: () => void; onManage?: () => void; onRefresh: () => void }) {
  const t = language(english); const locked = disabled || loading || modalOpen;
  return <div className="storage-actions">
    {status?.running && onManage && <button className={status.bucket_count > 0 ? 'button-primary' : undefined} type="button" disabled={locked} onClick={onManage}>{t('Kelola alokasi bucket', 'Manage bucket allocations')}</button>}
    <button className={!status?.running ? 'button-primary' : undefined} type="button" disabled={locked || !status} onClick={onConfigure}>{status?.configured ? t('Koneksi & HTTPS', 'Connection & HTTPS') : t('Aktifkan storage Qinpanel', 'Activate Qinpanel storage')}</button>
    <button className={status?.running && status.bucket_count === 0 ? 'button-primary' : undefined} type="button" disabled={locked || !status?.running} onClick={onBucket}><Plus size={16} />{t('Buat bucket Qinpanel', 'Create Qinpanel bucket')}</button>
    <button type="button" aria-label={t('Periksa layanan storage', 'Check storage service')} disabled={locked} onClick={onRefresh}><RefreshCw size={16} /></button>
  </div>;
}

export function BucketOwnerStatus({ english, context, contextLoading = false, contextError = '', onRefreshContext, ownerId }: Localized & BucketContext & { ownerId?: number }) {
  const t = language(english);
  if (contextLoading) return <p role="status" className="storage-note">{t('Memuat tenant pemilik bucket…', 'Loading bucket owners…')}</p>;
  if (!contextError && (ownerId ? context.owners.some(owner => owner.id === ownerId) : context.owners.length > 0)) return null;
  return <div className="storage-notice" role={contextError ? 'alert' : 'status'}>
    <p>{contextError ? t('Daftar tenant pemilik belum dapat dimuat. Bucket yang sudah ada tetap dapat dikelola.', 'Bucket owners could not be loaded. Existing buckets can still be managed.') : t('Siapkan tenant aktif sebagai pemilik bucket. Setelah tenant dibuat atau diaktifkan, kembali ke halaman ini untuk melanjutkan pembuatan bucket.', 'Set up an active tenant to own the bucket. After creating or activating the tenant, return here to continue creating your bucket.')}</p>
    {contextError && <p className="storage-error">{contextError}</p>}
    <div className="storage-actions">{!contextError && <a href="/tenants" target="_blank" rel="noopener noreferrer">{t('Kelola tenant (tab baru)', 'Manage tenants (new tab)')}</a>}{onRefreshContext && <button type="button" onClick={onRefreshContext}><RefreshCw size={16} />{t('Muat ulang tenant', 'Refresh tenants')}</button>}</div>
  </div>;
}

export function ManagedStorage({ english, context, contextLoading = false, contextError = '', onRefreshContext, ownerId, siteId, refreshKey = 0, disabled = false, onChanged, onCreated, onManage }: Localized & BucketContext & { onChanged?: () => void; refreshKey?: number; disabled?: boolean; ownerId?: number; siteId?: number; onCreated: (connection: StorageConnection) => void; onManage?: () => void }) {
  const t = language(english);
  const [status, setStatus] = useState<ManagedStorageStatus | null>(null); const [error, setError] = useState(''); const [loading, setLoading] = useState(true); const [refresh, setRefresh] = useState(0);
  const [dialog, setDialog] = useState<'configure' | 'bucket' | null>(null);
  useEffect(() => { const controller = new AbortController(); setLoading(true); setError(''); fetchManagedStorage(controller.signal).then(value => { if (!controller.signal.aborted) setStatus(value); }).catch(error => { if (!controller.signal.aborted) { setStatus(null); setError(errorMessage(error)); } }).finally(() => { if (!controller.signal.aborted) setLoading(false); }); return () => controller.abort(); }, [refresh, refreshKey]);
  return <section className="storage-managed" aria-label={t('Storage Qinpanel', 'Qinpanel storage')}>
    <div className="storage-managed-heading"><div><h2><Database size={20} /> {t('Storage Qinpanel', 'Qinpanel storage')}</h2><p>{t('Sediakan bucket dan API S3 dari server Anda sendiri.', 'Provide buckets and an S3 API from your own server.')}</p></div><span className="storage-managed-status">{loading ? t('Memeriksa…', 'Checking…') : !status ? t('Belum diketahui', 'Unknown') : status.running ? t('Aktif', 'Running') : status.configured ? t('Perlu diperiksa', 'Needs attention') : t('Belum diaktifkan', 'Not activated')}</span></div>
    {error && <p role="alert" className="storage-error">{error}</p>}
    {status?.configured && <dl className="storage-detail"><div><dt>Endpoint S3</dt><dd>{status.endpoint}</dd></div><div><dt>{t('Ruang tersedia / volume', 'Available / volume')}</dt><dd>{capacity(status.available_bytes)} / {capacity(status.capacity_bytes)}</dd></div><div><dt>{t('Alokasi bucket', 'Bucket allocations')}</dt><dd>{status.bucket_count}</dd></div></dl>}
    <p className="storage-note">{t('Storage lokal memakai disk server bersama; batas keras per bucket belum diterapkan. Atur alokasi transfer panel per tenant di bawah. S3 eksternal dapat dihubungkan dan digunakan bersamaan melalui Hubungkan S3.', 'Local storage uses the shared server disk; hard per-bucket limits are not enforced. Set tenant panel-transfer allowances below. Connect and use external S3 alongside local storage with Connect S3.')}</p>
    <p className="storage-note">{t('Data dan konfigurasi engine belum termasuk backup server panel. Siapkan backup storage terpisah sebelum menyimpan data penting.', 'Engine data and configuration are not included in panel server backups. Set up a separate storage backup before storing important data.')}</p>
    {status?.origin_ready === false && <p role="status" className="storage-notice">{t('Origin storage belum siap. Buka Koneksi & HTTPS untuk memeriksa hasil aktivasi.', 'The storage origin is not ready. Open Connection & HTTPS to review activation.')}</p>}
    {status?.connection_mode === 'tunnel' && <p className="storage-note">{t('Akses menggunakan Cloudflare Tunnel. Status layanan berasal dari pemeriksaan lokal; akses domain publik belum diuji dari luar server.', 'Access uses Cloudflare Tunnel. Service status reflects local checks; public domain access has not been tested from outside the server.')}</p>}
    {status?.running && <><p className="storage-note">{status.bucket_count === 0 ? t('Storage sudah aktif. Buat bucket pertama untuk mengunggah, mengunduh, dan mengelola file melalui panel.', 'Storage is running. Create your first bucket to upload, download and manage files in the panel.') : t('Kelola bucket, file, dan access key aplikasi melalui manajemen bucket di bawah.', 'Manage buckets, files and application access keys in bucket management below.')}</p><BucketOwnerStatus english={english} context={context} contextLoading={contextLoading} contextError={contextError} onRefreshContext={onRefreshContext} ownerId={ownerId} /></>}
    <ManagedStorageActions english={english} status={status} loading={loading} disabled={disabled} modalOpen={dialog !== null} onConfigure={() => setDialog('configure')} onBucket={() => setDialog('bucket')} onManage={onManage} onRefresh={() => setRefresh(value => value + 1)} />
    {dialog === 'configure' && <ConfigureDialog english={english} status={status} onClose={() => setDialog(null)} onSaved={value => { setStatus(value); setDialog(null); }} />}
    {dialog === 'bucket' && <BucketDialog english={english} context={context} contextLoading={contextLoading} contextError={contextError} onRefreshContext={onRefreshContext} ownerId={ownerId} siteId={siteId} onClose={() => { setDialog(null); setRefresh(value => value + 1); onChanged?.(); }} onSaved={value => { setDialog(null); setRefresh(value => value + 1); onCreated(value); }} />}
  </section>;
}

export function ConfigureDialog({ english, status, onClose, onSaved }: Localized & { status: ManagedStorageStatus | null; onClose: () => void; onSaved: (status: ManagedStorageStatus) => void }) {
  const t = language(english); const formId = useId();
  const [host, setHost] = useState(() => status?.endpoint ? new URL(status.endpoint).hostname : '');
  const [method, setMethod] = useState<'tunnel' | 'http01' | 'auto' | 'manual'>(status?.configured && status.connection_mode !== 'tunnel' ? 'auto' : 'tunnel');
  const [cloudflare, setCloudflare] = useState<CloudflareStatus | null>(null); const [zones, setZones] = useState<Zone[]>([]);
  const [tunnelId, setTunnelId] = useState(''); const [connectionError, setConnectionError] = useState(''); const [connectionLoading, setConnectionLoading] = useState(true);
  const initialized = useRef(false);
  const [email, setEmail] = useState(''); const [confirmed, setConfirmed] = useState(false);
  const [chain, setChain] = useState(''); const [key, setKey] = useState('');
  const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  const [inventory, setInventory] = useState<StorageTlsInventory | null>(null); const [reload, setReload] = useState(0);
  const [checking, setChecking] = useState(true); const [statusError, setStatusError] = useState('');
  const statusFailed = !!statusError;
  const requestId = useRef<string | null>(null); const saved = useRef(onSaved); saved.current = onSaved;
  const observedPending = useRef(false);
  const pending = !!inventory && ['queued', 'issuing', 'configuring'].includes(inventory.certificate.state);
  useEffect(() => {
    const controller = new AbortController(); let timer: ReturnType<typeof setTimeout> | undefined;
    const load = async () => {
      setChecking(true);
      try {
        const value = await fetchStorageTls(controller.signal);
        if (controller.signal.aborted) return;
        setInventory(value);
        if (!initialized.current) {
          initialized.current = true;
          if (value.certificate.hostname) setMethod(value.certificate.state === 'manual' ? 'manual' : value.certificate.method === 'tunnel' ? 'tunnel' : value.certificate.method === 'http01' ? 'http01' : 'auto');
          if (value.certificate.tunnel_id) setTunnelId(value.certificate.tunnel_id);
        }
        setStatusError('');
        setHost(current => current || value.certificate.hostname || '');
        setEmail(current => current || value.certificate.email || '');
        if (['queued', 'issuing', 'configuring'].includes(value.certificate.state)) observedPending.current = true;
        if (value.certificate.state === 'active' && (observedPending.current || value.certificate.request_id === requestId.current && requestId.current)) {
          const service = await fetchManagedStorage(controller.signal);
          if (!controller.signal.aborted && service.configured && service.running) saved.current(service);
        } else if (['queued', 'issuing', 'configuring'].includes(value.certificate.state)) timer = setTimeout(() => { void load(); }, 3000);
      } catch (error) { if (!controller.signal.aborted) setStatusError(errorMessage(error)); }
      finally { if (!controller.signal.aborted) setChecking(false); }
    };
    void load();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [reload]);
  useEffect(() => {
    if (method !== 'tunnel') return;
    const controller = new AbortController(); setConnectionLoading(true); setConnectionError('');
    const load = async () => {
      try {
        const value = await fetchCloudflareStatus(controller.signal);
        if (controller.signal.aborted) return;
        setCloudflare(value);
        if (!value.configured || !value.connector_installed) return;
        const items: Zone[] = [];
        for (let page = 1; page <= 20; page++) {
          const result = await cloudflareCommand<CloudflarePage<Zone>>({ action: 'zones', page });
          if (controller.signal.aborted) return;
          items.push(...result.items);
          if (page >= result.pagination.total_pages) break;
          if (page === 20) throw new Error(t('Daftar zona terlalu besar. Persempit cakupan credential Cloudflare ke zona storage.', 'Too many zones. Scope the Cloudflare credential to the storage zone.'));
        }
        setZones(items);
        const active = value.managed_tunnels.filter(item => item.local_status === 'active');
        if (active.length === 1) setTunnelId(current => current || active[0].id);
      } catch (error) { if (!controller.signal.aborted) setConnectionError(errorMessage(error)); }
      finally { if (!controller.signal.aborted) setConnectionLoading(false); }
    };
    void load(); return () => controller.abort();
  }, [method, reload, english]);
  const zone = zones.filter(item => host.trim() === item.name || host.trim().endsWith(`.${item.name}`)).sort((a, b) => b.name.length - a.name.length)[0];
  const tunnelReady = !connectionLoading && !connectionError && cloudflare?.configured && cloudflare.connector_installed && !!zone && cloudflare.managed_tunnels.some(item => item.id === tunnelId && item.local_status === 'active');
  useEffect(() => {
    const refresh = () => { if (!busy && !pending) setReload(value => value + 1); };
    window.addEventListener('focus', refresh);
    return () => window.removeEventListener('focus', refresh);
  }, [busy, pending]);
  const save = async (event: FormEvent) => {
    event.preventDefault(); if (busy || pending) return; setBusy(true); setError('');
    try {
      if (method !== 'manual') {
        if (!inventory || checking || statusFailed) return;
        if (method === 'tunnel' ? !tunnelReady : !confirmed || method === 'auto' && (inventory.dns.available === false || !inventory.dns.configured)) return;
        if (inventory.certificate.state === 'failed' && inventory.certificate.request_id === requestId.current) requestId.current = null;
        requestId.current ??= storageAllocationId();
        const certificate = await generateStorageTls({ request_id: requestId.current, expected_revision: inventory.certificate.revision, hostname: host.trim(), email: method === 'tunnel' ? '' : email.trim(), confirmed: method === 'tunnel' ? false : confirmed, method: method === 'auto' ? 'dns01' : method, ...(method === 'tunnel' ? { zone_id: zone?.id, tunnel_id: tunnelId } : {}) });
        observedPending.current = ['queued', 'issuing'].includes(certificate.state);
        setInventory({ ...inventory, certificate }); setReload(value => value + 1);
      } else {
        const value = await configureManagedStorage({ hostname: host.trim(), fullchain_pem: chain, private_key_pem: key });
        setChain(''); setKey(''); onSaved(value);
      }
    } catch (error) { setError(errorMessage(error)); setReload(value => value + 1); }
    finally { setBusy(false); }
  };
  const resetRequest = () => { requestId.current = null; setConfirmed(false); };
  const automatic = method !== 'manual';
  const activationBlocked = automatic && (checking || statusFailed || !inventory || (method === 'tunnel' ? !tunnelReady : !confirmed || method === 'auto' && (inventory.dns.available === false || !inventory.dns.configured)));
  return <Dialog english={english} title={t('Aktifkan storage Qinpanel', 'Activate Qinpanel storage')} description={t('Layanan S3 tersendiri di server ini, dengan endpoint HTTPS untuk aplikasi Anda.', 'A separate S3 service on this server, with an HTTPS endpoint for your applications.')} busy={busy} error={error || statusError} onClose={onClose} footer={<>
    <button type="button" disabled={busy} onClick={onClose}>{pending ? t('Tutup', 'Close') : t('Batal', 'Cancel')}</button>
    <button form={formId} type="submit" className="button-primary" disabled={busy || pending || activationBlocked}>{busy ? t('Memproses…', 'Processing…') : pending ? t('Aktivasi berjalan…', 'Activation in progress…') : method === 'tunnel' ? t('Aktifkan melalui Tunnel', 'Activate through Tunnel') : automatic ? t('Generate & aktifkan', 'Generate & activate') : status?.configured ? t('Perbarui sertifikat', 'Update certificate') : t('Aktifkan layanan', 'Activate service')}</button>
  </>}>
    <form id={formId} onSubmit={save} className="storage-form-section" aria-busy={busy}>
      <p className="storage-note">{t('Gunakan domain terpisah dari panel dan website. Domain dapat dihubungkan ke IP publik server atau melalui Cloudflare Tunnel.', 'Use a domain separate from the panel and websites. Connect it to the server public IP or through Cloudflare Tunnel.')}</p>
      <label>{t('Domain storage', 'Storage domain')}<input autoFocus required pattern="[a-z0-9.-]+" placeholder="storage.example.com" value={host} readOnly={!!status?.configured} disabled={busy || pending} onChange={event => { setHost(event.target.value); resetRequest(); }} /></label>
      <label>{t('Jalur akses storage', 'Storage access route')}<select aria-label={t('Jalur akses storage', 'Storage access route')} value={method} disabled={busy || pending} onChange={event => { initialized.current = true; setMethod(event.target.value as typeof method); setChain(''); setKey(''); resetRequest(); }}>
        <option value="tunnel">Cloudflare Tunnel</option>
        <option value="http01">{t('IP publik · HTTPS otomatis', 'Public IP · automatic HTTPS')}</option>
        <option value="auto">{t('Lanjutan · sertifikat DNS-01', 'Advanced · DNS-01 certificate')}</option>
        <option value="manual">{t('Gunakan sertifikat sendiri (PEM)', 'Use your own certificate (PEM)')}</option>
      </select></label>
      {method === 'tunnel' ? <>
        <p className="storage-note">{t('HTTPS publik ditangani Cloudflare. Tunnel meneruskan ke layanan storage lokal; tidak perlu token DNS-01 atau sertifikat PEM.', 'Cloudflare handles public HTTPS. Tunnel forwards to the local storage service; no DNS-01 token or PEM certificate is required.')}</p>
        {connectionLoading ? <p role="status">{t('Memeriksa koneksi Tunnel…', 'Checking Tunnel connection…')}</p> : connectionError ? <p role="alert" className="storage-error">{connectionError}</p> : !cloudflare?.configured || !cloudflare.connector_installed || !cloudflare.managed_tunnels.some(item => item.local_status === 'active') ? <p className="storage-notice">{t('Hubungkan Cloudflare dan jalankan konektor Tunnel pada server ini.', 'Connect Cloudflare and start the Tunnel connector on this server.')} <a href="/settings?section=network&network_tab=tunnels" target="_blank" rel="noopener noreferrer">{t('Buka pengaturan Cloudflare', 'Open Cloudflare settings')}</a></p> : <>
          <label>Tunnel<select required aria-label="Tunnel" value={tunnelId} disabled={busy || pending} onChange={event => { setTunnelId(event.target.value); resetRequest(); }}><option value="">{t('Pilih Tunnel aktif', 'Choose an active Tunnel')}</option>{cloudflare.managed_tunnels.filter(item => item.local_status === 'active').map(item => <option key={item.id} value={item.id}>{item.id}</option>)}</select></label>
          {host && !zone && <p role="status" className="storage-notice">{t('Domain storage harus berada dalam zona Cloudflare yang terhubung.', 'The storage domain must belong to a connected Cloudflare zone.')}</p>}
          {zone && <p className="storage-note">{t('Zona:', 'Zone:')} {zone.name}</p>}
        </>}
        <button type="button" disabled={busy || checking || connectionLoading} onClick={() => setReload(value => value + 1)}>{t('Muat ulang koneksi', 'Refresh connection')}</button>
      </> : automatic ? <>
        {method === 'auto' ? <>
        <p className="storage-note">{t('DNS-01 hanya diperlukan untuk metode Generate otomatis ini, bukan untuk memakai S3. Pilih Gunakan sertifikat sendiri bila sudah memiliki sertifikat HTTPS. Validasi DNS-01 Cloudflare bekerja untuk IP publik maupun Tunnel tanpa membuka port 80; panel membuat dan memasang private key.', 'DNS-01 is required only for this automatic generation method, not for using S3. Choose your own certificate if you already have an HTTPS certificate. Cloudflare DNS-01 works with public IPs and Tunnels without opening port 80; the panel generates and installs the private key.')}</p>
        <StorageDnsStatus english={english} dns={inventory?.dns} checking={checking} failed={statusFailed} />
        </> : <p className="storage-note">{t('Arahkan DNS domain storage ke IP publik server. Panel menerbitkan dan memperpanjang HTTPS melalui HTTP-01; port 80 dan 443 harus dapat dijangkau. Tidak memerlukan token DNS-01.', 'Point the storage domain DNS to the server public IP. The panel issues and renews HTTPS using HTTP-01; ports 80 and 443 must be reachable. No DNS-01 token is required.')}</p>}
        <label>{t('Email sertifikat', 'Certificate email')}<input type="email" required maxLength={254} autoComplete="email" value={email} disabled={busy || pending} onChange={event => { setEmail(event.target.value); resetRequest(); }} /></label>
        <label className="storage-tls-consent"><input type="checkbox" required checked={confirmed} disabled={busy || pending} onChange={event => setConfirmed(event.target.checked)} /><span>{t("Saya menyetujui ketentuan Let's Encrypt, penerbitan sertifikat untuk domain ini, dan perpanjangan otomatis.", "I agree to the Let's Encrypt terms, certificate issuance for this domain, and automatic renewal.")} <a href="https://letsencrypt.org/repository/" target="_blank" rel="noreferrer">{t('Baca ketentuan', 'Read terms')}</a></span></label>
        {inventory?.certificate.not_after && <p className="storage-note">{t('Sertifikat aktif berakhir:', 'Active certificate expires:')} {new Date(inventory.certificate.not_after * 1000).toLocaleDateString()}</p>}
        <button type="button" disabled={busy || checking} onClick={() => { setError(''); setReload(value => value + 1); }}>{t('Muat ulang status', 'Refresh status')}</button>
      </> : <>
        <label>{t('Sertifikat lengkap (PEM)', 'Full certificate chain (PEM)')}<textarea required rows={4} maxLength={65536} value={chain} spellCheck={false} autoComplete="off" disabled={busy || pending} onChange={event => setChain(event.target.value)} placeholder="-----BEGIN CERTIFICATE-----" /></label>
        <label>{t('Private key (PEM)', 'Private key (PEM)')}<textarea required rows={4} maxLength={65536} value={key} spellCheck={false} autoComplete="off" disabled={busy || pending} onChange={event => setKey(event.target.value)} placeholder="-----BEGIN PRIVATE KEY-----" /></label>
        <p className="storage-note">{t('Pemasangan sertifikat sendiri menonaktifkan perpanjangan otomatis storage. Perbarui PEM sebelum sertifikat berakhir.', 'Installing your own certificate disables automatic storage renewal. Update the PEM files before the certificate expires.')}</p>
      </>}
      {inventory?.certificate.error && <p role="alert" className="storage-error">{inventory.certificate.error}</p>}
      {pending && <p role="status" className="storage-notice">{t('Aktivasi berjalan di latar belakang. Anda boleh menutup dialog dan kembali untuk memeriksa hasilnya.', 'Activation runs in the background. You can close this dialog and return to check the result.')}</p>}
    </form>
  </Dialog>;
}

export function StorageDnsStatus({ english, dns, checking, failed }: Localized & { dns?: StorageTlsInventory['dns']; checking: boolean; failed: boolean }) {
  const t = language(english);
  if (checking) return <p role="status" className="storage-note">{t('Memeriksa token DNS-01 Cloudflare…', 'Checking the Cloudflare DNS-01 token…')}</p>;
  if (failed || !dns || dns.available === false) return <p role="status" className="storage-notice">{t('Status token DNS-01 belum dapat diperiksa. Muat ulang status atau periksa koneksi panel-agent; token yang tersimpan belum tentu terputus.', 'The DNS-01 token status could not be checked. Refresh the status or check the panel-agent connection; the saved token may still be connected.')}</p>;
  if (dns.configured) return <p className="storage-note">{t('Token DNS-01 terhubung untuk zona:', 'DNS-01 token connected for zone:')} {dns.zone_name}</p>;
  return <div className="storage-notice"><p>{t('Cloudflare Tunnel dan token DNS-01 dihubungkan secara terpisah. Tunnel menyediakan jalur akses; penerbitan sertifikat ini memerlukan token untuk membuat record TXT validasi domain.', 'Cloudflare Tunnel and the DNS-01 token are connected separately. Tunnel provides the access route; this certificate requires a token to create domain-validation TXT records.')}</p>
    <p>{t('Buka Sertifikat DNS-01 → Hubungkan token DNS-01. Gunakan token khusus dengan izin Zone DNS Edit dan Zone Read untuk zona domain storage.', 'Open DNS-01 certificates → Connect DNS-01 token. Use a dedicated token with Zone DNS Edit and Zone Read permissions for the storage domain zone.')}</p>
    <a href="/settings?section=network&network_tab=certificates" target="_blank" rel="noopener noreferrer">{t('Hubungkan token DNS-01 (tab baru)', 'Connect DNS-01 token (new tab)')}</a>
    <p>{t('Setelah tersimpan, kembali ke dialog ini. Status diperiksa ulang saat Anda kembali; isian domain dan email tetap ada.', 'After saving, return to this dialog. The status is refreshed when you return; your domain and email remain filled in.')}</p>
  </div>;
}

export function BucketDialog({ english, context, contextLoading = false, contextError = '', onRefreshContext, ownerId, siteId, onClose, onSaved }: Localized & BucketContext & { ownerId?: number; siteId?: number; onClose: () => void; onSaved: (connection: StorageConnection) => void }) {
  const t = language(english); const formId = useId(); const [requestId] = useState(storageAllocationId);
  const [owner, setOwner] = useState(ownerId ?? context.sites.find(value => value.id === siteId)?.owner_id ?? context.owners[0]?.id ?? 0); const [site, setSite] = useState<number | null>(siteId ?? null); const [label, setLabel] = useState(''); const [bucket, setBucket] = useState(''); const [busy, setBusy] = useState(false); const [reserved, setReserved] = useState(false); const [error, setError] = useState('');
  const ownerReady = !contextLoading && !contextError && context.owners.some(value => value.id === owner);
  const siteReady = site === null || context.sites.some(value => value.id === site && value.owner_id === owner);
  useEffect(() => { if (!reserved && !ownerId && !owner && context.owners.length) setOwner(context.sites.find(value => value.id === siteId)?.owner_id ?? context.owners[0].id); }, [context.owners, context.sites, owner, ownerId, reserved, siteId]);
  const save = async (event: FormEvent) => { event.preventDefault(); if (busy) return; if (!ownerReady || !siteReady) { setError(t('Pilih tenant aktif dan project miliknya sebelum membuat bucket.', 'Choose an active tenant and a project they own before creating the bucket.')); return; } setBusy(true); setReserved(true); setError(''); try { onSaved(await allocateManagedBucket({ request_id: requestId, owner_id: owner, site_id: site, label, bucket })); } catch (error) { setError(errorMessage(error)); } finally { setBusy(false); } };
  return <Dialog english={english} title={t('Buat bucket Qinpanel', 'Create Qinpanel bucket')} description={t('Alokasikan penyimpanan private untuk tenant atau project.', 'Allocate private storage for a tenant or project.')} busy={busy} error={error} onClose={onClose} footer={<><button type="button" disabled={busy} onClick={onClose}>{t('Tutup', 'Close')}</button><button form={formId} type="submit" className="button-primary" disabled={busy || !ownerReady || !siteReady}>{busy ? t('Membuat bucket…', 'Creating bucket…') : reserved ? t('Lanjutkan alokasi', 'Resume allocation') : t('Buat bucket', 'Create bucket')}</button></>}>
    <BucketOwnerStatus english={english} context={context} contextLoading={contextLoading} contextError={contextError} onRefreshContext={onRefreshContext} ownerId={ownerId} />
    {ownerReady && !siteReady && <p role="alert" className="storage-error">{t('Project tidak tersedia untuk tenant pemilik yang dipilih. Muat ulang tenant atau pilih pemilik project yang sesuai.', 'The project is unavailable for the selected owner. Refresh tenants or choose the project owner.')}</p>}
    <form id={formId} onSubmit={save} className="storage-form-section" aria-busy={busy}><div className="storage-form-grid"><label>{t('Nama tampilan', 'Display name')}<input autoFocus required maxLength={80} value={label} disabled={busy || reserved} onChange={event => setLabel(event.target.value)} /></label><label>{t('Nama bucket', 'Bucket name')}<input required minLength={3} maxLength={63} pattern="[a-z0-9][a-z0-9-]{1,61}[a-z0-9]" placeholder="project-media" value={bucket} disabled={busy || reserved} onChange={event => setBucket(event.target.value)} /></label><label>Tenant<select aria-label="Tenant" required value={owner || ''} disabled={busy || reserved || !!ownerId} onChange={event => { setOwner(Number(event.target.value)); setSite(null); }}><option value="" disabled>{t('Pilih tenant', 'Choose tenant')}</option>{context.owners.map(value => <option key={value.id} value={value.id}>{value.username}</option>)}</select></label><label>Project<select aria-label="Project" value={site ?? ''} disabled={busy || reserved || !!siteId} onChange={event => setSite(event.target.value ? Number(event.target.value) : null)}><option value="">{t('Tanpa project', 'No project')}</option>{context.sites.filter(value => value.owner_id === owner).map(value => <option key={value.id} value={value.id}>{value.domain}</option>)}</select></label></div><p className="storage-note">{t('Bucket hanya dapat diakses oleh tenant pemilik melalui panel dan key khusus bucket. Pembuatan bucket/rotasi key memulai ulang engine sebentar; transfer S3 yang berjalan dapat terputus. Kapasitas berbagi disk server tanpa kuota keras per bucket.', 'Only the owning tenant can access the bucket through the panel and bucket-specific keys. Bucket creation/key rotation briefly restarts the engine; active S3 transfers may be interrupted. Capacity shares the server disk without hard per-bucket quotas.')}</p>{reserved && error && <p className="storage-note">{t('Identitas alokasi dipertahankan. Lanjutkan permintaan yang sama atau tutup dan sinkronkan bucket dari daftar storage.', 'The allocation identity is retained. Resume the same request or close and reconcile the bucket from the storage list.')}</p>}</form>
  </Dialog>;
}

export function ManagedKeyDialog({ english, connection, onClose, onIssued }: Localized & { connection: StorageConnection; onClose: () => void; onIssued: () => void }) {
  const t = language(english); const formId = useId(); const [confirmation, setConfirmation] = useState(''); const [keys, setKeys] = useState<StorageKeys | null>(null); const [showSecret, setShowSecret] = useState(false); const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  const pending = ['pending', 'unknown'].includes(connection.application_key_state ?? '');
  const issue = async (event: FormEvent) => { event.preventDefault(); if (busy || confirmation !== connection.bucket) return; setBusy(true); setError(''); try { const value = await issueStorageKey(connection, confirmation, pending); setKeys(value.credentials); onIssued(); } catch (error) { setError(errorMessage(error)); onIssued(); } finally { setBusy(false); } };
  return <Dialog english={english} title={t('Access key aplikasi', 'Application access key')} description={`${connection.bucket} · ${connection.endpoint}`} busy={busy} error={error} onClose={onClose} footer={<><button type="button" disabled={busy} onClick={onClose}>{keys ? t('Sudah disimpan, tutup', 'Saved, close') : t('Batal', 'Cancel')}</button>{!keys && <button form={formId} type="submit" className="button-primary" disabled={busy || confirmation !== connection.bucket}><KeyRound size={16} />{busy ? t('Memproses…', 'Processing…') : pending ? t('Sinkronkan key', 'Reconcile key') : t('Terbitkan key baru', 'Issue new key')}</button>}</>}>
    {keys ? <div className="storage-form-section"><p role="status" className="storage-notice">{t('Simpan sekarang. Secret ini hanya ditampilkan pada hasil penerbitan ini. Jangan masukkan ke kode frontend atau repositori Git.', 'Save now. This secret is only shown in this issuance result. Never put it in frontend code or a Git repository.')}</p><label>Access key ID<input readOnly value={keys.access_key} autoComplete="off" /></label><label>Secret access key<input readOnly type={showSecret ? 'text' : 'password'} value={keys.secret_key} autoComplete="off" onFocus={event => event.currentTarget.select()} /></label><button type="button" aria-pressed={showSecret} onClick={() => setShowSecret(value => !value)}>{showSecret ? t('Sembunyikan secret', 'Hide secret') : t('Tampilkan secret', 'Show secret')}</button><button type="button" onClick={async () => { try { if (!navigator.clipboard) throw new Error('Clipboard unavailable'); await navigator.clipboard.writeText(`AWS_ACCESS_KEY_ID=${keys.access_key}\nAWS_SECRET_ACCESS_KEY=${keys.secret_key}`); } catch { setError(t('Clipboard tidak tersedia. Tampilkan secret, lalu pilih dan salin key secara manual.', 'Clipboard unavailable. Show the secret, then select and copy the keys manually.')); } }}>{t('Salin kedua key', 'Copy both keys')}</button></div> : <form id={formId} onSubmit={issue} className="storage-form-section"><p>{t('Key hanya memberi akses baca, list dan tulis pada bucket ini. Rotasi mencabut key aplikasi sebelumnya. Kredensial internal panel tetap terpisah.', 'The key grants read, list and write access to this bucket only. Rotation revokes the previous application key. Internal panel credentials remain separate.')}</p><p className="storage-note">{t('Key SDK S3 langsung mengikuti izin engine dan melewati rules/alokasi panel. Terbitkan hanya untuk aplikasi server yang Anda percaya.', 'A direct S3 SDK key follows engine permissions and bypasses panel rules/allowances. Issue it only to server applications you trust.')}</p><p className="storage-note">{t('Engine akan dimulai ulang sebentar. Transfer S3 yang sedang berjalan dapat terputus.', 'The engine briefly restarts. Active S3 transfers may be interrupted.')}</p><label>{t('Ketik nama bucket untuk melanjutkan', 'Type the bucket name to continue')}<input autoFocus required value={confirmation} disabled={busy} onChange={event => setConfirmation(event.target.value)} autoComplete="off" /></label></form>}
  </Dialog>;
}
