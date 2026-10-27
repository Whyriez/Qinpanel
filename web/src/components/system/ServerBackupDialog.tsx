import { useEffect, useId, useRef } from 'react';
import { Archive, FileSearch, RotateCcw, X, Loader2 } from 'lucide-react';
import { ModalOverlay } from '../ui/ModalOverlay';
import type { BackupOperation } from '../../api/serverBackup';
import '../../styles/server-backup.css';

interface Props {
  operation: BackupOperation; submitting: boolean; error: string;
  adminPassword: string; accepted: boolean;
  onPassword: (value: string) => void; onAccept: (value: boolean) => void;
  onClose: () => void; onSubmit: () => void;
}
export function ServerBackupDialog({ operation, submitting, error, adminPassword, accepted, onPassword, onAccept, onClose, onSubmit }: Props) {
  const id=useId(); const alert=useRef<HTMLParagraphElement>(null);
  useEffect(()=>{ if(error) alert.current?.focus(); },[error]);
  const restore=operation==='restore';
  const title=operation==='create'?'Buat backup server':restore?'Pulihkan server dari backup':'Periksa arsip backup';
  const action=operation==='create'?'Buat backup':restore?'Restore sekarang':'Periksa arsip';
  const Icon=operation==='create'?Archive:restore?RotateCcw:FileSearch;
  return <ModalOverlay className="server-backup-modal" aria-labelledby={`${id}-title`} aria-describedby={`${id}-description`} dismissDisabled={submitting} onDismiss={onClose}>
    <section className="dialog-panel server-backup-dialog" aria-busy={submitting}>
      <header className="server-backup-dialog-header"><div className="server-backup-dialog-icon"><Icon size={22} aria-hidden="true" /></div><div><p className="server-backup-eyebrow">Backup dan migrasi</p><h2 id={`${id}-title`}>{title}</h2></div><button type="button" aria-label="Tutup dialog backup" autoFocus disabled={submitting} onClick={onClose}><X size={19} /></button></header>
      <div className="dialog-body server-backup-dialog-body">
        <p id={`${id}-description`}>{operation==='create'?'Simpan data Qinpanel dalam satu arsip terenkripsi untuk pemulihan atau pindah server.':restore?'Data instalasi tujuan akan diganti dengan data dari arsip yang telah diperiksa.':'Dekripsi dan periksa kompatibilitas arsip sebelum memulihkan data. Data server tujuan belum diubah.'}</p>
        <dl className="server-backup-facts"><div><dt>{restore?'Perlindungan':'Cakupan'}</dt><dd>{restore?'Snapshot keselamatan dibuat sebelum perubahan.':'Akun dan 2FA, file website, database, email, runtime, sertifikat dan konfigurasi panel.'}</dd></div><div><dt>Keamanan arsip</dt><dd>Passphrase tidak ditampilkan atau disimpan di Tugas. Simpan terpisah dari berkas backup.</dd></div></dl>
        {operation!=='inspect' && <div className="server-backup-pause"><strong>Jeda layanan sementara</strong><p>Website, database, email, panel dan koneksi Tunnel dapat terputus selama snapshot. Proses berlanjut di server; status di Tugas tersambung kembali setelah layanan aktif.</p></div>}
        <p className="server-backup-dialog-note">{restore?'Setelah restore, gunakan akun dan 2FA dari server sumber. Perbarui DNS bila alamat server berubah.':'Setelah dijadwalkan, Anda boleh meninggalkan halaman ini dan memantau progres melalui Tugas.'}</p>
        {error&&<p ref={alert} tabIndex={-1} role="alert" className="server-backup-dialog-error">{error}</p>}
        {restore&&<><label className="server-backup-password">Password administrator server ini<input type="password" autoComplete="current-password" value={adminPassword} disabled={submitting} onChange={event=>onPassword(event.target.value)} /></label><label className="server-backup-accept"><input type="checkbox" checked={accepted} disabled={submitting} onChange={event=>onAccept(event.target.checked)} /><span>Saya memahami bahwa restore mengganti data instalasi baru dan menggunakan akun dari backup.</span></label></>}
      </div>
      <footer className="server-backup-dialog-footer"><button type="button" className="button-secondary" disabled={submitting} onClick={onClose}>Batal</button><button type="button" className="button-primary" disabled={submitting||restore&&(!accepted||!adminPassword)} onClick={onSubmit}>{submitting?<><Loader2 size={16} className="animate-spin motion-reduce:animate-none" aria-hidden="true" />Menjadwalkan…</>:action}</button></footer>
    </section>
  </ModalOverlay>;
}
