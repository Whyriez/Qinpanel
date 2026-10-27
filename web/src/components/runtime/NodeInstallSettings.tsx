import { useId } from 'react';
import type { NodeRuntimeInstallRequest } from '../../types/api';

export interface NodeInstallOptions {
  enable_corepack: boolean;
  mirror_url: string;
  offline_tarball_path: string;
  offline_checksum: string;
}
export const defaultNodeInstallOptions: NodeInstallOptions = { enable_corepack: true, mirror_url: '', offline_tarball_path: '', offline_checksum: '' };
export function nodeInstallPayload(version: string, options: NodeInstallOptions): NodeRuntimeInstallRequest {
  return { version, enable_corepack: options.enable_corepack, mirror_url: options.mirror_url.trim() || undefined,
    offline_tarball_path: options.offline_tarball_path.trim() || undefined, offline_checksum: options.offline_checksum.trim() || undefined };
}

export function NodeInstallSettings({ value, onChange, disabled }: { value: NodeInstallOptions; onChange: (value: NodeInstallOptions) => void; disabled: boolean }) {
  const id = useId();
  const field = 'min-h-10 w-full rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] px-3 py-2 text-sm';
  return <details className="rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-4">
    <summary className="cursor-pointer text-sm font-medium">Pengaturan pemasangan Node.js</summary>
    <div className="mt-4 space-y-4">
      <p className="text-xs text-[var(--ink-muted)]">Berlaku untuk pemasangan berikutnya. Pilih versi langsung dari daftar di atas.</p>
      <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={value.enable_corepack} disabled={disabled} onChange={event => onChange({ ...value, enable_corepack: event.target.checked })} />Aktifkan Corepack untuk pnpm dan yarn</label>
      <div className="space-y-2"><label htmlFor={`${id}-mirror`} className="block text-xs font-medium">Mirror unduhan (opsional)</label>
        <input id={`${id}-mirror`} className={field} value={value.mirror_url} disabled={disabled} placeholder="https://nodejs.org/dist" onChange={event => onChange({ ...value, mirror_url: event.target.value })} />
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2"><label htmlFor={`${id}-archive`} className="block text-xs font-medium">Arsip offline di server (opsional)</label>
          <input id={`${id}-archive`} className={field} value={value.offline_tarball_path} disabled={disabled} onChange={event => onChange({ ...value, offline_tarball_path: event.target.value })} />
        </div>
        <div className="space-y-2"><label htmlFor={`${id}-checksum`} className="block text-xs font-medium">Checksum SHA256 arsip offline</label>
          <input id={`${id}-checksum`} className={field} value={value.offline_checksum} disabled={disabled} onChange={event => onChange({ ...value, offline_checksum: event.target.value })} />
        </div>
      </div>
      <p className="text-xs text-[var(--ink-muted)]">Integritas unduhan diperiksa sebelum pemasangan. Pengaturan ini tetap mengikuti daftar versi LTS yang didukung.</p>
    </div>
  </details>;
}
