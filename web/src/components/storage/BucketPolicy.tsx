import { useEffect, useState, type FormEvent } from 'react';
import { LockKeyhole, RefreshCw, ShieldCheck } from 'lucide-react';
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
    <div className="storage-policy-heading"><div><h3><ShieldCheck size={17} />{admin ? t('Alokasi tenant', 'Tenant allocation') : t('Privasi & rules', 'Privacy & rules')}</h3><p className="storage-note">{t('Private · hanya tenant pemilik melalui sesi sendiri.', 'Private · owner tenant through their own session only.')}</p></div><button type="button" disabled={busy || locked} onClick={() => setReload(value => value + 1)} aria-label={t('Muat ulang alokasi dan rules', 'Refresh allocation and rules')}><RefreshCw size={16} /></button></div>
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
  const t = (id: string, en: string) => english ? en : id;
  const [rules, setRules] = useState(policy.rules); const [extensions, setExtensions] = useState(policy.rules.extensions.join(', '));
  const submit = (event: FormEvent) => { event.preventDefault(); onSave({ ...rules, extensions: extensions.split(',').map(value => value.trim().toLowerCase().replace(/^\./, '')).filter(Boolean) }); };
  return <details className="storage-rules-editor"><summary>{t('Atur rules bucket', 'Configure bucket rules')}</summary><p className="storage-note">{t('Default hanya pemilik: baca, upload, dan hapus. Akses publik tidak diaktifkan. Pembatasan ekstensi memeriksa nama file, bukan isi file.', 'Defaults allow the owner to read, upload and delete. Public access stays disabled. Extension checks examine file names, not file contents.')}</p><form onSubmit={submit}>
    <div className="storage-rule-switches">{(['read', 'write', 'delete'] as const).map(action => <label className="storage-check" key={action}><input type="checkbox" checked={rules[action]} disabled={disabled} onChange={event => setRules(value => ({ ...value, [action]: event.target.checked }))} />{({ read: t('Baca & unduh', 'Read & download'), write: t('Upload', 'Upload'), delete: t('Hapus', 'Delete') })[action]}</label>)}</div>
    <div className="storage-form-grid"><label>{t('Maksimum file (MiB)', 'Maximum file size (MiB)')}<input type="number" min="0.000001" max="32" step="any" required value={rules.max_file_bytes / 1048576} disabled={disabled} onChange={event => setRules(value => ({ ...value, max_file_bytes: Math.round(Number(event.target.value) * 1048576) }))} /></label><label>{t('Ekstensi diizinkan', 'Allowed extensions')}<input value={extensions} disabled={disabled} maxLength={500} placeholder="png, jpg, pdf" onChange={event => setExtensions(event.target.value)} /></label></div><p className="storage-note">{t('Ekstensi kosong mengizinkan semua jenis nama file. Rules dapat diubah kembali oleh pemilik.', 'An empty extension list permits all file names. The owner can change these rules later.')}</p><button type="submit" disabled={disabled || rules.max_file_bytes <= 0 || rules.max_file_bytes > 33554432}>{t('Simpan rules', 'Save rules')}</button>
  </form></details>;
}
