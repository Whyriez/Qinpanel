import { useEffect, useRef, useState, type FormEvent } from 'react';
import { ArrowLeft, Check, ChevronRight, Copy, Download, ExternalLink, Folder, Globe2, LayoutGrid, List, LockKeyhole, RefreshCw, Search, SlidersHorizontal, Trash2, Upload, X } from 'lucide-react';
import { ConfirmDangerDialog } from '../ui/ConfirmDangerDialog';
import { ModalOverlay } from '../ui/ModalOverlay';
import { StorageFileIcon, storageFileKind, storageFileType, type StorageFileKind } from './FilePresentation';
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
  const [selected, setSelected] = useState<StorageObject | null>(null); const [view, setView] = useState<'list' | 'grid'>('grid'); const [search, setSearch] = useState('');
  const [kind, setKind] = useState<StorageFileKind | 'all'>('all'); const [sort, setSort] = useState('name');
  const [showLocation, setShowLocation] = useState(false); const [dragging, setDragging] = useState(false);
  const [showUpload, setShowUpload] = useState(false); const [file, setFile] = useState<globalThis.File | null>(null); const [key, setKey] = useState(''); const [overwrite, setOverwrite] = useState(false); const [progress, setProgress] = useState<number | null>(null);
  const upload = useRef<AbortController | null>(null); const fileInput = useRef<HTMLInputElement>(null); const alive = useRef(true);
  const uploadedSelection = useRef<StorageObject | null>(null);
  useEffect(() => { alive.current = true; return () => { alive.current = false; upload.current?.abort(); onBusy(false); }; }, [onBusy]);
  useEffect(() => {
    const controller = new AbortController(); setLoading(true); setListing(null); setError(''); setVerified(false);
    fetchStorageObjects(connection, prefix, tokens[tokens.length - 1], controller.signal).then(value => {
      if (!controller.signal.aborted) { setListing(value.listing); setVerified(true); setSelected(previous => previous ? value.listing.objects.find(object => object.key === previous.key) ?? (uploadedSelection.current?.key === previous.key ? uploadedSelection.current : null) : null); }
    }).catch(error => { if (!controller.signal.aborted) setError(uploadedSelection.current ? t('Upload sudah berhasil, tetapi daftar folder belum dapat dimuat. Muat ulang file untuk mencoba lagi.', 'Upload succeeded, but the folder listing could not be loaded. Refresh files to retry.') : message(error)); }).finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [connection, prefix, tokens, refresh]);
  const changePrefix = (value: string) => { uploadedSelection.current = null; setPrefix(value); setDraftPrefix(value); setTokens([null]); setSelected(null); setSearch(''); setNotice(''); };
  const chooseFile = (next: globalThis.File | null) => {
    if (busy || !verified) return;
    setFile(next);
    const folder = prefix.endsWith('/') ? prefix : prefix.slice(0, prefix.lastIndexOf('/') + 1);
    setKey(next ? folder + next.name : '');
  };
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
        uploadedSelection.current = { key: uploadedKey, size: uploaded.size, modified_at: new Date().toISOString() };
        setSelected(uploadedSelection.current);
        setFile(null); setKey(''); setOverwrite(false); setShowUpload(false); if (fileInput.current) fileInput.current.value = '';
        // Stay in the containing folder. The key is a file, not a folder/prefix.
        // Keep its confirmed upload details if it falls outside the first page.
        const folder = uploadedKey.slice(0, uploadedKey.lastIndexOf('/') + 1);
        setPrefix(folder); setDraftPrefix(folder); setTokens([null]); setSearch(''); setRefresh(value => value + 1);
      }
    });
  };
  const objects = (listing?.objects.filter(object => object.key.toLowerCase().includes(search.toLowerCase()) && (kind === 'all' || storageFileKind(object.key) === kind)) ?? []).sort((a, b) => sort === 'newest' ? Date.parse(b.modified_at) - Date.parse(a.modified_at) : sort === 'size' ? b.size - a.size : (sort === 'name-desc' ? -1 : 1) * a.key.localeCompare(b.key, english ? 'en' : 'id', { numeric: true }));
  const folders = listing?.prefixes.filter(value => value.toLowerCase().includes(search.toLowerCase())).sort((a,b) => a.localeCompare(b)) ?? [];
  const segments = prefix.replace(/\/$/, '').split('/').filter(Boolean);
  return <section className="storage-explorer" aria-label={t('File dalam bucket', 'Bucket files')}>
    <div className="storage-explorer-toolbar">
      <nav className="storage-breadcrumb" aria-label={t('Lokasi folder', 'Folder location')}>
        {prefix && <button type="button" className="storage-icon-button" disabled={busy} aria-label={t('Ke atas', 'Up')} title={t('Ke atas', 'Up')} onClick={() => { const parent = prefix.replace(/\/$/, '').split('/').slice(0, -1).join('/'); changePrefix(parent ? `${parent}/` : ''); }}><ArrowLeft size={16} /></button>}
        <button type="button" disabled={busy} onClick={() => changePrefix('')}><Folder size={16} />{connection.bucket}</button>
        {segments.length ? segments.map((part, index) => <span key={`${index}:${part}`} className="storage-crumb"><ChevronRight size={13} /><button type="button" disabled={busy} aria-current={index === segments.length-1 ? 'location' : undefined} onClick={() => changePrefix(segments.slice(0,index+1).join('/')+'/')}>{part}</button></span>) : <span className="storage-crumb"><ChevronRight size={13} /><span>{t('Semua file', 'All files')}</span></span>}
      </nav>
      <div className="storage-actions"><button type="button" className="storage-icon-button" disabled={busy || loading} aria-label={t('Muat ulang file', 'Refresh files')} title={t('Muat ulang file', 'Refresh files')} onClick={() => { uploadedSelection.current = null; setRefresh(value => value + 1); }}><RefreshCw size={16} /></button><button type="button" className="button-primary" disabled={busy || !verified} aria-expanded={showUpload} onClick={() => setShowUpload(true)}><Upload size={16} />{t('Upload file', 'Upload file')}</button></div>
    </div>
    {showUpload && <ModalOverlay className="storage-upload-overlay" aria-label={t('Upload ke bucket', 'Upload to bucket')} onDismiss={() => setShowUpload(false)} dismissDisabled={busy} onClick={event => { if (event.target === event.currentTarget && !busy) setShowUpload(false); }}><section className="storage-upload storage-upload-card dialog-panel">
      <div className="storage-toolbar"><div><h3>{t('Upload file', 'Upload file')}</h3><p className="storage-note">{t('Tujuan', 'Destination')}: {connection.bucket}/{prefix}</p></div><button type="button" disabled={busy} onClick={() => setShowUpload(false)} aria-label={t('Tutup upload', 'Close upload')}><X size={18} /></button></div>
      <form onSubmit={send}>
        <label className={`storage-file-picker${dragging ? ' is-dragging' : ''}`} onDragOver={event => { event.preventDefault(); if (!busy) setDragging(true); }} onDragLeave={() => setDragging(false)} onDrop={event => { event.preventDefault(); setDragging(false); chooseFile(event.dataTransfer.files[0] ?? null); }}>
          <span className="storage-upload-symbol"><Upload size={24} /></span><strong>{file ? file.name : t('Tarik file ke sini', 'Drop a file here')}</strong><span>{file ? storageBytes(file.size) : t('atau klik untuk memilih dari perangkat', 'or click to choose from your device')}</span><small>{t('Maksimum 32 MiB; rules bucket dapat membatasi lebih kecil.', 'Maximum 32 MiB; bucket rules may impose a smaller limit.')}</small>
          <input ref={fileInput} type="file" disabled={busy || !verified} onChange={event => chooseFile(event.target.files?.[0] ?? null)} />
        </label>
        <label>Object key<input required maxLength={1024} disabled={busy || !verified} value={key} placeholder="uploads/photo.png" onChange={event => setKey(event.target.value)} /></label>
        <label className="storage-check"><input type="checkbox" checked={overwrite} disabled={busy || !verified} onChange={event => setOverwrite(event.target.checked)} />{t('Izinkan menimpa object dengan nama sama', 'Allow overwriting an object with the same key')}</label>
        {!conditional && <p className="storage-note">{t('Provider ini memerlukan persetujuan penimpaan. Gunakan nama unik untuk menyimpan file sebagai object baru.', 'This provider requires overwrite consent. Use a unique name to store a new object.')}</p>}
        {file && file.size > 33554432 && <p role="alert" className="storage-error">{t('File melebihi 32 MiB.', 'File exceeds 32 MiB.')}</p>}
        {progress !== null && <div role="status"><progress max={100} value={progress} aria-label={t('File dikirim ke panel', 'File sent to panel')} /><p className="storage-note">{progress < 100 ? `${progress}%` : t('Menunggu konfirmasi provider…', 'Waiting for provider confirmation…')}</p><button type="button" onClick={() => upload.current?.abort()}>{t('Batalkan transfer', 'Cancel transfer')}</button></div>}
        {error && <p role="alert" className="storage-error">{error}</p>}
        <div className="storage-upload-footer"><span className="storage-note">{t('Batas dan akses mengikuti pengaturan bucket.', 'Limits and access follow the bucket settings.')}</span><button type="submit" className="button-primary" disabled={busy || !verified || !file || file.size > 33554432 || !validObjectKey(key) || !conditional && !overwrite}><Upload size={15} />{t('Mulai upload', 'Start upload')}</button></div>
      </form>
    </section></ModalOverlay>}
    <div className="storage-filter-bar">
      <label className="storage-search"><Search size={16} /><input type="search" value={search} placeholder={t('Cari di halaman ini', 'Search this page')} aria-label={t('Cari file di halaman ini', 'Search files on this page')} onChange={event => setSearch(event.target.value)} /></label>
      <div className="storage-filter-controls"><select aria-label={t('Jenis file di halaman ini', 'File type on this page')} value={kind} onChange={event => setKind(event.target.value as StorageFileKind | 'all')}><option value="all">{t('Semua jenis', 'All types')}</option>{(['image','document','video','audio','archive','code','other'] as const).map((value,index) => <option key={value} value={value}>{(english ? ['Images','Documents','Videos','Audio','Archives','Code','Other'] : ['Gambar','Dokumen','Video','Audio','Arsip','Kode','Lainnya'])[index]}</option>)}</select>
        <select aria-label={t('Urutkan file di halaman ini', 'Sort files on this page')} value={sort} onChange={event => setSort(event.target.value)}><option value="name">{t('Nama A–Z', 'Name A–Z')}</option><option value="name-desc">{t('Nama Z–A', 'Name Z–A')}</option><option value="newest">{t('Terbaru', 'Newest')}</option><option value="size">{t('Ukuran terbesar', 'Largest first')}</option></select>
        <button type="button" className="storage-icon-button" aria-label={t('Buka lokasi folder', 'Open folder location')} title={t('Buka lokasi folder', 'Open folder location')} aria-expanded={showLocation} onClick={() => setShowLocation(value => !value)}><SlidersHorizontal size={16} /></button>
        <div className="storage-view-switch" role="group" aria-label={t('Tampilan file', 'File view')}><button type="button" aria-label={t('Tampilan daftar', 'List view')} title={t('Tampilan daftar', 'List view')} aria-pressed={view === 'list'} onClick={() => setView('list')}><List size={16} /></button><button type="button" aria-label={t('Tampilan grid', 'Grid view')} title={t('Tampilan grid', 'Grid view')} aria-pressed={view === 'grid'} onClick={() => setView('grid')}><LayoutGrid size={16} /></button></div>
      </div>
    </div>
    {showLocation && <form className="storage-prefix" onSubmit={event => { event.preventDefault(); changePrefix(draftPrefix); setShowLocation(false); }}><label>{t('Folder / prefix', 'Folder / prefix')}<input value={draftPrefix} maxLength={1024} disabled={busy} placeholder="uploads/" onChange={event => setDraftPrefix(event.target.value)} /></label><button type="submit" disabled={busy}>{t('Buka', 'Open')}</button></form>}
    {error && !showUpload && !deleting && <p role="alert" className="storage-error">{error}</p>}{notice && <p role="status" className="storage-notice"><Check size={15} />{notice}</p>}
    <div className={`storage-file-workspace${selected ? ' has-selection' : ''}`}><div className="storage-file-main">
      {loading ? <div role="status" className="storage-loading"><span className="storage-note">{t('Memuat file…', 'Loading files…')}</span><div className="storage-file-skeleton" aria-hidden="true">{[1,2,3,4,5,6].map(value => <span key={value} />)}</div></div> : listing && <>
        {objects.length + folders.length === 0 && <div className="storage-file-empty"><span className="storage-empty-symbol"><Folder size={28} /></span><h3>{search || kind !== 'all' ? t('Tidak ada file yang cocok', 'No matching files') : t('Simpan file pertama Anda', 'Store your first file')}</h3><p>{search || kind !== 'all' ? t('Coba pencarian atau jenis file lain pada halaman ini.', 'Try another search or file type on this page.') : t('Gambar, dokumen, video, dan file lainnya. Semua tersimpan dalam satu tempat.', 'Images, documents, videos and other files. Everything in one place.')}</p>{!search && kind === 'all' && <button type="button" disabled={busy || !verified} onClick={() => setShowUpload(true)}><Upload size={15} />{t('Pilih file untuk upload', 'Choose a file to upload')}</button>}</div>}
        {view === 'list' && objects.length + folders.length > 0 && <div className="storage-list-heading" aria-hidden="true"><span>{t('Nama', 'Name')}</span><span>{t('Jenis', 'Type')}</span><span>{t('Ukuran', 'Size')}</span><span>{t('Diubah', 'Modified')}</span><span /></div>}
        <ul className={`storage-file-list storage-file-list--${view}`} aria-label={t('Daftar file', 'File listing')}>
          {folders.map(folder => <li key={`folder:${folder}`}><button className="storage-file-row storage-folder-row" type="button" disabled={busy} onClick={() => changePrefix(folder)}><span className="storage-file-name"><span className="storage-folder-symbol"><Folder size={24} /></span><span><strong>{fileName(folder)}</strong><small>{t('Folder', 'Folder')}</small></span></span><span className="storage-row-type">{t('Folder', 'Folder')}</span><span className="storage-row-size">—</span><span className="storage-row-date">—</span><ChevronRight size={15} className="storage-row-arrow" /></button></li>)}
          {objects.map(object => <li key={object.key}><button className={`storage-file-row storage-file-row--${storageFileKind(object.key)}`} type="button" aria-pressed={selected?.key === object.key} disabled={busy} onClick={() => setSelected(object)}><span className="storage-file-name"><ObjectThumbnail connection={connection} object={object} access={access} /><span><strong title={object.key}>{fileName(object.key)}</strong><small>{storageFileType(object.key)}<span> · </span>{storageBytes(object.size)}</small></span></span><span className="storage-row-type">{storageFileType(object.key)}</span><span className="storage-row-size">{storageBytes(object.size)}</span><time className="storage-row-date" dateTime={object.modified_at}>{new Date(object.modified_at).toLocaleDateString(english ? 'en-US' : 'id-ID', { day:'numeric', month:'short' })}</time><ChevronRight size={15} className="storage-row-arrow" /></button></li>)}
        </ul>
        <div className="storage-pagination"><span className="storage-note">{objects.length + folders.length} {t('item', 'items')}<span> · </span>{t('Halaman', 'Page')} {tokens.length}</span><div className="storage-actions"><button type="button" disabled={busy || tokens.length < 2} onClick={() => { setSelected(null); setTokens(values => values.slice(0, -1)); }}>{t('Sebelumnya', 'Previous')}</button><button type="button" disabled={busy || !listing.next_token} onClick={() => { if (listing.next_token) { setSelected(null); setTokens(values => [...values, listing.next_token]); } }}>{t('Berikutnya', 'Next')}</button></div></div>
      </>}
    </div>{selected && <ObjectDetail key={`${selected.key}:${selected.modified_at}`} connection={connection} object={selected} access={access} english={english} disabled={busy} canDelete={verified} onClose={() => { uploadedSelection.current = null; setSelected(null); }} onDelete={() => setDeleting(selected.key)} />}</div>
    <ConfirmDangerDialog errorMessage={deleting ? error : undefined} isOpen={deleting !== null} onClose={() => setDeleting(null)} isLoading={busy} title={t('Hapus file?', 'Delete file?')} description={t('File dihapus dari provider. Riwayat versi mengikuti policy bucket.', 'The file is deleted at the provider. Version history follows bucket policy.')} confirmMatchText={deleting ?? ''} confirmButtonLabel={t('Hapus file', 'Delete file')} onConfirm={() => { if (deleting !== null) void act(async () => { await deleteStorageObject(connection, deleting); if (alive.current) { uploadedSelection.current = null; setDeleting(null); setSelected(null); setRefresh(value => value + 1); setNotice(t('File berhasil dihapus.', 'File deleted successfully.')); } }); }} />
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
  return <span className="storage-file-thumbnail" ref={element}>{url ? <img src={url} alt="" loading="lazy" onError={() => setUrl('')} /> : <StorageFileIcon name={object.key} />}</span>;
}

export function ObjectDetail({ connection, object, access, english, disabled, canDelete, onClose, onDelete }: { connection: StorageConnection; object: StorageObject; access?: StorageAccess | null; english: boolean; disabled: boolean; canDelete: boolean; onClose: () => void; onDelete: () => void }) {
  const t = (id: string, en: string) => english ? en : id;
  const isPublic = access?.state === 'ready' && access.visibility === 'public';
  const [compact, setCompact] = useState(false);
  useEffect(() => {
    const query = window.matchMedia('(max-width: 1100px)');
    const update = () => setCompact(query.matches);
    update(); query.addEventListener('change', update);
    return () => query.removeEventListener('change', update);
  }, []);
  const [preview, setPreview] = useState(''); const [previewError, setPreviewError] = useState('');
  const [link, setLink] = useState<StorageObjectLink | null>(() => { try { return isPublic ? { kind: 'public', url: publicStorageUrl(connection, object.key), expires_at: null } : null; } catch { return null; } }); const [busy, setBusy] = useState(false); const [error, setError] = useState(''); const [copied, setCopied] = useState<'url' | 'key' | null>(null);
  const [seconds, setSeconds] = useState(3600); const alive = useRef(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  useEffect(() => {
    setLink(null); setCopied(null);
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
    setBusy(true); setError(''); setCopied(null);
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
  const copy = async (value: string, target: 'url' | 'key') => {
    try { await navigator.clipboard.writeText(value); if (alive.current) { setCopied(target); setError(''); } }
    catch { if (alive.current) setError(t('Pilih dan salin URL secara manual; clipboard tidak tersedia.', 'Select and copy the URL manually; clipboard is unavailable.')); }
  };
  const label = access === null ? t('Memuat akses…', 'Loading access…') : access && access.state !== 'ready' ? t('Belum terkonfirmasi', 'Unconfirmed') : isPublic ? 'Public' : access?.visibility === 'provider' ? t('Provider', 'Provider') : 'Private';
  const blocked = disabled || busy || access === null || !!access && access.state !== 'ready';
  const detail = <aside className="storage-object-detail" aria-label={t('Detail file', 'File details')}>
    <div className="storage-detail-heading"><h3>{t('Detail file', 'File details')}</h3><div className="storage-actions"><button type="button" disabled={disabled || busy} onClick={() => void download()} aria-label={t('Unduh file', 'Download file')} title={t('Unduh file', 'Download file')}><Download size={17} /></button><button type="button" disabled={disabled || busy} onClick={onClose} aria-label={t('Tutup detail file', 'Close file details')} title={t('Tutup detail file', 'Close file details')}><X size={17} /></button></div></div>
    <div className="storage-detail-body">
      <div className={`storage-preview${!imageFile(object.key) ? ' storage-preview--file' : ''}`}>{preview ? <img src={preview} alt={fileName(object.key)} /> : <><StorageFileIcon name={object.key} size={48} /><span>{imageFile(object.key) && object.size <= 8 * 1048576 && !previewError ? t('Memuat preview…', 'Loading preview…') : t('Preview tidak tersedia', 'Preview unavailable')}</span></>}</div>{previewError && <p className="storage-note">{t('Preview tidak tersedia. Anda tetap dapat membuka URL atau mengunduh file.', 'Preview unavailable. You can still open the URL or download the file.')}</p>}
      <h3 className="storage-object-title">{fileName(object.key)}</h3><div className="storage-detail-meta"><span className={`storage-access-badge ${isPublic ? 'is-public' : ''}`}>{isPublic ? <Globe2 size={12} /> : <LockKeyhole size={12} />}{label}</span><span>{storageFileType(object.key)} · {storageBytes(object.size)}</span></div>
      <section className="storage-object-url"><label htmlFor="storage-object-url">{isPublic ? t('URL publik', 'Public URL') : t('URL akses sementara', 'Temporary access URL')}</label>
        <div className="storage-url-field"><textarea id="storage-object-url" readOnly rows={3} spellCheck={false} value={link?.url ?? ''} placeholder={isPublic ? t('Memuat URL…', 'Loading URL…') : t('Buat signed URL untuk membagikan file', 'Create a signed URL to share the file')} onFocus={event => event.currentTarget.select()} /></div>
        {link && <div className="storage-link-actions"><button type="button" disabled={blocked} onClick={() => void copy(link.url, 'url')} aria-label={t('Salin URL', 'Copy URL')}>{copied === 'url' ? <Check size={15} /> : <Copy size={15} />}{copied === 'url' ? t('Disalin', 'Copied') : t('Salin URL', 'Copy URL')}</button><a href={link.url} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer" aria-label={t('Buka URL', 'Open URL')} title={t('Buka URL', 'Open URL')}><ExternalLink size={15} />{t('Buka', 'Open')}</a></div>}
        {isPublic ? <p className="storage-note">{t('Dapat dibuka siapa pun tanpa login, selama bucket Public dan file masih ada.', 'Anyone can open this without signing in while the bucket is Public and the file exists.')}</p> : <><div className="storage-signed-controls"><label>{t('Masa berlaku', 'Expires in')}<select value={seconds} disabled={blocked} onChange={event => setSeconds(Number(event.target.value))}><option value={60}>{t('1 menit', '1 minute')}</option><option value={3600}>{t('1 jam', '1 hour')}</option><option value={86400}>{t('24 jam', '24 hours')}</option></select></label><button type="button" className="button-primary" disabled={blocked} onClick={() => void getUrl()}>{busy ? t('Menyiapkan…', 'Preparing…') : t('Dapatkan URL', 'Get URL')}</button></div><p className="storage-note">{t('Siapa pun yang memiliki signed URL dapat membuka file selama masa berlaku, tanpa login panel.', 'Anyone with the signed URL can open the file until it expires, without signing in to the panel.')}</p>{link?.expires_at && <p className="storage-link-expiry">{t('Berlaku hingga', 'Expires at')} {new Date(link.expires_at * 1000).toLocaleString()}</p>}</>}
        {copied && <p role="status" className="storage-note">{copied === 'url' ? t('URL disalin ke clipboard.', 'URL copied to clipboard.') : t('Object key disalin ke clipboard.', 'Object key copied to clipboard.')}</p>}
      </section>
      <section className="storage-file-information"><h4>{t('Informasi file', 'File information')}</h4><dl className="storage-file-properties"><div><dt>{t('Terakhir diubah', 'Last modified')}</dt><dd>{new Date(object.modified_at).toLocaleString(english ? 'en-US' : 'id-ID')}</dd></div><div><dt>Bucket</dt><dd>{connection.bucket}</dd></div><div><dt>Object key</dt><dd><code>{object.key}</code><button type="button" aria-label={t('Salin object key', 'Copy object key')} disabled={disabled || busy} onClick={() => void copy(object.key, 'key')}><Copy size={14} /></button></dd></div></dl></section>
      {error && <p role="alert" className="storage-error">{error}</p>}
      <div className="storage-detail-danger"><button type="button" className="storage-delete-button" disabled={disabled || busy || !canDelete} onClick={onDelete}><Trash2 size={15} />{t('Hapus file', 'Delete file')}</button></div>
    </div>
  </aside>;
  return compact ? <ModalOverlay className="storage-detail-overlay" variant="sheet" aria-label={t('Detail file', 'File details')} onDismiss={onClose} dismissDisabled={disabled || busy} onClick={event => { if (event.target === event.currentTarget && !disabled && !busy) onClose(); }}>{detail}</ModalOverlay> : detail;
}
