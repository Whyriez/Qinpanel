import { useId, useRef, useState } from 'react';
import { Download, Loader2 } from 'lucide-react';

export function isFullPythonVersion(version: string): boolean {
  return /^3\.(0|[1-9]\d{0,4})\.(0|[1-9]\d{0,4})$/.test(version) && version.split('.').every(part => Number(part) <= 65535);
}

interface Props {
  installedVersions: string[];
  busyVersions: string[];
  unavailableReason: string;
  onInstall: (version: string) => Promise<void>;
}

export function PythonManualInstall({ installedVersions, busyVersions, unavailableReason, onInstall }: Props) {
  const id = useId();
  const [version, setVersion] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState('');
  const inFlight = useRef(false);
  const value = version.trim();
  const installed = installedVersions.includes(value);
  const busy = busyVersions.includes(value);
  const valid = isFullPythonVersion(value);
  return <details className="rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-4">
    <summary className="cursor-pointer text-sm font-medium">Pasang versi Python lainnya</summary>
    <form className="mt-4 space-y-3" onSubmit={event => {
      event.preventDefault();
      if (inFlight.current || !valid || installed || busy || unavailableReason) return;
      inFlight.current = true; setSubmitting(true); setError('');
      void onInstall(value).then(() => setVersion(''))
        .catch(cause => setError(cause instanceof Error ? cause.message : 'Gagal membuat tugas Python.'))
        .finally(() => { inFlight.current = false; setSubmitting(false); });
    }}>
      <label htmlFor={id} className="block text-xs font-medium">Nomor versi lengkap</label>
      <div className="flex flex-col gap-2 sm:flex-row sm:items-start">
        <input id={id} value={version} onChange={event => { setVersion(event.target.value); setError(''); }} placeholder="Contoh: 3.15.0" autoComplete="off" maxLength={13} required disabled={submitting}
          aria-describedby={`${id}-help`} className="min-h-10 w-full rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] px-3 py-2 text-sm sm:max-w-xs" />
        <button type="submit" disabled={!valid || installed || busy || submitting || Boolean(unavailableReason)} className="inline-flex min-h-10 items-center justify-center gap-2 rounded-[var(--radius-md)] bg-[var(--primary)] px-3 py-2 text-xs font-semibold text-white disabled:opacity-50">
          {submitting ? <Loader2 size={14} className="animate-spin" /> : <Download size={14} />}{submitting ? 'Membuat tugas…' : 'Pasang versi Python'}
        </button>
      </div>
      <p id={`${id}-help`} className="text-xs text-[var(--ink-muted)]">Isi versi stabil lengkap, misalnya 3.15.0. Ketersediaannya diperiksa saat tugas berjalan.</p>
      {value && !valid && <p role="alert" className="text-xs text-[var(--danger)]">Gunakan format versi lengkap 3.x.y, tanpa path atau perintah.</p>}
      {(installed || busy || unavailableReason) && <p role="status" className="text-xs text-[var(--ink-muted)]">{installed ? 'Versi ini sudah terpasang.' : busy ? 'Versi ini sedang dipasang. Pantau progres pada daftar runtime.' : unavailableReason}</p>}
      {error && <p role="alert" className="text-sm text-[var(--danger)]">{error}</p>}
    </form>
  </details>;
}
