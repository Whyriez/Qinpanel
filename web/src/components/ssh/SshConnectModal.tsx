import { useEffect, useState } from 'react';
import { Terminal, X } from 'lucide-react';
import { ModalOverlay } from '../ui/ModalOverlay';
import { fetchSshConnectionInfo } from '../../api/client';
import type { SshConnectionInfoQuery, SshConnectionInfoResponse } from '../../types/api';

export interface SshConnectModalProps {
  isOpen: boolean;
  onClose: () => void;
  path?: string;
  siteId?: number;
  siteDomain?: string;
  tenantUsername?: string;
  onOpenSshKeys?: () => void;
  onOpenTerminal?: () => void;
}

function CopyBlock({ label, value }: { label: string; value: string }) {
  const [message, setMessage] = useState('');
  return <div className="space-y-2 min-w-0">
    <div className="flex flex-wrap items-center justify-between gap-2"><h3 className="font-semibold">{label}</h3>
      <button type="button" className="access-button" aria-label={`Salin ${label}`} onClick={async () => {
        try { await navigator.clipboard.writeText(value); setMessage('Tersalin'); }
        catch { setMessage('Gagal menyalin. Pilih teks untuk menyalin manual.'); }
      }}>Salin</button></div>
    <pre className="max-w-full overflow-x-auto rounded border border-[var(--line)] bg-[var(--raised)] p-3 text-xs select-all" tabIndex={0}>{value}</pre>
    {message && <p role="status">{message}</p>}
  </div>;
}

/** The server generates validated commands; no client host inference. */
export function SshConnectionDetails({ info }: { info: SshConnectionInfoResponse }) {
  const [tab, setTab] = useState<'commands' | 'sftp' | 'config'>('commands');
  const generated = info.endpoint_state === 'configured' || info.endpoint_state === 'manual_preview';
  return <div className="space-y-5 min-w-0">
    {info.endpoint_state === 'missing' && <p role="status">Endpoint SSH belum diatur. Minta administrator mengatur alamat utama SSH di pengaturan konektivitas. Terminal browser dapat dipakai sesuai kebijakan shell.</p>}
    {info.endpoint_state === 'unsupported_transport' && <p role="status">Endpoint memakai {info.transport === 'cloudflare_access' ? 'Cloudflare Access' : 'jaringan privat'}. Panduan direct tidak tersedia untuk jalur ini. Gunakan terminal browser sesuai kebijakan shell.</p>}
    {info.endpoint_state === 'manual_preview' && <p role="status">Pratinjau manual: alamat ini belum dipercaya atau disimpan sebagai endpoint. Fingerprint server terdaftar tidak berlaku untuk alamat ini.</p>}
    <dl className="grid grid-cols-1 sm:grid-cols-2 gap-3 break-all">
      <div><dt className="text-[var(--ink-muted)]">Host SSH</dt><dd>{info.host || 'Belum tersedia'}</dd></div>
      <div><dt className="text-[var(--ink-muted)]">Port SSH</dt><dd>{info.port ?? 'Belum tersedia'}</dd></div>
      <div><dt className="text-[var(--ink-muted)]">Username Linux</dt><dd>{info.username}</dd></div>
      <div><dt className="text-[var(--ink-muted)]">Kebijakan akses</dt><dd>{info.shell_policy === 'full' ? 'Shell penuh dan SFTP' : info.shell_policy === 'sftp' ? 'Hanya SFTP' : 'SSH dan SFTP dinonaktifkan'}</dd></div>
      <div className="sm:col-span-2"><dt className="text-[var(--ink-muted)]">Folder server</dt><dd>{info.active_path}</dd></div>
    </dl>
    {info.shell_policy === 'none' && <p role="status">Akun ini tidak diizinkan membuka SSH, SFTP, atau terminal browser. Hubungi administrator untuk mengubah kebijakan akses.</p>}
    {info.shell_policy === 'sftp' && <p role="status">Akses shell interaktif dinonaktifkan. SFTP memakai chroot /home; folder pada client adalah {info.sftp_path}.</p>}
    {info.can_sftp && generated && <>
      {!info.has_ssh_keys && <p role="status">Belum ada kunci SSH terdaftar. Tambahkan public key di profil sebelum mencoba login dengan kunci privat Anda.</p>}
      <section aria-label="Fingerprint host key" className="space-y-2 border border-[var(--line)] rounded p-3">
        <h3 className="font-semibold">Fingerprint host key</h3>
        {info.host_keys_status === 'local_configuration' && info.host_keys.length > 0 ? <>
          {info.host_keys.map(key => <p key={key.key_type} className="break-all"><span>{key.key_type}</span><br /><code>{key.fingerprint_sha256}</code></p>)}
          <p>Identitas dari konfigurasi OpenSSH lokal. Cocokkan fingerprint yang ditampilkan client sebelum menerima host key. Ini belum membuktikan endpoint dapat dijangkau atau login berhasil.</p>
        </> : <p>{info.endpoint_state === 'manual_preview' ? 'Fingerprint tidak tersedia untuk override manual.' : 'Fingerprint belum tersedia. Verifikasi identitas host dengan administrator sebelum menerima host key.'}</p>}
      </section>
      <div role="tablist" aria-label="Panduan koneksi" className="flex flex-wrap gap-2">
        {(['commands', 'sftp', 'config'] as const).map(t => <button type="button" role="tab" tabIndex={tab === t ? 0 : -1} aria-selected={tab === t} aria-controls="ssh-guide-content" id={`ssh-guide-${t}`} key={t} onClick={() => setTab(t)} onKeyDown={event => {
          const tabs = ['commands', 'sftp', 'config'] as const;
          const index = tabs.indexOf(t);
          const next = event.key === 'ArrowRight' ? (index + 1) % 3 : event.key === 'ArrowLeft' ? (index + 2) % 3 : event.key === 'Home' ? 0 : event.key === 'End' ? 2 : -1;
          if (next >= 0) { event.preventDefault(); setTab(tabs[next]); document.getElementById(`ssh-guide-${tabs[next]}`)?.focus(); }
        }} className="access-button">{t === 'commands' ? 'Perintah' : t === 'sftp' ? 'Client SFTP' : 'Konfigurasi OpenSSH'}</button>)}
      </div>
      <div role="tabpanel" id="ssh-guide-content" aria-labelledby={`ssh-guide-${tab}`} className="space-y-4 min-w-0">
        {tab === 'commands' && <>
          <p>Perintah berikut memakai sintaks shell POSIX, misalnya Bash atau Zsh. Jalankan di komputer Anda.</p>
          {info.can_shell && info.ssh_command && <CopyBlock label="Perintah SSH" value={info.ssh_command} />}
          {info.sftp_command && <CopyBlock label="Perintah SFTP" value={info.sftp_command} />}
          {info.sftp_cd_command && <><p>Setelah login, jalankan perintah ini pada prompt sftp&gt; untuk membuka folder aktif.</p><CopyBlock label="Buka folder di SFTP" value={info.sftp_cd_command} /></>}
        </>}
        {tab === 'sftp' && <>
          <p>Pilih protokol SFTP di FileZilla, WinSCP, atau Cyberduck. Gunakan host, port dan username di atas, serta file kunci privat yang sesuai dengan public key terdaftar.</p>
          <p className="break-all">Direktori awal SFTP: <code>{info.sftp_path}</code></p>
          {info.sftp_url && <CopyBlock label="URL SFTP" value={info.sftp_url} />}
          <p>Dukungan membuka URL bergantung pada client. Direktori awal dapat diisi manual.</p>
        </>}
        {tab === 'config' && info.ssh_config_snippet && <>
          <CopyBlock label="Konfigurasi OpenSSH" value={info.ssh_config_snippet} />
          <p>Tambahkan blok ini ke ~/.ssh/config dan sesuaikan IdentityFile dengan kunci privat Anda. Kebijakan SFTP tetap berlaku pada akun yang dibatasi.</p>
        </>}
      </div>
    </>}
  </div>;
}

export function SshConnectModal({ isOpen, onClose, path, siteId, tenantUsername, onOpenSshKeys, onOpenTerminal }: SshConnectModalProps) {
  const [info, setInfo] = useState<SshConnectionInfoResponse | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  const [override, setOverride] = useState<{ host: string; port: number }>();
  const [host, setHost] = useState('');
  const [port, setPort] = useState('22');
  const [revision, setRevision] = useState(0);
  useEffect(() => { if (!isOpen) { setOverride(undefined); setInfo(null); setHost(''); setPort('22'); } }, [isOpen]);
  useEffect(() => {
    if (!isOpen) return;
    let active = true;
    setInfo(null); setError(''); setLoading(true);
    const query: SshConnectionInfoQuery = { path, site_id: siteId, tenant: tenantUsername, ...override };
    fetchSshConnectionInfo(query).then(data => {
      if (active) { setInfo(data); setHost(data.host || ''); setPort(String(data.port ?? 22)); }
    }).catch(cause => { if (active) setError(cause instanceof Error ? cause.message : 'Gagal memuat informasi koneksi.'); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [isOpen, path, siteId, tenantUsername, override, revision]);
  useEffect(() => {
    if (!isOpen) return;
    const dialog = document.getElementById('ssh-connect-title')?.closest<HTMLElement>('[role="dialog"]');
    // Retry/loading can remove the focused action. Keep keyboard dismissal and
    // the shared focus trap working after that asynchronous replacement.
    if (dialog && !dialog.contains(document.activeElement)) dialog.focus({ preventScroll: true });
  }, [isOpen, info, error, loading]);
  if (!isOpen) return null;
  return <ModalOverlay role="dialog" aria-modal="true" aria-labelledby="ssh-connect-title" onDismiss={onClose} className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-3">
    <div className="dialog-panel w-full max-w-2xl max-h-[90dvh] flex flex-col overflow-hidden rounded-lg border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)]">
      <header className="dialog-header flex items-center justify-between gap-3 p-4 border-b border-[var(--line)]"><h2 id="ssh-connect-title" className="font-semibold flex items-center gap-2"><Terminal size={18} />Koneksi SSH &amp; SFTP</h2><button type="button" aria-label="Tutup dialog" onClick={onClose} className="access-button"><X size={18} /></button></header>
      <div className="dialog-body p-4 space-y-5 text-sm overflow-y-auto min-h-0">
        {loading && <p role="status">Memuat informasi koneksi...</p>}
        {error && <div role="alert" className="space-y-2"><p>{error}</p><button className="access-button" onClick={() => setRevision(r => r + 1)}>Coba lagi</button></div>}
        {info && <SshConnectionDetails key={`${revision}-${info.endpoint_state}-${info.endpoint_id}`} info={info} />}
        <details><summary className="cursor-pointer">Pratinjau host dan port manual</summary>
          <form className="space-y-3 mt-3" onSubmit={e => { e.preventDefault(); setInfo(null); setOverride({ host, port: Number(port) }); setRevision(r => r + 1); }}>
            <p>Hanya menghasilkan panduan. Tidak menyimpan endpoint, menguji koneksi, atau menjalankan perintah.</p>
            <label className="block">Host manual<input className="access-input w-full" aria-label="Host manual" value={host} onChange={e => setHost(e.target.value)} required maxLength={253} disabled={loading} /></label>
            <label className="block">Port manual<input className="access-input w-full" aria-label="Port manual" type="number" min={1} max={65535} step={1} required value={port} onChange={e => setPort(e.target.value)} disabled={loading} /></label>
            <div className="flex flex-wrap gap-2"><button className="access-button" disabled={loading}>Buat pratinjau</button><button type="button" className="access-button" disabled={loading} onClick={() => { setInfo(null); setOverride(undefined); setRevision(r => r + 1); }}>Pakai endpoint terdaftar</button></div>
          </form>
        </details>
      </div>
      <footer className="dialog-footer flex flex-wrap items-center gap-2 p-4 border-t border-[var(--line)]">
        {info?.can_shell && (onOpenTerminal ? <button className="access-button" onClick={() => { onClose(); onOpenTerminal(); }}>Buka terminal browser</button> : <a className="access-button" href={`/terminal?tenant=${info.user_id}`}>Buka terminal browser</a>)}
        {info?.can_sftp && (onOpenSshKeys ? <button className="access-button" onClick={() => { onClose(); onOpenSshKeys(); }}>Kelola kunci SSH</button> : <a className="access-button" href="/profile">Kelola kunci SSH</a>)}
        <button className="access-button ml-auto" onClick={onClose}>Selesai</button>
      </footer>
    </div>
  </ModalOverlay>;
}
