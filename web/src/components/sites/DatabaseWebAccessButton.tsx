import { useRef, useState } from 'react';
import { ExternalLink, Loader2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { openDatabaseWebAccess } from '../../api/client';

export function DatabaseWebAccessButton({ id, name, engine }: { id: number; name: string; engine: string }) {
  const { i18n } = useTranslation();
  const english = i18n.language === 'en';
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const submitting = useRef(false);
  const tool = engine === 'mariadb' ? 'phpMyAdmin' : 'Adminer';
  const open = async () => {
    if (submitting.current) return;
    setError('');
    const tab = window.open('about:blank', '_blank');
    if (!tab) { setError(english ? 'Allow new tabs for this panel, then try again.' : 'Izinkan tab baru untuk panel ini, lalu coba lagi.'); return; }
    tab.opener = null;
    tab.document.title = english ? 'Opening database manager' : 'Membuka pengelola database';
    tab.document.body.textContent = english ? 'Preparing secure database access…' : 'Menyiapkan akses database…';
    submitting.current = true; setBusy(true);
    try {
      const result = await openDatabaseWebAccess(id);
      const expected = `/api/v1/databases/${id}/web/`;
      if (result.url !== expected) throw new Error(english ? 'Invalid database manager address.' : 'Alamat pengelola database tidak valid.');
      if (!tab.closed) tab.location.replace(expected);
    } catch (cause) {
      tab.close();
      setError(cause instanceof Error ? cause.message : (english ? 'Unable to open database manager.' : 'Gagal membuka pengelola database.'));
    } finally { submitting.current = false; setBusy(false); }
  };
  return <div className="max-w-full space-y-1">
    <button type="button" onClick={() => void open()} disabled={busy}
      aria-label={english ? `Open ${tool} for ${name} in a new tab` : `Buka ${tool} untuk ${name} di tab baru`}
      className="inline-flex min-h-9 items-center gap-2 rounded-[var(--radius-md)] border border-[var(--primary)] px-3 py-1.5 text-xs font-medium text-[var(--primary)] hover:bg-[var(--bg)] disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--primary)]">
      {busy ? <Loader2 size={14} className="animate-spin" /> : <ExternalLink size={14} />}
      {english ? 'Open' : 'Buka'} {tool}
    </button>
    {error && <p role="alert" className="max-w-sm break-words text-xs text-[var(--danger)]">{error}</p>}
  </div>;
}
