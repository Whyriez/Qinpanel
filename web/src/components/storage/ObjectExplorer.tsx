import { useEffect, useRef, useState, type FormEvent } from 'react';
import { ArrowLeft, Check, Copy, Download, ExternalLink, File, Folder, Globe2, Image, LayoutGrid, List, LockKeyhole, RefreshCw, Search, Trash2, Upload, X } from 'lucide-react';
import { ConfirmDangerDialog } from '../ui/ConfirmDangerDialog';
import { deleteStorageObject, downloadStorageObject, fetchStorageObjects, fetchStorageObjectLink, previewStorageObject, publicStorageUrl, safeDownload, safeStorageObjectLink, uploadStorageObject, validObjectKey, type StorageAccess, type StorageConnection, type StorageListing, type StorageObject, type StorageObjectLink } from '../../api/storage';
import { storageBytes } from './BucketPolicy';

const message = (error: unknown) => error instanceof Error ? error.message : 'Storage request failed';
const imageFile = (key: string) => /\.(png|jpe?g|gif|webp)$/i.test(key);
const fileName = (key: string) => key.replace(/\/$/, '').split('/').pop() || key;

export function ObjectExplorer({ connection, english, access, onBusy }: { connection: StorageConnection; english: boolean; access?: StorageAccess | null; onBusy: (busy: boolean) => void }) {
  const t = (id: string, en: string) => english ? en : id;
  const conditional = connection.conditional_put ?? ['aws', 'r2'].includes(connection.provider);
  const [prefix, setPrefix] = useState(''); const [draftPrefix, setDraftPrefix] = useState(''); const [tokens, setTokens] = useState<(string | null)[]>([null]);
  const [listing, setListing] = useState<StorageListing | null>(null); const [loading, setLoading] = useState(true);
  const [error, setError] = useState(''); const [notice, setNotice] = useState(''); const [refresh, setRefresh] = useState(0);
  const [busy, setBusy] = useState(false); const [verified, setVerified] = useState(false); const [deleting, setDeleting] = useState<string | null>(null);
  const [selected, setSelected] = useState<StorageObject | null>(null); const [view, setView] = useState<'list' | 'grid'>('list'); const [search, setSearch] = useState('');
  const [showUpload, setShowUpload] = useState(false); const [file, setFile] = useState<globalThis.File | null>(null); const [key, setKey] = useState(''); const [overwrite, setOverwrite] = useState(false); const [progress, setProgress] = useState<number | null>(null);
  const upload = useRef<AbortController | null>(null); const fileInput = useRef<HTMLInputElement>(null); const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; upload.current?.abort(); onBusy(false); }; }, [onBusy]);
  useEffect(() => {
    const controller = new AbortController(); setLoading(true); setListing(null); setError(''); setVerified(false);
    fetchStorageObjects(connection, prefix, tokens[tokens.length - 1], controller.signal).then(value => {
      if (!controller.signal.aborted) { setListing(value.listing); setVerified(true); setSelected(previous => previous ? value.listing.objects.find(object => object.key === previous.key) ?? null : null); }
    }).catch(error => { if (!controller.signal.aborted) setError(message(error)); }).finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [connection, prefix, tokens, refresh]);
  const changePrefix = (value: string) => { setPrefix(value); setDraftPrefix(value); setTokens([null]); setSelected(null); setSearch(''); setNotice(''); };
  const act = async (action: () => Promise<void>) => {
    if (busy) return; setBusy(true); onBusy(true); setError(''); setNotice('');
    try { await action(); } catch (error) { if (alive.current) { setError(message(error)); setVerified(false); } }
    finally { if (alive.current) { setBusy(false); onBusy(false); setProgress(null); } }
  };
  const send = (event: FormEvent) => {
    event.preventDefault(); if (!file || !validObjectKey(key) || !conditional && !overwrite) return;
    const uploaded = file; const uploadedKey = key;
    void act(async () => {
      const controller = new AbortController(); upload.current = controller; setProgress(0);
      await uploadStorageObject(connection, uploaded, uploadedKey, overwrite, value => { if (alive.current) setProgress(value); }, controller.signal);
      if (alive.current) {
        setNotice(t('Upload berhasil. Pilih file untuk preview, detail, dan URL akses.', 'Upload confirmed. Select a file for its preview, details and access URL.'));
        setSelected({ key: uploadedKey, size: uploaded.size, modified_at: new Date().toISOString() });
        setFile(null); setKey(''); setOverwrite(false); setShowUpload(false); if (fileInput.current) fileInput.current.value = '';
        // Listing the exact prefix makes a new upload discoverable even in a paginated bucket.
        setPrefix(uploadedKey); setDraftPrefix(uploadedKey); setTokens([null]); setSearch(''); setRefresh(value => value + 1);
      }
    });
  };
  const objects = listing?.objects.filter(object => object.key.toLowerCase().includes(search.toLowerCase())) ?? [];
  const folders = listing?.prefixes.filter(value => value.toLowerCase().includes(search.toLowerCase())) ?? [];
  return <section className="storage-explorer" aria-label={t('File dalam bucket', 'Bucket files')}>
    <div className="storage-explorer-toolbar"><div className="storage-breadcrumb"><Folder size={17} /><strong>{connection.bucket}</strong><span>/</span><span title={prefix}>{prefix || t('Semua file', 'All files')}</span></div><div className="storage-actions"><button type="button" disabled={busy || loading} aria-label={t('Muat ulang file', 'Refresh files')} onClick={() => setRefresh(value => value + 1)}><RefreshCw size={16} /></button><button type="button" className="button-primary" disabled={busy || !verified} aria-expanded={showUpload} onClick={() => setShowUpload(value => !value)}><Upload size={16} />{t('Upload file', 'Upload file')}</button></div></div>
    {showUpload && <section className="storage-upload storage-upload-card"><div className="storage-toolbar"><h3>{t('Upload ke bucket', 'Upload to bucket')}</h3><button type="button" disabled={busy} onClick={() => setShowUpload(false)} aria-label={t('Tutup upload', 'Close upload')}><X size={16} /></button></div><form onSubmit={send}>
      <label className="storage-file-picker"><Upload size={22} /><span>{t('Pilih file dari perangkat', 'Choose a file from your device')}</span><small>{t('Maksimum 32 MiB; rules bucket dapat membatasi lebih kecil.', 'Maximum 32 MiB; bucket rules may impose a smaller limit.')}</small><input ref={fileInput} type="file" disabled={busy || !verified} onChange={event => { const next = event.target.files?.[0] ?? null; setFile(next); const folder = prefix.endsWith('/') ? prefix : prefix.split('/').slice(0, -1).join('/'); setKey(next ? (folder ? `${folder.replace(/\/$/, '')}/` : '') + next.name : ''); }} /></label>
      <label>Object key<input required maxLength={1024} disabled={busy || !verified} value={key} placeholder="uploads/photo.png" onChange={event => setKey(event.target.value)} /></label>
      <label className="storage-check"><input type="checkbox" checked={overwrite} disabled={busy || !verified} onChange={event => setOverwrite(event.target.checked)} />{t('Izinkan menimpa object dengan nama sama', 'Allow overwriting an object with the same key')}</label>
      {!conditional && <p className="storage-note">{t('Provider ini belum mendukung pencegahan overwrite terverifikasi. Gunakan key unik dan setujui penimpaan untuk melanjutkan.', 'Overwrite prevention is unverified for this provider. Use a unique key and approve overwriting to continue.')}</p>}
      {file && file.size > 33554432 && <p role="alert" className="storage-error">{t('File melebihi 32 MiB.', 'File exceeds 32 MiB.')}</p>}
      {progress !== null && <div role="status"><progress max={100} value={progress} aria-label={t('File dikirim ke panel', 'File sent to panel')} /><p className="storage-note">{progress < 100 ? `${progress}%` : t('Menunggu konfirmasi provider…', 'Waiting for provider confirmation…')}</p><button type="button" onClick={() => upload.current?.abort()}>{t('Batalkan transfer', 'Cancel transfer')}</button></div>}
      <div><button type="submit" className="button-primary" disabled={busy || !verified || !file || file.size > 33554432 || !validObjectKey(key) || !conditional && !overwrite}>{t('Mulai upload', 'Start upload')}</button></div>
    </form></section>}
    <form className="storage-prefix" onSubmit={event => { event.preventDefault(); changePrefix(draftPrefix); }}><label>{t('Folder / prefix', 'Folder / prefix')}<input value={draftPrefix} maxLength={1024} disabled={busy} placeholder="uploads/" onChange={event => setDraftPrefix(event.target.value)} /></label><button type="submit" disabled={busy}>{t('Buka', 'Open')}</button>{prefix && <button type="button" disabled={busy} onClick={() => { const parent = prefix.replace(/\/$/, '').split('/').slice(0, -1).join('/'); changePrefix(parent ? `${parent}/` : ''); }}><ArrowLeft size={15} />{t('Ke atas', 'Up')}</button>}</form>
    <div className="storage-filter-bar"><label className="storage-search"><Search size={16} /><input type="search" value={search} placeholder={t('Cari di halaman ini', 'Search this page')} aria-label={t('Cari file di halaman ini', 'Search files on this page')} onChange={event => setSearch(event.target.value)} /></label><div className="storage-view-switch"><button type="button" aria-label={t('Tampilan daftar', 'List view')} aria-pressed={view === 'list'} onClick={() => setView('list')}><List size={17} /></button><button type="button" aria-label={t('Tampilan grid', 'Grid view')} aria-pressed={view === 'grid'} onClick={() => setView('grid')}><LayoutGrid size={17} /></button></div></div>
    {error && <p role="alert" className="storage-error">{error}</p>}{notice && <p role="status" className="storage-notice">{notice}</p>}
    <div className={`storage-file-workspace${selected ? ' has-selection' : ''}`}><div className="storage-file-main">
      {loading ? <p role="status" className="storage-empty">{t('Memuat file…', 'Loading files…')}</p> : listing && <>
        {objects.length + folders.length === 0 && <div className="storage-file-empty"><Folder size={32} /><h3>{search ? t('File tidak ditemukan', 'No matching files') : t('Folder ini masih kosong', 'This folder is empty')}</h3><p>{search ? t('Coba kata lain atau halaman berikutnya.', 'Try another search or the next page.') : t('Upload file pertama Anda untuk mulai mengelola dan membagikan file.', 'Upload your first file to start managing and sharing files.')}</p></div>}
        <ul className={`storage-file-list storage-file-list--${view}`} aria-label={t('Daftar file', 'File listing')}>{folders.map(folder => <li key={`folder:${folder}`}><button className="storage-file-row storage-folder-row" type="button" disabled={busy} onClick={() => changePrefix(folder)}><Folder size={24} /><span><strong>{fileName(folder)}</strong><small>{t('Folder', 'Folder')}</small></span></button></li>)}{objects.map(object => <li key={object.key}><button className="storage-file-row" type="button" aria-pressed={selected?.key === object.key} disabled={busy} onClick={() => setSelected(object)}><ObjectThumbnail connection={connection} object={object} access={access} /><span><strong title={object.key}>{fileName(object.key)}</strong><small>{storageBytes(object.size)}</small></span><time dateTime={object.modified_at}>{new Date(object.modified_at).toLocaleDateString(english ? 'en-US' : 'id-ID')}</time></button></li>)}</ul>
        <div className="storage-pagination"><span className="storage-note">{objects.length + folders.length} {t('item di halaman', 'items on page')} {tokens.length}</span><div className="storage-actions"><button type="button" disabled={busy || tokens.length < 2} onClick={() => { setSelected(null); setTokens(values => values.slice(0, -1)); }}>{t('Sebelumnya', 'Previous')}</button><button type="button" disabled={busy || !listing.next_token} onClick={() => { if (listing.next_token) { setSelected(null); setTokens(values => [...values, listing.next_token]); } }}>{t('Berikutnya', 'Next')}</button></div></div>
      </>}
    </div>{selected && <ObjectDetail key={`${selected.key}:${selected.modified_at}`} connection={connection} object={selected} access={access} english={english} disabled={busy} canDelete={verified} onClose={() => setSelected(null)} onDelete={() => setDeleting(selected.key)} />}</div>
    <ConfirmDangerDialog isOpen={deleting !== null} onClose={() => setDeleting(null)} isLoading={busy} title={t('Hapus file?', 'Delete file?')} description={t('File dihapus dari provider. Riwayat versi mengikuti policy bucket.', 'The file is deleted at the provider. Version history follows bucket policy.')} confirmMatchText={deleting ?? ''} confirmButtonLabel={t('Hapus file', 'Delete file')} onConfirm={() => { if (deleting !== null) void act(async () => { await deleteStorageObject(connection, deleting); if (alive.current) { setDeleting(null); setSelected(null); setRefresh(value => value + 1); setNotice(t('Penghapusan dikonfirmasi provider.', 'Deletion confirmed by the provider.')); } }); }} />
  </section>;
}

const thumbnailSlots: Promise<void>[] = [Promise.resolve(), Promise.resolve(), Promise.resolve()];
let nextThumbnailSlot = 0;
function ObjectThumbnail({ connection, object, access }: { connection: StorageConnection; object: StorageObject; access?: StorageAccess | null }) {
  const [url, setUrl] = useState(''); const element = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    setUrl('');
    if (!imageFile(object.key)) return;
    if (access?.state === 'ready' && access.visibility === 'public') {
      try { setUrl(publicStorageUrl(connection, object.key)); } catch { /* Show the file icon. */ }
      return;
    }
    if (object.size > 8 * 1048576) return;
    const controller = new AbortController(); let blobUrl = ''; let scheduled = false;
    const load = () => {
      if (scheduled) return; scheduled = true;
      const slot = nextThumbnailSlot++ % thumbnailSlots.length;
      thumbnailSlots[slot] = thumbnailSlots[slot].then(async () => {
        if (controller.signal.aborted) return;
        try { const blob = await previewStorageObject(connection, object.key, controller.signal); if (!controller.signal.aborted) { blobUrl = URL.createObjectURL(blob); setUrl(blobUrl); } } catch { /* Individual previews can fall back to an icon. */ }
      });
    };
    const observer = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) { observer?.disconnect(); load(); } });
    if (observer && element.current) observer.observe(element.current); else load();
    return () => { observer?.disconnect(); controller.abort(); if (blobUrl) URL.revokeObjectURL(blobUrl); };
  }, [connection, object.key, object.size, access?.state, access?.visibility]);
  return <span className="storage-file-thumbnail" ref={element}>{url ? <img src={url} alt="" loading="lazy" onError={() => setUrl('')} /> : imageFile(object.key) ? <Image size={24} /> : <File size={24} />}</span>;
}

export function ObjectDetail({ connection, object, access, english, disabled, canDelete, onClose, onDelete }: { connection: StorageConnection; object: StorageObject; access?: StorageAccess | null; english: boolean; disabled: boolean; canDelete: boolean; onClose: () => void; onDelete: () => void }) {
  const t = (id: string, en: string) => english ? en : id;
  const isPublic = access?.state === 'ready' && access.visibility === 'public';
  const [preview, setPreview] = useState(''); const [previewError, setPreviewError] = useState('');
  const [link, setLink] = useState<StorageObjectLink | null>(() => { try { return isPublic ? { kind: 'public', url: publicStorageUrl(connection, object.key), expires_at: null } : null; } catch { return null; } }); const [busy, setBusy] = useState(false); const [error, setError] = useState(''); const [copied, setCopied] = useState(false);
  const [seconds, setSeconds] = useState(3600); const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  useEffect(() => {
    setLink(null); setCopied(false);
    if (!isPublic) return;
    try { setLink({ kind: 'public', url: publicStorageUrl(connection, object.key), expires_at: null }); } catch { setError('Invalid storage URL'); }
  }, [connection, object.key, isPublic]);
  useEffect(() => {
    if (!imageFile(object.key) || object.size > 8 * 1048576) return;
    const controller = new AbortController(); let url = '';
    previewStorageObject(connection, object.key, controller.signal).then(blob => { if (!controller.signal.aborted) { url = URL.createObjectURL(blob); setPreview(url); } }).catch(error => { if (!controller.signal.aborted) setPreviewError(message(error)); });
    return () => { controller.abort(); if (url) URL.revokeObjectURL(url); };
  }, [connection, object.key, object.size]);
  useEffect(() => { if (!link?.expires_at) return; const timer = window.setTimeout(() => setLink(null), Math.max(0, link.expires_at * 1000 - Date.now())); return () => window.clearTimeout(timer); }, [link]);
  const getUrl = async () => {
    setBusy(true); setError(''); setCopied(false);
    try { const result = await fetchStorageObjectLink(connection, object.key, seconds); const url = safeStorageObjectLink(result, connection, object.key); if (alive.current) setLink({ ...result, url }); }
    catch (error) { if (alive.current) setError(message(error)); }
    finally { if (alive.current) setBusy(false); }
  };
  const download = async () => {
    setBusy(true); setError('');
    try { const result = await downloadStorageObject(connection, object.key); const anchor = document.createElement('a'); anchor.href = safeDownload(result, connection); anchor.download = fileName(object.key); anchor.rel = 'noopener noreferrer'; anchor.referrerPolicy = 'no-referrer'; document.body.append(anchor); anchor.click(); anchor.remove(); }
    catch (error) { if (alive.current) setError(message(error)); }
    finally { if (alive.current) setBusy(false); }
  };
  const copy = async (value: string) => {
    try { await navigator.clipboard.writeText(value); if (alive.current) { setCopied(true); setError(''); } }
    catch { if (alive.current) setError(t('Pilih dan salin URL secara manual; clipboard tidak tersedia.', 'Select and copy the URL manually; clipboard is unavailable.')); }
  };
  const label = access === null ? t('Memuat akses…', 'Loading access…') : access && access.state !== 'ready' ? t('Belum terkonfirmasi', 'Unconfirmed') : isPublic ? 'Public' : access?.visibility === 'provider' ? t('Provider', 'Provider') : 'Private';
  const blocked = disabled || busy || access === null || !!access && access.state !== 'ready';
  return <aside className="storage-object-detail" aria-label={t('Detail file', 'File details')}>
    <div className="storage-toolbar"><h3>{t('Detail file', 'File details')}</h3><div className="storage-actions"><button type="button" disabled={disabled || busy} onClick={() => void download()} aria-label={t('Unduh file', 'Download file')} title={t('Unduh file', 'Download file')}><Download size={17} /></button><button type="button" onClick={onClose} aria-label={t('Tutup detail file', 'Close file details')}><X size={17} /></button></div></div>
    <div className="storage-preview">{preview ? <img src={preview} alt={fileName(object.key)} /> : <><File size={42} /><span>{imageFile(object.key) && object.size <= 8 * 1048576 && !previewError ? t('Memuat preview...', 'Loading preview...') : t('Preview tidak tersedia', 'Preview unavailable')}</span></>}</div>{previewError && <p className="storage-note">{previewError}</p>}
    <h3 className="storage-object-title">{fileName(object.key)}</h3><span className={`storage-access-badge ${isPublic ? 'is-public' : ''}`}>{isPublic ? <Globe2 size={12} /> : <LockKeyhole size={12} />}{label}</span>
    <dl className="storage-file-properties"><div><dt>{t('Ukuran', 'Size')}</dt><dd>{storageBytes(object.size)}</dd></div><div><dt>{t('Terakhir diubah', 'Last modified')}</dt><dd>{new Date(object.modified_at).toLocaleString(english ? 'en-US' : 'id-ID')}</dd></div><div><dt>Bucket</dt><dd>{connection.bucket}</dd></div><div><dt>Object key</dt><dd><code>{object.key}</code><button type="button" aria-label={t('Salin object key', 'Copy object key')} disabled={disabled || busy} onClick={() => void copy(object.key)}><Copy size={14} /></button></dd></div></dl>
    <div className="storage-object-url"><label htmlFor="storage-object-url">{isPublic ? t('URL publik', 'Public URL') : t('URL akses sementara', 'Temporary access URL')}</label><div className="storage-url-field"><input id="storage-object-url" readOnly value={link?.url ?? ''} placeholder={isPublic ? t('Memuat URL...', 'Loading URL...') : t('Buat signed URL untuk membagikan file', 'Create a signed URL to share the file')} onFocus={event => event.currentTarget.select()} /><button type="button" disabled={!link || blocked} onClick={() => link && void copy(link.url)} aria-label={t('Salin URL', 'Copy URL')} title={t('Salin URL', 'Copy URL')}>{copied ? <Check size={16} /> : <Copy size={16} />}</button>{link && <a href={link.url} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer" aria-label={t('Buka URL', 'Open URL')} title={t('Buka URL', 'Open URL')}><ExternalLink size={16} /></a>}</div>
      {isPublic ? <p className="storage-note">{t('URL tetap, tanpa login atau tanggal kedaluwarsa. Akses tersedia selama bucket Public dan file masih ada.', 'A stable URL with no login or expiry. Access remains available while the bucket is Public and the file exists.')}</p> : <><div className="storage-signed-controls"><label>{t('Masa berlaku', 'Expires in')}<select value={seconds} disabled={blocked} onChange={event => setSeconds(Number(event.target.value))}><option value={60}>{t('1 menit', '1 minute')}</option><option value={3600}>{t('1 jam', '1 hour')}</option><option value={86400}>{t('24 jam', '24 hours')}</option></select></label><button type="button" className="button-primary" disabled={blocked} onClick={() => void getUrl()}>{busy ? t('Menyiapkan...', 'Preparing...') : t('Dapatkan URL', 'Get URL')}</button></div><p className="storage-note">{t('Siapa pun yang memiliki signed URL dapat membuka file selama masa berlaku, tanpa login panel.', 'Anyone with the signed URL can open the file until it expires, without signing in to the panel.')}</p>{link?.expires_at && <p className="storage-note">{t('Berlaku hingga', 'Expires at')} {new Date(link.expires_at * 1000).toLocaleString()}</p>}</>}
      {copied && <p role="status" className="storage-note">{t('Disalin ke clipboard.', 'Copied to clipboard.')}</p>}
    </div>{error && <p role="alert" className="storage-error">{error}</p>}
    <button type="button" className="storage-delete-button" disabled={disabled || busy || !canDelete} onClick={onDelete}><Trash2 size={15} />{t('Hapus file', 'Delete file')}</button>
  </aside>;
}
