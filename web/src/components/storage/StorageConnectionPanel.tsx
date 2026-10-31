import { useState, type ReactNode } from 'react';
import { Check, Copy, KeyRound, Plug, RefreshCw, ShieldCheck } from 'lucide-react';
import { projectStorageEnvironment, type StorageConnection } from '../../api/storage';

export function StorageConnectionPanel({ connection, english, disabled, onTest, onKeys, children }: { connection: StorageConnection; english: boolean; disabled: boolean; onTest: () => void; onKeys?: () => void; children: ReactNode }) {
  const t = (id: string, en: string) => english ? en : id;
  const [copied, setCopied] = useState(''), [error, setError] = useState('');
  const copy = async (label: string, value: string) => {
    try { await navigator.clipboard.writeText(value); setCopied(label); setError(''); }
    catch { setError(t('Gagal menyalin. Pilih teks untuk menyalinnya secara manual.', 'Copy failed. Select the text to copy it manually.')); }
  };
  const fields = [['Endpoint', connection.endpoint], ['Bucket', connection.bucket], ['Region', connection.region], [t('Gaya alamat', 'Addressing style'), connection.addressing === 'path' ? 'Path style' : 'Virtual host']];
  return <div className="storage-connect-workspace">
    <header className="storage-section-heading"><span className="storage-section-icon"><Plug size={21} /></span><div><h3>{t('Hubungkan aplikasi Anda', 'Connect your application')}</h3><p className="storage-note">{t('Gunakan endpoint, bucket, dan access key untuk terhubung melalui SDK S3.', 'Use the endpoint, bucket and access key to connect using an S3 SDK.')}</p></div><button type="button" disabled={disabled} onClick={onTest}><RefreshCw size={15} />{t('Uji koneksi', 'Test connection')}</button></header>
    <section className="storage-settings-card" aria-label={t('Detail koneksi S3', 'S3 connection details')}><div className="storage-settings-card-heading"><h4>{t('Detail koneksi S3', 'S3 connection details')}</h4><span className="storage-provider-tag">{connection.managed ? 'Qinpanel' : ({ aws: 'Amazon S3', r2: 'Cloudflare R2', supabase: 'Supabase', compatible: 'S3 compatible' })[connection.provider]}</span></div>
      <dl className="storage-connect-fields">{fields.map(([label, value]) => <div key={label}><dt>{label}</dt><dd><code tabIndex={0}>{value}</code><button type="button" className="storage-copy-control" disabled={disabled} onClick={() => void copy(label, value)} aria-label={`${t('Salin', 'Copy')} ${label}`}>{copied === label ? <Check size={15} /> : <Copy size={15} />}</button></dd></div>)}</dl>
    </section>
    <section className="storage-settings-card storage-key-card"><span className="storage-section-icon"><KeyRound size={20} /></span><div><h4>{t('Access key aplikasi', 'Application access key')}</h4><p className="storage-note">{onKeys ? t('Buat atau rotasi key khusus bucket ini. Simpan secret di backend aplikasi Anda.', 'Issue or rotate a key for this bucket. Store the secret in your application backend.') : t('Gunakan key dari penyedia storage. Simpan secret di backend aplikasi, bukan di kode browser.', 'Use keys from your storage provider. Keep secrets in your application backend, outside browser code.')}</p></div>{onKeys && <button type="button" disabled={disabled} onClick={onKeys}><KeyRound size={15} />{t('Kelola access key', 'Manage access key')}</button>}</section>
    <section className="storage-settings-card"><div className="storage-settings-card-heading"><div><h4>{t('Project & environment', 'Project & environment')}</h4><p className="storage-note">{t('Pilih project tujuan dan salin konfigurasi ke environment aplikasinya.', 'Choose a project and copy the configuration to its environment.')}</p></div></div>{children}
      <div className="storage-env-header"><span>.env</span><button type="button" disabled={disabled} onClick={() => void copy('environment', projectStorageEnvironment(connection))}><Copy size={14} />{copied === 'environment' ? t('Tersalin', 'Copied') : t('Salin konfigurasi', 'Copy configuration')}</button></div>
      <pre className="storage-code" tabIndex={0}>{projectStorageEnvironment(connection)}</pre>
      <p className="storage-note storage-connect-note"><ShieldCheck size={15} />{t('Ganti placeholder key dengan kredensial aplikasi. Konfigurasi ini tidak diterapkan otomatis dan tidak memulai ulang project.', 'Replace key placeholders with application credentials. This configuration is not applied automatically and does not restart the project.')}</p>
    </section>
    {copied && <p role="status" className="storage-note">{t('Disalin ke clipboard.', 'Copied to clipboard.')}</p>}
    {error && <p role="alert" className="storage-error">{error}</p>}
  </div>;
}
