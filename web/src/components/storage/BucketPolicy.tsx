import { useEffect, useId, useState, type FormEvent } from 'react';
import { Download, LockKeyhole, RefreshCw, ShieldCheck, Trash2, Upload } from 'lucide-react';
import { fetchStoragePolicy, measureStorageUsage, saveStorageAllocation, saveStorageRules, type StorageConnection, type StoragePolicy, type StorageRules } from '../../api/storage';

export const storageBytes = (value: number | null) => {
  if (value === null) return '—';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  const index = Math.min(4, Math.max(0, Math.floor(Math.log2(Math.max(value, 1)) / 10)));
  return `${(value / 1024 ** index).toLocaleString(undefined, { maximumFractionDigits: index ? 1 : 0 })} ${units[index]}`;
};
const message = (error: unknown) => error instanceof Error ? error.message : 'Storage request failed';

export function BucketPolicy({ connection, admin, english, locked }: { connection: StorageConnection; admin: boolean; english: boolean; locked: boolean }) {
  const t = (id: string, en: string) => english ? en : id;
  const [policy, setPolicy] = useState<StoragePolicy | null>(null);
  const [error, setError] = useState(''); const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false); const [reload, setReload] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    fetchStoragePolicy(connection.id, controller.signal).then(value => { if (!controller.signal.aborted) { setPolicy(value); setError(''); } }).catch(error => { if (!controller.signal.aborted) setError(message(error)); });
    return () => controller.abort();
  }, [connection.id, reload]);
  const act = async (action: () => Promise<StoragePolicy>) => {
    setBusy(true); setError(''); setNotice('');
    try { setPolicy(await action()); setNotice(t('Pengaturan diperbarui.', 'Settings updated.')); }
    catch (error) { setError(message(error)); }
    finally { setBusy(false); }
  };
  return <section className="storage-policy" aria-label={t('Alokasi dan keamanan bucket', 'Bucket allocation and security')}>
    <div className="storage-policy-heading"><div><h3><ShieldCheck size={17} />{admin ? t('Alokasi tenant', 'Tenant allocation') : t('Rules transfer panel', 'Panel transfer rules')}</h3><p className="storage-note">{t('Kelola batas transfer melalui panel. Akses URL mengikuti pengaturan bucket.', 'Manage panel transfer limits. URL access follows the bucket settings.')}</p></div><button type="button" disabled={busy || locked} onClick={() => setReload(value => value + 1)} aria-label={t('Muat ulang alokasi dan rules', 'Refresh allocation and rules')}><RefreshCw size={16} /></button></div>
    {error && <p role="alert" className="storage-error">{error}</p>}{notice && <p role="status" className="storage-note">{notice}</p>}
    {!policy ? <p role="status" className="storage-note">{t('Memuat alokasi…', 'Loading allocation…')}</p> : <>
      <div className="storage-metrics"><div><span>{t('Pemakaian terukur', 'Measured usage')}</span><strong>{storageBytes(policy.used_bytes)}</strong></div><div><span>{t('Alokasi panel', 'Panel allowance')}</span><strong>{policy.quota_bytes === null ? t('Belum dibatasi', 'Not limited') : storageBytes(policy.quota_bytes)}</strong></div><div><span>Objects</span><strong>{policy.object_count === null ? '—' : policy.object_count.toLocaleString()}</strong></div></div>
      {policy.quota_bytes !== null && policy.used_bytes !== null && <progress className="storage-quota-meter" max={policy.quota_bytes} value={Math.min(policy.quota_bytes, policy.used_bytes)} aria-label={t('Pemakaian alokasi', 'Allowance usage')} />}
      <p className="storage-note">{policy.measured_at ? `${t('Diukur', 'Measured')} ${new Date(policy.measured_at * 1000).toLocaleString(english ? 'en-US' : 'id-ID')}` : t('Belum diukur. Admin dapat memperbarui total tanpa membuka daftar file.', 'Not measured yet. An administrator can refresh totals without opening file listings.')}</p>
      {admin ? <AllocationEditor key={policy.revision} english={english} policy={policy} disabled={busy || locked} onSave={quota => act(() => saveStorageAllocation(connection.id, policy, quota))} onMeasure={() => act(() => measureStorageUsage(connection.id))} /> : <RulesEditor key={policy.revision} english={english} policy={policy} disabled={busy || locked} onSave={rules => act(() => saveStorageRules(connection.id, policy, rules))} />}
    </>}
    <p className="storage-note storage-rule-scope"><LockKeyhole size={14} />{t('Rules dan alokasi membatasi transfer melalui panel. SDK S3 langsung mengikuti policy provider dan dapat melewati batas panel. Alokasi bukan reservasi disk atau kuota keras engine.', 'Rules and allowances limit panel transfers. Direct S3 SDK access follows provider policies and can bypass panel limits. Allowances do not reserve disk or impose an engine hard quota.')}</p>
  </section>;
}

export function AllocationEditor({ english, policy, disabled, onSave, onMeasure }: { english: boolean; policy: StoragePolicy; disabled: boolean; onSave: (quota: number | null) => void; onMeasure: () => void }) {
  const t = (id: string, en: string) => english ? en : id;
  const [quota, setQuota] = useState(policy.quota_bytes === null ? '' : String(policy.quota_bytes / 1073741824));
  const value = quota.trim() ? Math.round(Number(quota) * 1073741824) : null;
  const invalid = value !== null && (!Number.isSafeInteger(value) || value <= 0);
  return <form className="storage-allocation-form" onSubmit={event => { event.preventDefault(); if (!invalid) onSave(value); }}><label>{t('Alokasi bucket (GiB)', 'Bucket allowance (GiB)')}<input type="number" min="0.000001" step="any" value={quota} disabled={disabled} placeholder={t('Tanpa batas panel', 'No panel limit')} onChange={event => setQuota(event.target.value)} /></label><div className="storage-actions"><button type="submit" disabled={disabled || invalid || value === policy.quota_bytes}>{t('Simpan alokasi', 'Save allowance')}</button><button type="button" disabled={disabled} onClick={onMeasure}><RefreshCw size={15} />{t('Ukur pemakaian', 'Measure usage')}</button></div><p className="storage-note">{t('Kosongkan untuk menghapus batas panel. Pengukuran hanya menampilkan jumlah dan ukuran total object; nama dan isi file tetap tersembunyi.', 'Leave empty to remove the panel limit. Measurements show only object count and total size; file names and contents remain hidden.')}</p></form>;
}

export function RulesEditor({ english, policy, disabled, onSave }: { english: boolean; policy: StoragePolicy; disabled: boolean; onSave: (rules: StorageRules) => void }) {
  const sizeId = useId();
  const t = (id: string, en: string) => english ? en : id;
  const [rules, setRules] = useState(policy.rules); const [extensions, setExtensions] = useState(policy.rules.extensions.join(', '));
  const normalized = [...new Set(extensions.split(',').map(value => value.trim().toLowerCase().replace(/^\./, '')).filter(Boolean))];
  const valid = Number.isSafeInteger(rules.max_file_bytes) && rules.max_file_bytes > 0 && rules.max_file_bytes <= 33554432 && normalized.length <= 32 && normalized.every(value => /^[a-z0-9]{1,16}$/.test(value));
  const changed = JSON.stringify({ ...rules, extensions: normalized }) !== JSON.stringify(policy.rules);
  const reset = () => { setRules(policy.rules); setExtensions(policy.rules.extensions.join(', ')); };
  const submit = (event: FormEvent) => { event.preventDefault(); if (valid && changed && !disabled) onSave({ ...rules, extensions: normalized }); };
  const operations = [
    { action: 'read' as const, Icon: Download, label: t('Baca & unduh', 'Read & download'), description: t('Buka preview, buat URL, dan unduh melalui panel.', 'Preview, generate URLs and download through the panel.') },
    { action: 'write' as const, Icon: Upload, label: t('Upload', 'Upload'), description: t('Unggah file dan buat folder baru.', 'Upload files and create new folders.') },
    { action: 'delete' as const, Icon: Trash2, label: t('Hapus', 'Delete'), description: t('Hapus file dari bucket melalui panel.', 'Delete bucket files through the panel.') },
  ];
  return <form className="storage-rules-editor storage-rules-form" onSubmit={submit} aria-label={t('Atur rules bucket', 'Configure bucket rules')}>
    <div className="storage-section-heading"><span className="storage-section-icon"><ShieldCheck size={20} /></span><div><h4>{t('Atur rules bucket', 'Configure bucket rules')}</h4><p className="storage-note">{t('Atur operasi pemilik melalui panel. Akses baca publik mengikuti pilihan akses bucket di atas.', 'Configure owner operations through the panel. Anonymous read follows the bucket access setting above.')}</p></div></div>
    <fieldset disabled={disabled} className="storage-rules-fieldset"><legend>{t('Izin operasi', 'Operation permissions')}</legend><div className="storage-permission-grid">{operations.map(({ action, Icon, label, description }) => <label className={`storage-permission-card${rules[action] ? ' is-enabled' : ''}`} key={action}><span className="storage-permission-label"><Icon size={17} /><strong>{label}</strong><input type="checkbox" role="switch" aria-label={label} checked={rules[action]} onChange={event => setRules(value => ({ ...value, [action]: event.target.checked }))} /></span><span className="storage-note">{description}</span></label>)}</div></fieldset>
    <fieldset disabled={disabled} className="storage-rules-fieldset"><legend>{t('Batas upload', 'Upload limits')}</legend><div className="storage-rule-limits"><div><label htmlFor={sizeId}>{t('Maksimum file (MiB)', 'Maximum file size (MiB)')}</label><span className="storage-number-field"><input id={sizeId} aria-describedby={`${sizeId}-hint`} type="number" min="0.000001" max="32" step="any" required value={rules.max_file_bytes / 1048576} onChange={event => setRules(value => ({ ...value, max_file_bytes: Math.round(Number(event.target.value) * 1048576) }))} /><span aria-hidden="true">MiB</span></span><small id={`${sizeId}-hint`} className="storage-note">{t('Batas panel hingga 32 MiB per file.', 'Panel limit up to 32 MiB per file.')}</small></div><div><label>{t('Ekstensi diizinkan', 'Allowed extensions')}<input aria-label={t('Ekstensi diizinkan', 'Allowed extensions')} value={extensions} disabled={disabled} maxLength={500} placeholder={t('Semua jenis file', 'All file types')} onChange={event => setExtensions(event.target.value)} /></label><p className="storage-note">{t('Pisahkan dengan koma. Kosongkan untuk mengizinkan semua jenis file.', 'Separate with commas. Leave empty to allow all file types.')}</p></div></div>
      <div className="storage-extension-presets" role="group" aria-label={t('Preset ekstensi', 'Extension presets')}><span>{t('Isi cepat:', 'Quick fill:')}</span>{[[t('Semua jenis', 'All types'), ''], [t('Gambar', 'Images'), 'png, jpg, jpeg, webp, gif, svg'], [t('Dokumen', 'Documents'), 'pdf, doc, docx, xls, xlsx, txt, csv']].map(([label,value]) => <button key={label} type="button" disabled={disabled} onClick={() => setExtensions(value)}>{label}</button>)}</div>
      {normalized.length > 0 && <div className="storage-extension-chips" aria-label={t('Ekstensi terpilih', 'Selected extensions')}>{normalized.map(extension => <span key={extension}>.{extension}</span>)}</div>}
      {!valid && <p role="alert" className="storage-error">{t('Periksa ukuran file dan ekstensi: gunakan huruf/angka, maksimal 16 karakter per ekstensi.', 'Check file size and extensions: use letters/numbers, up to 16 characters per extension.')}</p>}
    </fieldset>
    <div className="storage-editor-footer"><span className="storage-note" role="status">{changed ? t('Ada perubahan yang belum disimpan.', 'You have unsaved changes.') : t('Pengaturan sudah tersimpan.', 'Settings are saved.')}</span><button type="button" disabled={disabled || !changed} onClick={reset}>{t('Batalkan perubahan', 'Discard changes')}</button><button type="submit" className="button-primary" disabled={disabled || !valid || !changed}>{t('Simpan rules', 'Save rules')}</button></div>
  </form>;
}
