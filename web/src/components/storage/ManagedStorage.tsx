import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { Database, KeyRound, Plus, RefreshCw, X } from 'lucide-react';
import { ModalOverlay } from '../ui/ModalOverlay';
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

export function ManagedStorage({ english, context, ownerId, siteId, refreshKey = 0, disabled = false, onChanged, onCreated }: Localized & { onChanged?: () => void; refreshKey?: number; disabled?: boolean; context: StorageContext; ownerId?: number; siteId?: number; onCreated: (connection: StorageConnection) => void }) {
  const t = language(english);
  const [status, setStatus] = useState<ManagedStorageStatus | null>(null); const [error, setError] = useState(''); const [loading, setLoading] = useState(true); const [refresh, setRefresh] = useState(0);
  const [dialog, setDialog] = useState<'configure' | 'bucket' | null>(null);
  useEffect(() => { const controller = new AbortController(); setLoading(true); setError(''); fetchManagedStorage(controller.signal).then(value => { if (!controller.signal.aborted) setStatus(value); }).catch(error => { if (!controller.signal.aborted) { setStatus(null); setError(errorMessage(error)); } }).finally(() => { if (!controller.signal.aborted) setLoading(false); }); return () => controller.abort(); }, [refresh, refreshKey]);
  return <section className="storage-managed" aria-label={t('Storage Qinpanel', 'Qinpanel storage')}>
    <div className="storage-managed-heading"><div><h2><Database size={20} /> {t('Storage Qinpanel', 'Qinpanel storage')}</h2><p>{t('Sediakan bucket dan API S3 dari server Anda sendiri.', 'Provide buckets and an S3 API from your own server.')}</p></div><span className="storage-managed-status">{loading ? t('Memeriksa…', 'Checking…') : !status ? t('Belum diketahui', 'Unknown') : status.running ? t('Aktif', 'Running') : status.configured ? t('Perlu diperiksa', 'Needs attention') : t('Belum diaktifkan', 'Not activated')}</span></div>
    {error && <p role="alert" className="storage-error">{error}</p>}
    {status?.configured && <dl className="storage-detail"><div><dt>Endpoint S3</dt><dd>{status.endpoint}</dd></div><div><dt>{t('Ruang tersedia / volume', 'Available / volume')}</dt><dd>{capacity(status.available_bytes)} / {capacity(status.capacity_bytes)}</dd></div><div><dt>{t('Alokasi bucket', 'Bucket allocations')}</dt><dd>{status.bucket_count}</dd></div></dl>}
    <p className="storage-note">{t('Bucket bersifat private. Kapasitas memakai disk server bersama; batas keras per bucket belum diterapkan. Penyedia S3 lain tetap dapat dihubungkan melalui Hubungkan S3.', 'Buckets are private. Capacity uses the shared server disk; hard per-bucket limits are not enforced. Connect another S3 provider using Connect S3.')}</p>
    <p className="storage-note">{t('Data dan konfigurasi engine belum termasuk backup server panel. Siapkan backup storage terpisah sebelum menyimpan data penting.', 'Engine data and configuration are not included in panel server backups. Set up a separate storage backup before storing important data.')}</p>
    <div className="storage-actions"><button className="button-primary" type="button" disabled={disabled || loading || !status || dialog !== null} onClick={() => setDialog('configure')}>{status?.configured ? t('Sertifikat & pengaturan', 'Certificate & settings') : t('Aktifkan storage Qinpanel', 'Activate Qinpanel storage')}</button><button type="button" disabled={disabled || loading || !status?.running || !context.owners.length || dialog !== null} onClick={() => setDialog('bucket')}><Plus size={16} />{t('Buat bucket Qinpanel', 'Create Qinpanel bucket')}</button><button type="button" aria-label={t('Periksa layanan storage', 'Check storage service')} disabled={disabled || loading || dialog !== null} onClick={() => setRefresh(value => value + 1)}><RefreshCw size={16} /></button></div>
    {dialog === 'configure' && <ConfigureDialog english={english} status={status} onClose={() => setDialog(null)} onSaved={value => { setStatus(value); setDialog(null); }} />}
    {dialog === 'bucket' && <BucketDialog english={english} context={context} ownerId={ownerId} siteId={siteId} onClose={() => { setDialog(null); setRefresh(value => value + 1); onChanged?.(); }} onSaved={value => { setDialog(null); setRefresh(value => value + 1); onCreated(value); }} />}
  </section>;
}

export function ConfigureDialog({ english, status, onClose, onSaved }: Localized & { status: ManagedStorageStatus | null; onClose: () => void; onSaved: (status: ManagedStorageStatus) => void }) {
  const t = language(english); const formId = useId();
  const [host, setHost] = useState(() => status?.endpoint ? new URL(status.endpoint).hostname : '');
  const [method, setMethod] = useState<'auto' | 'manual'>('auto');
  const [email, setEmail] = useState(''); const [confirmed, setConfirmed] = useState(false);
  const [chain, setChain] = useState(''); const [key, setKey] = useState('');
  const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  const [inventory, setInventory] = useState<StorageTlsInventory | null>(null); const [reload, setReload] = useState(0);
  const requestId = useRef<string | null>(null); const saved = useRef(onSaved); saved.current = onSaved;
  const observedPending = useRef(false);
  const pending = !!inventory && ['queued', 'issuing', 'configuring'].includes(inventory.certificate.state);
  useEffect(() => {
    const controller = new AbortController(); let timer: ReturnType<typeof setTimeout> | undefined;
    const load = async () => {
      try {
        const value = await fetchStorageTls(controller.signal);
        if (controller.signal.aborted) return;
        setInventory(value);
        setHost(current => current || value.certificate.hostname || '');
        setEmail(current => current || value.certificate.email || '');
        if (['queued', 'issuing', 'configuring'].includes(value.certificate.state)) observedPending.current = true;
        if (value.certificate.state === 'active' && (observedPending.current || value.certificate.request_id === requestId.current && requestId.current)) {
          const service = await fetchManagedStorage(controller.signal);
          if (!controller.signal.aborted && service.configured && service.running) saved.current(service);
        } else if (['queued', 'issuing', 'configuring'].includes(value.certificate.state)) timer = setTimeout(() => { void load(); }, 3000);
      } catch (error) { if (!controller.signal.aborted) setError(errorMessage(error)); }
    };
    void load();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [reload]);
  const save = async (event: FormEvent) => {
    event.preventDefault(); if (busy || pending) return; setBusy(true); setError('');
    try {
      if (method === 'auto') {
        if (!inventory || !confirmed) return;
        if (inventory.certificate.state === 'failed' && inventory.certificate.request_id === requestId.current) requestId.current = null;
        requestId.current ??= storageAllocationId();
        const certificate = await generateStorageTls({ request_id: requestId.current, expected_revision: inventory.certificate.revision, hostname: host.trim(), email: email.trim(), confirmed });
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
  const automatic = method === 'auto';
  return <Dialog english={english} title={t('Aktifkan storage Qinpanel', 'Activate Qinpanel storage')} description={t('Layanan S3 tersendiri di server ini, dengan endpoint HTTPS untuk aplikasi Anda.', 'A separate S3 service on this server, with an HTTPS endpoint for your applications.')} busy={busy} error={error} onClose={onClose} footer={<>
    <button type="button" disabled={busy} onClick={onClose}>{pending ? t('Tutup', 'Close') : t('Batal', 'Cancel')}</button>
    <button form={formId} type="submit" className="button-primary" disabled={busy || pending || automatic && (!inventory?.dns.configured || !confirmed)}>{busy ? t('Memproses…', 'Processing…') : pending ? t('Penerbitan berjalan…', 'Issuance in progress…') : automatic ? t('Generate & aktifkan', 'Generate & activate') : status?.configured ? t('Perbarui sertifikat', 'Update certificate') : t('Aktifkan layanan', 'Activate service')}</button>
  </>}>
    <form id={formId} onSubmit={save} className="storage-form-section" aria-busy={busy}>
      <p className="storage-note">{t('Gunakan domain terpisah dari panel dan website. Domain dapat dihubungkan ke IP publik server atau melalui Cloudflare Tunnel.', 'Use a domain separate from the panel and websites. Connect it to the server public IP or through Cloudflare Tunnel.')}</p>
      <label>{t('Domain storage', 'Storage domain')}<input autoFocus required pattern="[a-z0-9.-]+" placeholder="storage.example.com" value={host} readOnly={!!status?.configured} disabled={busy || pending} onChange={event => { setHost(event.target.value); resetRequest(); }} /></label>
      <label>{t('Sertifikat HTTPS', 'HTTPS certificate')}<select aria-label={t('Sertifikat HTTPS', 'HTTPS certificate')} value={method} disabled={busy || pending} onChange={event => { setMethod(event.target.value as 'auto' | 'manual'); setChain(''); setKey(''); resetRequest(); }}>
        <option value="auto">{t("Generate otomatis · Let's Encrypt", "Generate automatically · Let's Encrypt")}</option>
        <option value="manual">{t('Gunakan sertifikat sendiri (PEM)', 'Use your own certificate (PEM)')}</option>
      </select></label>
      {automatic ? <>
        <p className="storage-note">{t('Validasi DNS-01 Cloudflare bekerja untuk IP publik maupun Tunnel, tanpa membuka port 80. Private key dibuat dan dipasang oleh panel; tidak perlu disalin.', 'Cloudflare DNS-01 works with public IPs and Tunnels without opening port 80. The panel generates and installs the private key; no copying is required.')}</p>
        {inventory?.dns.configured ? <p className="storage-note">{t('Zona DNS terhubung:', 'Connected DNS zone:')} {inventory.dns.zone_name}</p> : <p className="storage-notice">{t('Hubungkan token DNS-01 Cloudflare terlebih dahulu di', 'Connect a Cloudflare DNS-01 token first in')} <a href="/settings?section=network">{t('Pengaturan server → DNS dan Cloudflare', 'Server settings → DNS and Cloudflare')}</a>.</p>}
        <label>{t('Email sertifikat', 'Certificate email')}<input type="email" required maxLength={254} autoComplete="email" value={email} disabled={busy || pending} onChange={event => { setEmail(event.target.value); resetRequest(); }} /></label>
        <label className="storage-tls-consent"><input type="checkbox" required checked={confirmed} disabled={busy || pending} onChange={event => setConfirmed(event.target.checked)} /><span>{t("Saya menyetujui ketentuan Let's Encrypt, penerbitan sertifikat untuk domain ini, dan perpanjangan otomatis menggunakan token DNS yang terhubung.", "I agree to the Let's Encrypt terms, certificate issuance for this domain, and automatic renewal using the connected DNS token.")} <a href="https://letsencrypt.org/repository/" target="_blank" rel="noreferrer">{t('Baca ketentuan', 'Read terms')}</a></span></label>
        {inventory?.certificate.not_after && <p className="storage-note">{t('Sertifikat aktif berakhir:', 'Active certificate expires:')} {new Date(inventory.certificate.not_after * 1000).toLocaleDateString()}</p>}
        {inventory?.certificate.error && <p role="alert" className="storage-error">{inventory.certificate.error}</p>}
        {pending && <p role="status" className="storage-notice">{t('Penerbitan dan aktivasi berjalan di latar belakang. Anda boleh menutup dialog; status tersimpan dan perpanjangan dijadwalkan otomatis.', 'Issuance and activation run in the background. You can close this dialog; status is persisted and renewal is scheduled automatically.')}</p>}
        <button type="button" disabled={busy} onClick={() => { setError(''); setReload(value => value + 1); }}>{t('Muat ulang status sertifikat', 'Refresh certificate status')}</button>
      </> : <>
        <label>{t('Sertifikat lengkap (PEM)', 'Full certificate chain (PEM)')}<textarea required rows={4} maxLength={65536} value={chain} spellCheck={false} autoComplete="off" disabled={busy || pending} onChange={event => setChain(event.target.value)} placeholder="-----BEGIN CERTIFICATE-----" /></label>
        <label>{t('Private key (PEM)', 'Private key (PEM)')}<textarea required rows={4} maxLength={65536} value={key} spellCheck={false} autoComplete="off" disabled={busy || pending} onChange={event => setKey(event.target.value)} placeholder="-----BEGIN PRIVATE KEY-----" /></label>
        <p className="storage-note">{t('Pemasangan sertifikat sendiri menonaktifkan perpanjangan otomatis storage. Perbarui PEM sebelum sertifikat berakhir.', 'Installing your own certificate disables automatic storage renewal. Update the PEM files before the certificate expires.')}</p>
      </>}
    </form>
  </Dialog>;
}

function BucketDialog({ english, context, ownerId, siteId, onClose, onSaved }: Localized & { context: StorageContext; ownerId?: number; siteId?: number; onClose: () => void; onSaved: (connection: StorageConnection) => void }) {
  const t = language(english); const formId = useId(); const [requestId] = useState(storageAllocationId);
  const [owner, setOwner] = useState(ownerId ?? context.owners[0]?.id ?? 0); const [site, setSite] = useState<number | null>(siteId ?? null); const [label, setLabel] = useState(''); const [bucket, setBucket] = useState(''); const [busy, setBusy] = useState(false); const [reserved, setReserved] = useState(false); const [error, setError] = useState('');
  const save = async (event: FormEvent) => { event.preventDefault(); if (busy) return; setBusy(true); setReserved(true); setError(''); try { onSaved(await allocateManagedBucket({ request_id: requestId, owner_id: owner, site_id: site, label, bucket })); } catch (error) { setError(errorMessage(error)); } finally { setBusy(false); } };
  return <Dialog english={english} title={t('Buat bucket Qinpanel', 'Create Qinpanel bucket')} description={t('Alokasikan penyimpanan private untuk tenant atau project.', 'Allocate private storage for a tenant or project.')} busy={busy} error={error} onClose={onClose} footer={<><button type="button" disabled={busy} onClick={onClose}>{t('Tutup', 'Close')}</button><button form={formId} type="submit" className="button-primary" disabled={busy}>{busy ? t('Membuat bucket…', 'Creating bucket…') : reserved ? t('Lanjutkan alokasi', 'Resume allocation') : t('Buat bucket', 'Create bucket')}</button></>}>
    <form id={formId} onSubmit={save} className="storage-form-section" aria-busy={busy}><div className="storage-form-grid"><label>{t('Nama tampilan', 'Display name')}<input autoFocus required maxLength={80} value={label} disabled={busy || reserved} onChange={event => setLabel(event.target.value)} /></label><label>{t('Nama bucket', 'Bucket name')}<input required minLength={3} maxLength={63} pattern="[a-z0-9][a-z0-9-]{1,61}[a-z0-9]" placeholder="project-media" value={bucket} disabled={busy || reserved} onChange={event => setBucket(event.target.value)} /></label><label>Tenant<select aria-label="Tenant" required value={owner || ''} disabled={busy || reserved || !!ownerId} onChange={event => { setOwner(Number(event.target.value)); setSite(null); }}><option value="" disabled>{t('Pilih tenant', 'Choose tenant')}</option>{context.owners.map(value => <option key={value.id} value={value.id}>{value.username}</option>)}</select></label><label>Project<select aria-label="Project" value={site ?? ''} disabled={busy || reserved || !!siteId} onChange={event => setSite(event.target.value ? Number(event.target.value) : null)}><option value="">{t('Tanpa project', 'No project')}</option>{context.sites.filter(value => value.owner_id === owner).map(value => <option key={value.id} value={value.id}>{value.domain}</option>)}</select></label></div><p className="storage-note">{t('Bucket hanya dapat diakses oleh tenant pemilik melalui panel dan key khusus bucket. Pembuatan bucket/rotasi key memulai ulang engine sebentar; transfer S3 yang berjalan dapat terputus. Kapasitas berbagi disk server tanpa kuota keras per bucket.', 'Only the owning tenant can access the bucket through the panel and bucket-specific keys. Bucket creation/key rotation briefly restarts the engine; active S3 transfers may be interrupted. Capacity shares the server disk without hard per-bucket quotas.')}</p>{reserved && error && <p className="storage-note">{t('Identitas alokasi dipertahankan. Lanjutkan permintaan yang sama atau tutup dan sinkronkan bucket dari daftar storage.', 'The allocation identity is retained. Resume the same request or close and reconcile the bucket from the storage list.')}</p>}</form>
  </Dialog>;
}

export function ManagedKeyDialog({ english, connection, onClose, onIssued }: Localized & { connection: StorageConnection; onClose: () => void; onIssued: () => void }) {
  const t = language(english); const formId = useId(); const [confirmation, setConfirmation] = useState(''); const [keys, setKeys] = useState<StorageKeys | null>(null); const [showSecret, setShowSecret] = useState(false); const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  const pending = ['pending', 'unknown'].includes(connection.application_key_state ?? '');
  const issue = async (event: FormEvent) => { event.preventDefault(); if (busy || confirmation !== connection.bucket) return; setBusy(true); setError(''); try { const value = await issueStorageKey(connection, confirmation, pending); setKeys(value.credentials); onIssued(); } catch (error) { setError(errorMessage(error)); onIssued(); } finally { setBusy(false); } };
  return <Dialog english={english} title={t('Access key aplikasi', 'Application access key')} description={`${connection.bucket} · ${connection.endpoint}`} busy={busy} error={error} onClose={onClose} footer={<><button type="button" disabled={busy} onClick={onClose}>{keys ? t('Sudah disimpan, tutup', 'Saved, close') : t('Batal', 'Cancel')}</button>{!keys && <button form={formId} type="submit" className="button-primary" disabled={busy || confirmation !== connection.bucket}><KeyRound size={16} />{busy ? t('Memproses…', 'Processing…') : pending ? t('Sinkronkan key', 'Reconcile key') : t('Terbitkan key baru', 'Issue new key')}</button>}</>}>
    {keys ? <div className="storage-form-section"><p role="status" className="storage-notice">{t('Simpan sekarang. Secret ini hanya ditampilkan pada hasil penerbitan ini. Jangan masukkan ke kode frontend atau repositori Git.', 'Save now. This secret is only shown in this issuance result. Never put it in frontend code or a Git repository.')}</p><label>Access key ID<input readOnly value={keys.access_key} autoComplete="off" /></label><label>Secret access key<input readOnly type={showSecret ? 'text' : 'password'} value={keys.secret_key} autoComplete="off" onFocus={event => event.currentTarget.select()} /></label><button type="button" aria-pressed={showSecret} onClick={() => setShowSecret(value => !value)}>{showSecret ? t('Sembunyikan secret', 'Hide secret') : t('Tampilkan secret', 'Show secret')}</button><button type="button" onClick={async () => { try { if (!navigator.clipboard) throw new Error('Clipboard unavailable'); await navigator.clipboard.writeText(`AWS_ACCESS_KEY_ID=${keys.access_key}\nAWS_SECRET_ACCESS_KEY=${keys.secret_key}`); } catch { setError(t('Clipboard tidak tersedia. Tampilkan secret, lalu pilih dan salin key secara manual.', 'Clipboard unavailable. Show the secret, then select and copy the keys manually.')); } }}>{t('Salin kedua key', 'Copy both keys')}</button></div> : <form id={formId} onSubmit={issue} className="storage-form-section"><p>{t('Key hanya memberi akses baca, list dan tulis pada bucket ini. Rotasi mencabut key aplikasi sebelumnya. Kredensial internal panel tetap terpisah.', 'The key grants read, list and write access to this bucket only. Rotation revokes the previous application key. Internal panel credentials remain separate.')}</p><p className="storage-note">{t('Engine akan dimulai ulang sebentar. Transfer S3 yang sedang berjalan dapat terputus.', 'The engine briefly restarts. Active S3 transfers may be interrupted.')}</p><label>{t('Ketik nama bucket untuk melanjutkan', 'Type the bucket name to continue')}<input autoFocus required value={confirmation} disabled={busy} onChange={event => setConfirmation(event.target.value)} autoComplete="off" /></label></form>}
  </Dialog>;
}
