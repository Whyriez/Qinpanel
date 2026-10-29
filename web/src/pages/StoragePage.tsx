import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { HardDrive, LockKeyhole, Plus, RefreshCw } from 'lucide-react';
import { useAuth } from '../context/AuthContext';
import { useBrowserRoute } from '../lib/router';
import { ConfirmDangerDialog } from '../components/ui/ConfirmDangerDialog';
import { ConnectionDialog } from '../components/storage/ConnectionDialog';
import { ManagedStorage, ManagedKeyDialog } from '../components/storage/ManagedStorage';
import { bindStorageProject, removeManagedBucket, reconcileManagedBucket, createStorageBucket, disconnectStorage, fetchStorageConnections, fetchStorageContext, projectStorageEnvironment, testStorageConnection, type StorageConnection, type StorageContext } from '../api/storage';
import { ObjectExplorer } from '../components/storage/ObjectExplorer';
import { BucketPolicy } from '../components/storage/BucketPolicy';
import '../styles/storage.css';

const message = (error: unknown) => error instanceof Error ? error.message : 'Storage request failed';
const providerName = (provider: string) => ({ aws: 'Amazon S3', r2: 'Cloudflare R2', supabase: 'Supabase S3', compatible: 'S3 compatible' }[provider] ?? provider);

export function StoragePage({ scopeSiteId, scopeOwnerId }: { scopeSiteId?: number; scopeOwnerId?: number }) {
  const { user } = useAuth(); const { i18n } = useTranslation(); const route = useBrowserRoute();
  const english = i18n.language.startsWith('en'); const t = (id: string, en: string) => english ? en : id;
  const querySite = Number(route.search.get('site')); const siteId = scopeSiteId ?? (Number.isSafeInteger(querySite) && querySite > 0 ? querySite : undefined);
  const [context, setContext] = useState<StorageContext>({ owners: [], sites: [], upload_limit_bytes: 33554432 });
  const [contextLoading, setContextLoading] = useState(true); const [contextError, setContextError] = useState(''); const [contextRefresh, setContextRefresh] = useState(0);
  const management = useRef<HTMLHeadingElement>(null);
  const [connections, setConnections] = useState<StorageConnection[]>([]); const [selectedId, setSelectedId] = useState(route.search.get('connection') ?? '');
  const [loading, setLoading] = useState(true); const [error, setError] = useState(''); const [notice, setNotice] = useState('');
  const [refresh, setRefresh] = useState(0); const [dialog, setDialog] = useState<StorageConnection | 'new' | null>(null);
  const [keyDialog, setKeyDialog] = useState<StorageConnection | null>(null);
  const [danger, setDanger] = useState<'disconnect' | 'create' | 'remove-managed' | null>(null); const [busy, setBusy] = useState(false); const [objectBusy, setObjectBusy] = useState(false);
  const connection = connections.find(value => value.id === selectedId); const admin = user?.role === 'admin'; const ownerSession = user?.role === 'user' && !user?.impersonated_by;
  useEffect(() => {
    const controller = new AbortController(); setLoading(true); setError('');
    fetchStorageConnections(siteId, controller.signal).then(response => {
      if (controller.signal.aborted) return; const visible = response.connections.filter(value => !scopeOwnerId || value.owner_id === scopeOwnerId); setConnections(visible);
      setSelectedId(previous => visible.some(value => value.id === previous) ? previous : visible[0]?.id ?? '');
    }).catch(error => { if (!controller.signal.aborted) setError(message(error)); }).finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [siteId, scopeOwnerId, refresh, user?.id]);
  useEffect(() => {
    const controller = new AbortController(); setContextLoading(true); setContextError('');
    fetchStorageContext(controller.signal).then(value => { if (!controller.signal.aborted) setContext(value); })
      .catch(error => { if (!controller.signal.aborted) setContextError(message(error)); })
      .finally(() => { if (!controller.signal.aborted) setContextLoading(false); });
    return () => controller.abort();
  }, [contextRefresh, refresh, user?.id]);
  useEffect(() => {
    const update = () => setContextRefresh(value => value + 1);
    window.addEventListener('focus', update);
    return () => window.removeEventListener('focus', update);
  }, []);
  const act = async (action: () => Promise<unknown>, success: string) => {
    setBusy(true); setError(''); setNotice('');
    try { await action(); setNotice(success); setDanger(null); setRefresh(value => value + 1); }
    catch (error) { setError(message(error)); } finally { setBusy(false); }
  };
  return <section className="storage-page" aria-label="Storage S3">
    <header className="storage-heading"><div><h1>Storage S3</h1><p>{admin ? t('Distribusikan disk server dan S3 ke tenant. Kelola alokasi tanpa membuka file mereka.', 'Distribute server disk and S3 to tenants. Manage allowances without opening their files.') : t('File, folder, dan akses aplikasi dalam satu ruang kerja private.', 'Files, folders and application access in one private workspace.')}</p></div>
      <div className="storage-actions"><button type="button" disabled={loading || busy || objectBusy} onClick={() => setRefresh(value => value + 1)}><RefreshCw size={16} /> {t('Muat ulang', 'Refresh')}</button>{admin && <button type="button" className="button-primary" disabled={busy || objectBusy || loading || contextLoading || !!contextError || !context.owners.length} onClick={() => setDialog('new')}><Plus size={16} /> {t('Hubungkan S3', 'Connect S3')}</button>}</div>
    </header>
    {admin && <ManagedStorage onChanged={() => setRefresh(value => value + 1)} refreshKey={refresh} disabled={busy || objectBusy} english={english} context={context} contextLoading={contextLoading} contextError={contextError} onRefreshContext={() => setContextRefresh(value => value + 1)} ownerId={scopeOwnerId} siteId={siteId} onManage={() => { management.current?.scrollIntoView({ behavior: 'smooth', block: 'start' }); management.current?.focus({ preventScroll: true }); }} onCreated={value => { setSelectedId(value.id); setRefresh(current => current + 1); }} />}
    {siteId && <p className="storage-note">{t('Koneksi untuk project ini.', 'Connections for this project.')} {!scopeSiteId && <a href="/storage">{t('Lihat semua storage', 'View all storage')}</a>}</p>}
    {error && <p role="alert" className="storage-error">{error}</p>}{notice && <p role="status" className="storage-notice">{notice}</p>}
    <div className="storage-management-heading"><h2 ref={management} tabIndex={-1}>{admin ? t('Bucket & alokasi tenant', 'Buckets & tenant allowances') : t('Bucket & file Anda', 'Your buckets & files')}</h2><p className="storage-note">{admin ? t('Bucket lokal dan S3 eksternal dapat aktif bersamaan. Isi bucket hanya dapat dibuka tenant pemilik.', 'Local buckets and external S3 can run together. Only the owner tenant can open bucket contents.') : t('Pilih bucket untuk mengelola file. Klik file untuk membuka preview, detail, dan tautan unduh.', 'Choose a bucket to manage files. Select a file for its preview, details and download link.')}</p></div>
    {loading ? <p role="status" className="storage-note">{t('Memuat koneksi storage…', 'Loading storage connections…')}</p> : !connections.length ? <div className="storage-empty"><HardDrive size={30} /><h2>{t('Belum ada bucket di panel', 'No buckets added yet')}</h2><p>{admin ? t('Buat bucket melalui Storage Qinpanel atau hubungkan bucket AWS S3, R2, Supabase, dan penyedia S3 lainnya.', 'Create a bucket with Qinpanel storage or connect AWS S3, R2, Supabase, and other S3 providers.') : t('Minta administrator menyiapkan koneksi bucket untuk tenant atau project Anda.', 'Ask your administrator to connect a bucket for your tenant or project.')}</p></div> : <>
      <label className="storage-mobile-picker">{t('Pilih koneksi', 'Choose connection')}<select value={selectedId} disabled={busy || objectBusy} onChange={e => { setSelectedId(e.target.value); setNotice(''); }} aria-label={t('Pilih koneksi storage', 'Choose storage connection')}>{connections.map(value => <option value={value.id} key={value.id}>{value.label} · {value.bucket}</option>)}</select></label>
      <div className="storage-layout"><aside className="storage-connections" aria-label={t('Koneksi storage', 'Storage connections')}>{connections.map(value => <button type="button" key={value.id} className="storage-connection" aria-current={value.id === selectedId} disabled={busy || objectBusy} onClick={() => { setSelectedId(value.id); setNotice(''); }}>{value.label}<small>{value.managed ? t('Disk server', 'Server disk') : providerName(value.provider)} · {value.bucket}</small></button>)}</aside>
        {connection && <div className="storage-content"><div className="storage-toolbar"><h2>{connection.label}</h2><div className="storage-actions">{ownerSession && <button type="button" disabled={busy || objectBusy} onClick={() => act(() => testStorageConnection(connection), t('Daftar bucket berhasil diakses. Izin download dan tulis mengikuti policy provider.', 'Bucket listing verified. Download and write permissions follow provider policy.'))}>{busy ? t('Memproses…', 'Processing…') : t('Uji koneksi', 'Test connection')}</button>}{admin && !connection.managed && <><button type="button" disabled={busy || objectBusy} onClick={() => setDialog(connection)}>{t('Edit koneksi', 'Edit connection')}</button><button type="button" disabled={busy || objectBusy} onClick={() => setDanger('disconnect')}>{t('Lepas koneksi', 'Disconnect')}</button></>}</div></div>
          {admin && connection.managed && <div className="storage-actions mt-4">{connection.managed_state === 'deleting' ? <p className="storage-note">{t('Lanjutkan penghapusan bucket yang terputus.', 'Resume the interrupted bucket removal.')}</p> : connection.managed_state !== 'ready' ? <button type="button" disabled={busy || objectBusy} onClick={() => act(() => reconcileManagedBucket(connection), t('Bucket disinkronkan.', 'Bucket reconciled.'))}>{t('Sinkronkan alokasi bucket', 'Reconcile bucket allocation')}</button> : null}<button type="button" disabled={busy || objectBusy} onClick={() => setDanger('remove-managed')}>{t('Hapus bucket kosong', 'Remove empty bucket')}</button></div>}
          <dl className="storage-detail"><div><dt>Provider</dt><dd>{connection.managed ? t('Disk server · Qinpanel', 'Server disk · Qinpanel') : providerName(connection.provider)}</dd></div><div><dt>Bucket</dt><dd>{connection.bucket}</dd></div><div><dt>Endpoint</dt><dd>{connection.endpoint}</dd></div><div><dt>Region / addressing</dt><dd>{connection.region} · {connection.addressing}</dd></div></dl>
          {admin && <p className="storage-note">{t('Pemilik', 'Owner')}: <strong>{context.owners.find(value => value.id === connection.owner_id)?.username ?? `#${connection.owner_id}`}</strong></p>}
          {connection.last_operation === 'unknown' && <p role="status" className="storage-notice">{t('Hasil transfer terakhir belum dipastikan. Periksa daftar object sebelum mencoba kembali.', 'The last transfer outcome is unknown. Check the object listing before retrying.')}</p>}
          {connection.last_operation === 'running' && <p role="status" className="storage-note">{t('Ada operasi yang masih berjalan di koneksi ini.', 'An operation is still running on this connection.')}</p>}
          {connection.managed && connection.managed_state !== 'ready' ? <p role="status" className="storage-notice">{t('Alokasi bucket belum siap. Administrator perlu menyelesaikan operasi storage yang tertunda.', 'The bucket allocation is not ready. An administrator must complete the pending storage operation.')}</p> : <BucketWorkspace key={`${user?.id}:${connection.id}:${connection.revision}:${refresh}`} admin={!ownerSession} impersonated={!!user?.impersonated_by} connection={connection} english={english} onBusy={setObjectBusy} />}
          {(admin || ownerSession) && <BucketPolicy key={`${connection.id}:${refresh}`} connection={connection} admin={admin} english={english} locked={busy || objectBusy} />}
          {ownerSession && connection.managed && connection.managed_state === 'ready' && <div className="storage-actions mt-4"><button type="button" disabled={busy || objectBusy} onClick={() => setKeyDialog(connection)}>{t('Access key aplikasi', 'Application access key')}</button></div>}
          <ProjectBinding key={`${connection.id}:${connection.revision}`} connection={connection} context={context} english={english} locked={!!scopeSiteId || busy || objectBusy || contextLoading || !!contextError} onChanged={() => setRefresh(value => value + 1)} />
          <details className="mt-6"><summary>{t('Konfigurasi aplikasi', 'Application configuration')}</summary><p className="storage-note">{t('Tambahkan konfigurasi ini melalui environment/secret project Anda. Tenant pemilik membuat access key bucket lokal melalui tombol Access key aplikasi; perubahan tidak diterapkan otomatis atau memulai ulang aplikasi.', 'Add this configuration through your project environment/secrets. The owner tenant issues local bucket keys through Application access key; changes are not applied automatically and do not restart the application.')}</p><pre className="storage-code">{projectStorageEnvironment(connection)}</pre></details>
          {admin && !connection.managed && <details className="mt-5"><summary>{t('Buat bucket baru di provider', 'Create a new bucket at the provider')}</summary><p className="storage-note">{t('Gunakan hanya jika bucket belum ada dan credential memiliki izin membuat bucket. Operasi dapat dikenai biaya provider.', 'Use only if the bucket does not exist and the credentials allow bucket creation. Provider charges may apply.')}</p><button type="button" disabled={busy || objectBusy} onClick={() => setDanger('create')}>{t('Buat bucket baru', 'Create new bucket')}</button></details>}
        </div>}
      </div></>}
    {ownerSession && keyDialog && <ManagedKeyDialog english={english} connection={connections.find(value => value.id === keyDialog.id) ?? keyDialog} onClose={() => setKeyDialog(null)} onIssued={() => setRefresh(value => value + 1)} />}
    {dialog && <ConnectionDialog key={dialog === 'new' ? 'new' : dialog.id} connection={dialog === 'new' ? undefined : dialog} context={context} ownerId={scopeOwnerId} siteId={siteId} english={english} onClose={() => setDialog(null)} onSaved={value => { setDialog(null); setSelectedId(value.id); setRefresh(current => current + 1); setNotice(t('Koneksi tersimpan untuk tenant pemilik.', 'Connection saved for its owner tenant.')); }} />}
    {connection && <ConfirmDangerDialog isOpen={danger !== null} onClose={() => setDanger(null)} isLoading={busy} title={danger === 'remove-managed' ? t('Hapus bucket Qinpanel kosong?', 'Remove empty Qinpanel bucket?') : danger === 'create' ? t('Buat bucket baru?', 'Create a new bucket?') : t('Lepas koneksi storage?', 'Disconnect storage?')} description={danger === 'remove-managed' ? t('Bucket harus kosong. Semua access key bucket ini akan dicabut dan alokasinya dihapus. Engine dimulai ulang sebentar sehingga transfer S3 dapat terputus.', 'The bucket must be empty. All its access keys will be revoked and its allocation removed. The engine briefly restarts, which can interrupt S3 transfers.') : danger === 'create' ? t('Panel akan meminta provider membuat bucket dengan nama berikut. Izin dan tagihan mengikuti provider.', 'The panel will ask the provider to create the named bucket. Provider permissions and billing apply.') : t('Credential dan koneksi di panel dihapus. Bucket dan object di provider tetap ada.', 'Panel credentials and connection metadata will be removed. Provider buckets and objects are retained.')} confirmMatchText={connection.bucket} confirmButtonLabel={danger === 'remove-managed' ? t('Hapus bucket kosong', 'Remove empty bucket') : danger === 'create' ? t('Buat bucket', 'Create bucket') : t('Lepas koneksi', 'Disconnect')} onConfirm={() => act(() => danger === 'remove-managed' ? removeManagedBucket(connection) : danger === 'create' ? createStorageBucket(connection) : disconnectStorage(connection), danger === 'remove-managed' ? t('Bucket kosong dihapus dan semua key dicabut.', 'Empty bucket removed and all its keys revoked.') : danger === 'create' ? t('Permintaan pembuatan bucket berhasil. Muat ulang object untuk memeriksa.', 'Bucket creation request succeeded. Refresh objects to verify.') : t('Koneksi dilepas; data provider tetap ada.', 'Disconnected; provider data is retained.'))} />}
  </section>;
}

export function BucketWorkspace({ admin, impersonated, connection, english, onBusy }: { admin: boolean; impersonated: boolean; connection: StorageConnection; english: boolean; onBusy: (busy: boolean) => void }) {
  if (admin || impersonated) return <div className="storage-private-workspace"><LockKeyhole size={26} /><h3>{english ? 'Tenant files stay private' : 'File tenant tetap private'}</h3><p>{english ? 'This session can manage bucket metadata. File names, previews, download links and application keys are available only to the owner tenant in their own session.' : 'Sesi ini dapat mengelola metadata bucket. Nama file, preview, tautan unduh, dan access key aplikasi hanya tersedia bagi tenant pemilik melalui sesi sendiri.'}</p></div>;
  return <ObjectExplorer connection={connection} english={english} onBusy={onBusy} />;
}

function ProjectBinding({ connection, context, english, locked, onChanged }: { connection: StorageConnection; context: StorageContext; english: boolean; locked: boolean; onChanged: () => void }) {
  const t = (id: string, en: string) => english ? en : id; const [site, setSite] = useState(String(connection.site_id ?? '')); const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  return <form className="storage-project" onSubmit={async event => { event.preventDefault(); setBusy(true); setError(''); try { await bindStorageProject(connection, site ? Number(site) : null); onChanged(); } catch (error) { setError(message(error)); } finally { setBusy(false); } }}><label>{t('Hubungkan ke project', 'Link to project')}<select value={site} disabled={locked || busy} onChange={e => setSite(e.target.value)}><option value="">{t('Tanpa project', 'No project')}</option>{context.sites.filter(site => site.owner_id === connection.owner_id).map(site => <option value={site.id} key={site.id}>{site.domain}</option>)}</select></label><button type="submit" disabled={locked || busy || site === String(connection.site_id ?? '')}>{t('Simpan hubungan', 'Save project link')}</button>{error && <p role="alert" className="storage-error">{error}</p>}</form>;
}
