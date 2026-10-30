import { useCallback, useEffect, useId, useState, type ReactNode } from 'react';
import { Cylinder, Globe2, LockKeyhole, RefreshCw, Settings2, FolderOpen, Plug } from 'lucide-react';
import { fetchStorageAccess, saveStorageAccess, storageAllocationId, type StorageAccess, type StorageConnection } from '../../api/storage';
import { ObjectExplorer } from './ObjectExplorer';
import { BucketPolicy } from './BucketPolicy';

export function OwnerBucketWorkspace({ connection, english, onBusy, connectionTools }: { connection: StorageConnection; english: boolean; onBusy: (busy: boolean) => void; connectionTools?: ReactNode }) {
  const t = (id: string, en: string) => english ? en : id;
  const workspaceId = useId();
  const [access, setAccess] = useState<StorageAccess | null>(null); const [error, setError] = useState(''); const [refresh, setRefresh] = useState(0);
  const [tab, setTab] = useState<'files' | 'settings' | 'connection'>('files'); const [busy, setBusy] = useState(false);
  const reportBusy = useCallback((value: boolean) => { setBusy(value); onBusy(value); }, [onBusy]);
  useEffect(() => { const controller = new AbortController(); fetchStorageAccess(connection.id, controller.signal).then(value => { if (!controller.signal.aborted) { setAccess(value); setError(''); } }).catch(error => { if (!controller.signal.aborted) setError(error instanceof Error ? error.message : 'Storage access unavailable'); }); return () => controller.abort(); }, [connection.id, refresh]);
  useEffect(() => {
    if (access?.state !== 'applying') return;
    const timer = window.setInterval(() => setRefresh(value => value + 1), 5000);
    return () => window.clearInterval(timer);
  }, [access?.state]);
  const label = !access ? t('Memuat akses…', 'Loading access…') : access.state !== 'ready' ? t('Perlu sinkronisasi', 'Needs reconciliation') : access.visibility === 'public' ? 'Public' : access.visibility === 'private' ? 'Private' : t('Diatur provider', 'Provider managed');
  return <section className="storage-owner-workspace" aria-label={t('File browser', 'File browser')}>
    <div className="storage-workspace-heading"><div className="storage-bucket-identity"><span className="storage-bucket-symbol"><Cylinder size={21} /></span><div><h2>{connection.bucket}</h2><p>{connection.label}<span> · </span>{connection.managed ? t('Disk server', 'Server disk') : t('Storage eksternal', 'External storage')}</p></div></div><button type="button" className={`storage-access-badge ${access?.visibility === 'public' ? 'is-public' : ''}`} disabled={busy} onClick={() => setTab('settings')}>{access?.visibility === 'public' ? <Globe2 size={14} /> : <LockKeyhole size={14} />}{label}<Settings2 size={13} /></button></div>
    <div className="storage-workspace-tabs" role="tablist" aria-label={t('Workspace bucket', 'Bucket workspace')} onKeyDown={event => {
      if (!['ArrowLeft','ArrowRight','Home','End'].includes(event.key)) return;
      const tabs = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]:not(:disabled)'));
      const index = tabs.indexOf(document.activeElement as HTMLButtonElement);
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length-1 : (index + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length;
      event.preventDefault(); tabs[next]?.focus(); tabs[next]?.click();
    }}><button id={`${workspaceId}-files`} aria-controls={`${workspaceId}-panel`} type="button" role="tab" aria-selected={tab === 'files'} tabIndex={tab === 'files' ? 0 : -1} disabled={busy} onClick={() => setTab('files')}><FolderOpen size={15} />{t('File', 'Files')}</button><button id={`${workspaceId}-settings`} aria-controls={`${workspaceId}-panel`} type="button" role="tab" aria-selected={tab === 'settings'} tabIndex={tab === 'settings' ? 0 : -1} disabled={busy} onClick={() => setTab('settings')}><Settings2 size={15} />{t('Pengaturan bucket', 'Bucket settings')}</button>{connectionTools && <button id={`${workspaceId}-connection`} aria-controls={`${workspaceId}-panel`} type="button" role="tab" aria-selected={tab === 'connection'} tabIndex={tab === 'connection' ? 0 : -1} disabled={busy} onClick={() => setTab('connection')}><Plug size={15} />{t('Koneksi S3', 'S3 connection')}</button>}</div>
    {error && <p className="storage-error" role="alert">{error}<button type="button" onClick={() => setRefresh(v => v+1)}><RefreshCw size={14} />{t('Coba lagi', 'Retry')}</button></p>}
    <div id={`${workspaceId}-panel`} role="tabpanel" aria-labelledby={`${workspaceId}-${tab === 'settings' ? 'settings' : tab === 'connection' ? 'connection' : 'files'}`}>{tab === 'files' ? <ObjectExplorer connection={connection} english={english} access={access} onBusy={reportBusy} /> : tab === 'connection' ? <div className="storage-connection-panel">{connectionTools}</div> : <div className="storage-bucket-settings">{access && <AccessEditor key={`${access.revision}:${access.state}`} connection={connection} access={access} english={english} onChanged={setAccess} onBusy={reportBusy} />}<BucketPolicy connection={connection} admin={false} english={english} locked={busy} /></div>}</div>
  </section>;
}

export function AccessEditor({ connection, access, english, onChanged, onBusy }: { connection: StorageConnection; access: StorageAccess; english: boolean; onChanged: (value: StorageAccess) => void; onBusy: (busy: boolean) => void }) {
  const t = (id: string, en: string) => english ? en : id;
  const [visibility, setVisibility] = useState<'private' | 'public'>(access.desired_visibility); const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  const save = async () => {
    if (busy) return; setBusy(true); onBusy(true); setError('');
    try { onChanged(await saveStorageAccess(connection.id, { request_id: access.state === 'unknown' && access.request_id ? access.request_id : storageAllocationId(), expected_revision: access.revision, visibility: access.state === 'unknown' ? access.desired_visibility : visibility })); }
    catch (error) { setError(error instanceof Error ? error.message : 'Access update failed'); try { onChanged(await fetchStorageAccess(connection.id)); } catch { /* Keep the original request failure visible. */ } }
    finally { setBusy(false); onBusy(false); }
  };
  return <section className="storage-access-editor" aria-label={t('Akses bucket', 'Bucket access')}><div className="storage-policy-heading"><div><h3>{t('Akses bucket', 'Bucket access')}</h3><p className="storage-note">{t('Tentukan siapa yang dapat membuka file lewat URL.', 'Choose who can open files using their URL.')}</p></div>{access.visibility === 'public' ? <Globe2 size={20} /> : <LockKeyhole size={20} />}</div>
    {access.can_change ? <><div className="storage-access-options">{(['private', 'public'] as const).map(value => <label key={value} className={visibility === value ? 'selected' : ''}><input type="radio" name={`bucket-access-${connection.id}`} value={value} checked={visibility === value} disabled={busy || access.state !== 'ready'} onChange={() => setVisibility(value)} /><span><strong>{value === 'public' ? 'Public' : 'Private'}</strong><small>{value === 'public' ? t('Siapa pun yang memiliki URL dapat membaca semua file di bucket ini.', 'Anyone with a URL can read all files in this bucket.') : t('File hanya dapat dibaca dengan autentikasi atau signed URL sementara.', 'Files require authentication or a temporary signed URL.')}</small></span></label>)}</div>
      <p className="storage-note">{t('Upload, hapus, dan pengelolaan file tetap memerlukan akses pemilik. Perubahan berlaku untuk seluruh isi bucket dan dapat dikembalikan.', 'Uploading, deletion and file management still require owner access. This applies to the whole bucket and can be reversed.')}</p>
      {access.state !== 'ready' && <p role="status" className="storage-notice">{t('Akses belum terkonfirmasi. Sinkronkan perubahan sebelum membuat URL atau mengubah akses lagi.', 'Access is not confirmed. Reconcile the pending change before creating URLs or changing access again.')}</p>}
      <button type="button" className="button-primary" disabled={busy || access.state === 'applying' || access.state === 'ready' && visibility === access.visibility} onClick={() => void save()}>{busy ? t('Menyimpan…', 'Saving…') : access.state === 'unknown' ? t('Sinkronkan perubahan akses', 'Reconcile access change') : t('Simpan akses bucket', 'Save bucket access')}</button>
    </> : <p className="storage-note">{t('Bucket eksternal mengikuti kebijakan penyedia. Signed URL tetap tersedia pada detail file; akses publik diatur melalui dashboard penyedia.', 'External buckets follow their provider policies. File details can create signed URLs; configure public access at the provider.')}</p>}
    {error && <p role="alert" className="storage-error">{error}</p>}
  </section>;
}
