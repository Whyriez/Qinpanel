import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { ShieldCheck, X } from 'lucide-react';
import { ModalOverlay } from '../ui/ModalOverlay';
import { saveStorageConnection, type ConnectionInput, type StorageConnection, type StorageContext, type StorageProvider } from '../../api/storage';

interface Props { connection?: StorageConnection; context: StorageContext; ownerId?: number; siteId?: number; english: boolean; onClose: () => void; onSaved: (connection: StorageConnection) => void }
export function ConnectionDialog({ connection, context, ownerId, siteId, english, onClose, onSaved }: Props) {
  const t = (id: string, en: string) => english ? en : id; const id = useId();
  const [form, setForm] = useState<ConnectionInput>(() => ({ expected_revision: connection?.revision ?? 0, owner_id: connection?.owner_id ?? ownerId ?? context.owners[0]?.id ?? 0, site_id: connection?.site_id ?? siteId ?? null, label: connection?.label ?? '', provider: connection?.provider ?? 'aws', endpoint: connection?.endpoint ?? '', region: connection?.region ?? 'us-east-1', addressing: connection?.addressing ?? 'path', bucket: connection?.bucket ?? '' }));
  const [access, setAccess] = useState(''); const [secret, setSecret] = useState(''); const [session, setSession] = useState('');
  const [busy, setBusy] = useState(false); const [error, setError] = useState('');
  const errorRef = useRef<HTMLParagraphElement>(null);
  useEffect(() => { if (error) errorRef.current?.focus(); }, [error]);
  const field = <K extends keyof ConnectionInput>(key: K, value: ConnectionInput[K]) => setForm(current => ({ ...current, [key]: value }));
  const provider = (value: StorageProvider) => { setForm(current => ({ ...current, provider: value, region: value === 'r2' ? 'auto' : 'us-east-1', addressing: 'path', endpoint: '' })); setAccess(''); setSecret(''); setSession(''); };
  const save = async (event: FormEvent) => {
    event.preventDefault(); if (busy) return; setError('');
    if ((!connection || access || secret || session) && (!access || !secret)) { setError(t('Isi kedua key untuk menyimpan atau merotasi credential.', 'Enter both keys to save or rotate credentials.')); return; }
    setBusy(true);
    try { const value = await saveStorageConnection({ ...form, ...(access && secret ? { credentials: { access_key: access, secret_key: secret, session_token: session || null } } : {}) }, connection?.id); setAccess(''); setSecret(''); setSession(''); onSaved(value); }
    catch (error) { setError(error instanceof Error ? error.message : t('Gagal menyimpan koneksi.', 'Could not save connection.')); }
    finally { setBusy(false); }
  };
  return <ModalOverlay className="storage-modal" aria-labelledby={`${id}-title`} aria-describedby={`${id}-description`} onDismiss={onClose} dismissDisabled={busy}>
    <section className="dialog-panel storage-dialog">
      <header className="dialog-header storage-dialog-header">
        <div><h2 id={`${id}-title`}>{connection ? t('Edit koneksi S3', 'Edit S3 connection') : t('Hubungkan storage S3', 'Connect S3 storage')}</h2><p id={`${id}-description`}>{t('Hubungkan bucket yang sudah tersedia untuk tenant atau project Anda.', 'Connect an existing bucket for your tenant or project.')}</p></div>
        <button type="button" aria-label={t('Tutup', 'Close')} onClick={onClose} disabled={busy}><X size={18} /></button>
      </header>
      <form className="dialog-form" onSubmit={save} aria-busy={busy}>
        <div className="dialog-body storage-dialog-body">
          <fieldset className="storage-form-section">
            <legend>{t('Identitas koneksi', 'Connection details')}</legend>
            <div className="storage-form-grid">
              <label>{t('Nama koneksi', 'Connection name')}<input autoFocus required maxLength={80} value={form.label} onChange={e => field('label', e.target.value)} disabled={busy} /></label>
              <label>Provider<select aria-label="Provider" value={form.provider} onChange={e => provider(e.target.value as StorageProvider)} disabled={busy}><option value="aws">Amazon S3</option><option value="r2">Cloudflare R2</option><option value="supabase">Supabase S3</option><option value="compatible">{t('S3 kompatibel', 'S3 compatible')}</option></select></label>
              <label>Tenant<select aria-label="Tenant" required value={form.owner_id || ''} disabled={busy || !!connection || !!ownerId} onChange={e => setForm(current => ({ ...current, owner_id: Number(e.target.value), site_id: null }))}><option value="" disabled>{t('Pilih tenant', 'Choose tenant')}</option>{context.owners.map(owner => <option key={owner.id} value={owner.id}>{owner.username}</option>)}</select></label>
              <label>Project<select aria-label="Project" value={form.site_id ?? ''} disabled={busy || !!siteId} onChange={e => field('site_id', e.target.value ? Number(e.target.value) : null)}><option value="">{t('Tanpa project', 'No project')}</option>{context.sites.filter(site => site.owner_id === form.owner_id).map(site => <option key={site.id} value={site.id}>{site.domain}</option>)}</select></label>
            </div>
          </fieldset>

          <fieldset className="storage-form-section">
            <legend>{t('Tujuan bucket', 'Bucket destination')}</legend>
            <div className="storage-form-grid storage-destination-grid">
              <label className="storage-form-wide">Endpoint HTTPS<input required type="url" autoComplete="off" spellCheck={false} value={form.endpoint} placeholder={form.provider === 'supabase' ? 'https://project.storage.supabase.co/storage/v1/s3' : form.provider === 'r2' ? 'https://account.r2.cloudflarestorage.com' : 'https://s3.us-east-1.amazonaws.com'} disabled={busy} onChange={e => field('endpoint', e.target.value)} /></label>
              <label>Bucket<input required minLength={3} maxLength={63} spellCheck={false} value={form.bucket} disabled={busy} onChange={e => field('bucket', e.target.value)} /></label>
              <label>Region<input required maxLength={63} value={form.region} disabled={busy || form.provider === 'r2'} onChange={e => field('region', e.target.value)} /></label>
              <label>{t('Gaya alamat', 'Addressing style')}<select aria-label={t('Gaya alamat', 'Addressing style')} value={form.addressing} disabled={busy || ['r2', 'supabase'].includes(form.provider)} onChange={e => field('addressing', e.target.value as 'path' | 'virtual_host')}><option value="path">Path-style</option><option value="virtual_host">Virtual-hosted</option></select></label>
            </div>
            {form.provider === 'compatible' && <p className="storage-note storage-field-hint">{t('Gunakan endpoint S3 yang sudah berjalan. Formulir ini menghubungkan layanan tersebut.', 'Use an S3 endpoint that is already running. This form connects that service.')}</p>}
          </fieldset>

          <fieldset className="storage-form-section" aria-describedby={`${id}-credentials-note`}>
            <legend>{t('Kredensial akses', 'Access credentials')}</legend>
            <div className="storage-form-grid">
              <label>Access key ID<input type="password" autoComplete="new-password" spellCheck={false} required={!connection} maxLength={256} value={access} disabled={busy} onChange={e => setAccess(e.target.value)} /></label>
              <label>Secret access key<input type="password" autoComplete="new-password" spellCheck={false} required={!connection} maxLength={1024} value={secret} disabled={busy} onChange={e => setSecret(e.target.value)} /></label>
            </div>
            <details className="storage-session">
              <summary>{t('Session token (opsional)', 'Session token (optional)')}</summary>
              <label>{t('Session token', 'Session token')}<input type="password" autoComplete="new-password" spellCheck={false} maxLength={16384} value={session} disabled={busy} onChange={e => setSession(e.target.value)} /></label>
            </details>
            <div className="storage-credential-note"><ShieldCheck size={17} aria-hidden="true" /><p id={`${id}-credentials-note`}>{t('Key dienkripsi dan tidak ditampilkan kembali. Gunakan key khusus bucket.', 'Keys are encrypted and never shown again. Use bucket-scoped keys.')}{connection && <> {t('Kosongkan seluruh key untuk mempertahankan credential; isi ulang saat mengganti tujuan bucket.', 'Leave all key fields empty to keep credentials; re-enter them when changing the bucket destination.')}</>}</p></div>
            {form.provider === 'supabase' && <p className="storage-note storage-field-hint">{t('Access key S3 Supabase standar memiliki akses seluruh bucket dan melewati RLS. Gunakan credential session/RLS atau project terpisah untuk akses tenant yang terbatas.', 'Standard Supabase S3 access keys can access all buckets and bypass RLS. Use session/RLS credentials or a separate project for restricted tenant access.')}</p>}
          </fieldset>
          {error && <p ref={errorRef} role="alert" tabIndex={-1} className="storage-error">{error}</p>}
        </div>
        <footer className="dialog-footer storage-dialog-footer">
          <p>{t('Menyimpan koneksi tidak membuat bucket baru.', 'Saving a connection does not create a bucket.')}</p>
          <div><button type="button" onClick={onClose} disabled={busy}>{t('Batal', 'Cancel')}</button><button type="submit" className="button-primary" disabled={busy || !form.owner_id}>{busy ? t('Menyimpan…', 'Saving…') : t('Simpan koneksi', 'Save connection')}</button></div>
        </footer>
      </form>
    </section>
  </ModalOverlay>;
}
