import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useCallback, useMemo } from 'react';
import {
  Archive,
  Plus,
  Trash2,
  Download,
  RotateCcw,
  Calendar,
  Folder,
  Database,
  Clock,
  HardDrive,
  RefreshCw,
  Search,
  CheckCircle2,
  AlertCircle,
  Loader2,
  X,
  Cloud,
  FileArchive,
  Sliders,
} from 'lucide-react';
import {
  fetchSiteBackups,
  createSiteBackup,
  deleteSiteBackup,
  restoreSiteBackup,
  fetchBackupSchedule,
  updateBackupSchedule,
  getBackupDownloadUrl,
} from '../../api/client';
import {
  BackupItem,
  BackupFormat,
  BackupSchedule,
  BackupScheduleFrequency,
  BackupStorageType,
  CreateBackupRequest,
  RestoreBackupRequest,
  UpdateScheduleRequest,
} from '../../types/api';
import { ConfirmDangerDialog } from '../ui/ConfirmDangerDialog';

export interface SiteBackupsTabProps {
  siteId?: number;
  siteDomain?: string;
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  return `${(bytes / Math.pow(1024, i)).toFixed(i === 0 ? 0 : 2)} ${units[i]}`;
}

function formatDateTime(unixSecs: number): string {
  if (!unixSecs) return '-';
  const d = new Date(unixSecs * 1000);
  return d.toLocaleString('id-ID', {
    day: '2-digit',
    month: 'short',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  });
}

export const SiteBackupsTab: React.FC<SiteBackupsTabProps> = ({
  siteId,
  siteDomain,
}) => {

  const [backups, setBackups] = useState<BackupItem[]>([]);
  const [totalBytesUsed, setTotalBytesUsed] = useState<number>(0);
  const [schedule, setSchedule] = useState<BackupSchedule | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [searchQuery, setSearchQuery] = useState('');
  const [toastMessage, setToastMessage] = useState<{
    text: string;
    type: 'success' | 'error';
  } | null>(null);

  // Create Backup Modal
  const [isCreateModalOpen, setIsCreateModalOpen] = useState(false);
  const [createLabel, setCreateLabel] = useState('');
  const [createFormat, setCreateFormat] = useState<BackupFormat>('tar.zst');
  const [createIncludeFiles, setCreateIncludeFiles] = useState(true);
  const [createIncludeDatabases, setCreateIncludeDatabases] = useState(true);
  const [isCreating, setIsCreating] = useState(false);

  // Restore Modal
  const [isRestoreModalOpen, setIsRestoreModalOpen] = useState(false);
  const [selectedBackupForRestore, setSelectedBackupForRestore] = useState<BackupItem | null>(null);
  const [restoreTargetType, setRestoreTargetType] = useState<'inplace' | 'custom'>('inplace');
  const [restoreCustomPath, setRestoreCustomPath] = useState('');
  const [restoreFiles, setRestoreFiles] = useState(true);
  const [restoreDatabases, setRestoreDatabases] = useState(true);
  const [isRestoring, setIsRestoring] = useState(false);

  // Schedule Modal
  const [isScheduleModalOpen, setIsScheduleModalOpen] = useState(false);
  const [scheduleEnabled, setScheduleEnabled] = useState(false);
  const [scheduleFrequency, setScheduleFrequency] = useState<BackupScheduleFrequency>('daily');
  const [scheduleHour, setScheduleHour] = useState(2);
  const [scheduleMinute, setScheduleMinute] = useState(0);
  const [scheduleRetention, setScheduleRetention] = useState(7);
  const [scheduleIncludeFiles, setScheduleIncludeFiles] = useState(true);
  const [scheduleIncludeDatabases, setScheduleIncludeDatabases] = useState(true);
  const [scheduleStorageType, setScheduleStorageType] = useState<BackupStorageType>('local');
  const [s3Endpoint, setS3Endpoint] = useState('');
  const [s3Bucket, setS3Bucket] = useState('');
  const [s3Region, setS3Region] = useState('us-east-1');
  const [s3AccessKey, setS3AccessKey] = useState('');
  const [s3SecretKey, setS3SecretKey] = useState('');
  const [s3Prefix, setS3Prefix] = useState('');
  const [isSavingSchedule, setIsSavingSchedule] = useState(false);

  // Delete Backup State
  const [backupToDelete, setBackupToDelete] = useState<BackupItem | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  // Load Data
  const loadData = useCallback(async () => {
    if (!siteId) return;
    setIsLoading(true);
    try {
      const [listRes, schedRes] = await Promise.all([
        fetchSiteBackups(siteId),
        fetchBackupSchedule(siteId).catch(() => null),
      ]);
      setBackups(listRes.backups || []);
      setTotalBytesUsed(listRes.total_bytes_used || 0);
      setSchedule(schedRes);

      if (schedRes) {
        setScheduleEnabled(schedRes.is_enabled);
        setScheduleFrequency(schedRes.frequency);
        setScheduleHour(schedRes.time_hour);
        setScheduleMinute(schedRes.time_minute);
        setScheduleRetention(schedRes.retention_count);
        setScheduleIncludeFiles(schedRes.include_files);
        setScheduleIncludeDatabases(schedRes.include_databases);
        setScheduleStorageType(schedRes.storage_destination);
        setS3Endpoint(schedRes.s3_endpoint || '');
        setS3Bucket(schedRes.s3_bucket || '');
        setS3Region(schedRes.s3_region || 'us-east-1');
        setS3AccessKey(schedRes.s3_access_key || '');
        setS3Prefix(schedRes.s3_prefix || '');
      }
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal memuat daftar cadangan',
        type: 'error',
      });
    } finally {
      setIsLoading(false);
    }
  }, [siteId]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Create Backup Action
  const handleCreateBackup = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!siteId) return;
    if (!createIncludeFiles && !createIncludeDatabases) {
      setToastMessage({
        text: 'Pilih setidaknya satu konten untuk dicadangkan (berkas atau basis data)',
        type: 'error',
      });
      return;
    }

    setIsCreating(true);
    try {
      const payload: CreateBackupRequest = {
        label: createLabel.trim() || undefined,
        format: createFormat,
        include_files: createIncludeFiles,
        include_databases: createIncludeDatabases,
      };
      const res = await createSiteBackup(siteId, payload);
      setToastMessage({
        text: res.message || 'Proses pembuatan cadangan telah dimulai di latar belakang',
        type: 'success',
      });
      setIsCreateModalOpen(false);
      setCreateLabel('');
      await loadData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal memulai pembuatan cadangan',
        type: 'error',
      });
    } finally {
      setIsCreating(false);
    }
  };

  // Restore Backup Action
  const handleRestoreBackup = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!siteId || !selectedBackupForRestore) return;
    if (!restoreFiles && !restoreDatabases) {
      setToastMessage({
        text: 'Pilih setidaknya satu komponen untuk dipulihkan (berkas atau basis data)',
        type: 'error',
      });
      return;
    }

    setIsRestoring(true);
    try {
      const payload: RestoreBackupRequest = {
        target_subpath: restoreTargetType === 'custom' ? restoreCustomPath.trim() : undefined,
        restore_files: restoreFiles,
        restore_databases: restoreDatabases,
      };
      const res = await restoreSiteBackup(siteId, selectedBackupForRestore.id, payload);
      setToastMessage({
        text: res.message || 'Proses pemulihan cadangan telah dimulai di latar belakang',
        type: 'success',
      });
      setIsRestoreModalOpen(false);
      setSelectedBackupForRestore(null);
      setRestoreCustomPath('');
      await loadData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal memulai pemulihan cadangan',
        type: 'error',
      });
    } finally {
      setIsRestoring(false);
    }
  };

  // Update Schedule Action
  const handleSaveSchedule = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!siteId) return;

    setIsSavingSchedule(true);
    try {
      const payload: UpdateScheduleRequest = {
        is_enabled: scheduleEnabled,
        frequency: scheduleFrequency,
        time_hour: Number(scheduleHour),
        time_minute: Number(scheduleMinute),
        retention_count: Number(scheduleRetention),
        include_files: scheduleIncludeFiles,
        include_databases: scheduleIncludeDatabases,
        storage_destination: scheduleStorageType,
        s3_endpoint: scheduleStorageType === 's3' ? s3Endpoint.trim() : undefined,
        s3_bucket: scheduleStorageType === 's3' ? s3Bucket.trim() : undefined,
        s3_region: scheduleStorageType === 's3' ? s3Region.trim() : undefined,
        s3_access_key: scheduleStorageType === 's3' ? s3AccessKey.trim() : undefined,
        s3_secret_key: scheduleStorageType === 's3' && s3SecretKey.trim() ? s3SecretKey.trim() : undefined,
        s3_prefix: scheduleStorageType === 's3' ? s3Prefix.trim() : undefined,
      };

      const updated = await updateBackupSchedule(siteId, payload);
      setSchedule(updated);
      setToastMessage({
        text: 'Pengaturan jadwal cadangan berhasil disimpan',
        type: 'success',
      });
      setIsScheduleModalOpen(false);
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal menyimpan pengaturan jadwal cadangan',
        type: 'error',
      });
    } finally {
      setIsSavingSchedule(false);
    }
  };

  // Delete Backup Action
  const handleDeleteBackup = async () => {
    if (!siteId || !backupToDelete) return;
    setIsDeleting(true);
    try {
      await deleteSiteBackup(siteId, backupToDelete.id);
      setToastMessage({
        text: `Berkas cadangan ${backupToDelete.filename} berhasil dihapus`,
        type: 'success',
      });
      setBackupToDelete(null);
      await loadData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal menghapus berkas cadangan',
        type: 'error',
      });
    } finally {
      setIsDeleting(false);
    }
  };

  // Filtered backups
  const filteredBackups = useMemo(() => {
    if (!searchQuery.trim()) return backups;
    const q = searchQuery.toLowerCase();
    return backups.filter(
      (b) =>
        b.filename.toLowerCase().includes(q) ||
        (b.label && b.label.toLowerCase().includes(q))
    );
  }, [backups, searchQuery]);

  return (
    <div className="space-y-6">
      {/* Toast Alert */}
      {toastMessage && (
        <div
          role="alert"
          className={`p-3.5 rounded-[var(--radius-md)] text-xs flex items-center justify-between border ${
            toastMessage.type === 'success'
              ? 'bg-[var(--success)]/10 text-[var(--success)] border-[var(--success)]/30'
              : 'bg-[var(--danger)]/10 text-[var(--danger)] border-[var(--danger)]/30'
          }`}
        >
          <div className="flex items-center gap-2">
            {toastMessage.type === 'success' ? (
              <CheckCircle2 className="w-4 h-4 shrink-0" />
            ) : (
              <AlertCircle className="w-4 h-4 shrink-0" />
            )}
            <span>{toastMessage.text}</span>
          </div>
          <button
            onClick={() => setToastMessage(null)}
            className="p-1 hover:bg-black/5 dark:hover:bg-white/5 rounded"
            aria-label="Tutup notifikasi"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      )}

      {/* Summary Cards */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        {/* Total Storage Used */}
        <div className="p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-xs">
          <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
            <span className="flex items-center gap-1.5 font-medium">
              <HardDrive className="w-4 h-4 text-blue-500" />
              Total Penggunaan Ruang Cadangan
            </span>
            <span className="font-semibold text-[var(--ink)]">
              {formatBytes(totalBytesUsed)}
            </span>
          </div>
          <div className="mt-2 text-[11px] text-[var(--ink-muted)]">
            Total {backups.length} arsip tersimpan untuk situs {siteDomain || 'ini'}
          </div>
        </div>

        {/* Schedule Status */}
        <div className="p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-xs">
          <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
            <span className="flex items-center gap-1.5 font-medium">
              <Calendar className="w-4 h-4 text-emerald-500" />
              Pencadangan Otomatis
            </span>
            <span
              className={`px-2 py-0.5 text-[10px] font-semibold rounded-full ${
                schedule?.is_enabled
                  ? 'bg-emerald-500/10 text-emerald-600 border border-emerald-500/20'
                  : 'bg-zinc-500/10 text-zinc-500 border border-zinc-500/20'
              }`}
            >
              {schedule?.is_enabled ? 'Aktif' : 'Nonaktif'}
            </span>
          </div>
          <div className="mt-2 text-[11px] text-[var(--ink-muted)]">
            {schedule?.is_enabled
              ? `${schedule.frequency === 'daily' ? 'Harian' : schedule.frequency === 'weekly' ? 'Mingguan' : 'Bulanan'} pada ${String(schedule.time_hour).padStart(2, '0')}:${String(schedule.time_minute).padStart(2, '0')} (Retensi: ${schedule.retention_count})`
              : 'Jadwal otomatis belum diaktifkan'}
          </div>
        </div>

        {/* Storage Destination */}
        <div className="p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-xs">
          <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
            <span className="flex items-center gap-1.5 font-medium">
              <Cloud className="w-4 h-4 text-indigo-500" />
              Tujuan Arsip
            </span>
            <span className="font-semibold text-[var(--ink)] capitalize">
              {schedule?.storage_destination === 's3' ? 'Remote S3' : 'Lokal Host'}
            </span>
          </div>
          <div className="mt-2 text-[11px] text-[var(--ink-muted)] truncate">
            {schedule?.storage_destination === 's3' && schedule.s3_bucket
              ? `Bucket: ${schedule.s3_bucket}`
              : 'Tersimpan aman di folder ~/backups tenant'}
          </div>
        </div>
      </div>

      {/* Action Bar */}
      <div className="flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3">
        <div className="relative flex-1 max-w-sm">
          <Search className="w-4 h-4 absolute left-3 top-2.5 text-[var(--ink-muted)]" />
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Cari berkas atau label cadangan..."
            className="w-full pl-9 pr-3 py-1.5 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] placeholder-[var(--ink-muted)] focus:outline-hidden focus:border-[var(--primary)]"
          />
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={loadData}
            disabled={isLoading}
            className="p-1.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] disabled:opacity-50 transition-colors"
            title="Muat ulang daftar cadangan"
            aria-label="Muat ulang"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? 'animate-spin' : ''}`} />
          </button>

          <button
            type="button"
            onClick={() => setIsScheduleModalOpen(true)}
            className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] transition-colors flex items-center gap-1.5"
          >
            <Sliders className="w-3.5 h-3.5 text-indigo-500" />
            <span>Jadwal & Retensi</span>
          </button>

          <button
            type="button"
            onClick={() => setIsCreateModalOpen(true)}
            className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 transition-opacity flex items-center gap-1.5 shadow-xs"
          >
            <Plus className="w-3.5 h-3.5" />
            <span>Buat Cadangan</span>
          </button>
        </div>
      </div>

      {/* Backups Table */}
      <div className="border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--surface)] overflow-hidden shadow-xs">
        {isLoading && backups.length === 0 ? (
          <div className="p-8 text-center text-xs text-[var(--ink-muted)] flex items-center justify-center gap-2">
            <Loader2 className="w-4 h-4 animate-spin text-[var(--primary)]" />
            Memuat daftar arsip cadangan...
          </div>
        ) : filteredBackups.length === 0 ? (
          <div className="p-12 text-center">
            <FileArchive className="w-10 h-10 mx-auto text-[var(--ink-muted)]/50 mb-3" />
            <h3 className="text-sm font-semibold text-[var(--ink)]">Belum ada berkas cadangan</h3>
            <p className="text-xs text-[var(--ink-muted)] mt-1 max-w-sm mx-auto">
              Buat snapshot cadangan pertama situs ini untuk mengamankan berkas web dan basis data Anda.
            </p>
            <button
              type="button"
              onClick={() => setIsCreateModalOpen(true)}
              className="mt-4 px-3.5 py-2 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 transition-opacity inline-flex items-center gap-1.5 shadow-xs"
            >
              <Plus className="w-3.5 h-3.5" />
              <span>Buat Cadangan Sekarang</span>
            </button>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left border-collapse text-xs">
              <thead>
                <tr className="border-b border-[var(--line)] bg-[var(--raised)]/50 text-[var(--ink-muted)] font-medium">
                  <th className="py-2.5 px-3">Nama Berkas & Label</th>
                  <th className="py-2.5 px-3">Tanggal Dibuat</th>
                  <th className="py-2.5 px-3">Ukuran</th>
                  <th className="py-2.5 px-3">Isi Cadangan</th>
                  <th className="py-2.5 px-3">Status</th>
                  <th className="py-2.5 px-3 text-right">Aksi</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--line)]">
                {filteredBackups.map((b) => (
                  <tr key={b.id} className="hover:bg-[var(--raised)]/30 transition-colors">
                    <td className="py-3 px-3">
                      <div className="font-mono font-medium text-[var(--ink)] flex items-center gap-1.5">
                        <FileArchive className="w-3.5 h-3.5 text-blue-500 shrink-0" />
                        <span className="truncate max-w-xs" title={b.filename}>{b.filename}</span>
                      </div>
                      {b.label && (
                        <div className="mt-0.5">
                          <span className="inline-block px-1.5 py-0.5 rounded text-[10px] font-medium bg-blue-500/10 text-blue-600 border border-blue-500/20">
                            {b.label}
                          </span>
                        </div>
                      )}
                    </td>

                    <td className="py-3 px-3 text-[var(--ink-muted)] whitespace-nowrap">
                      {formatDateTime(b.created_at)}
                    </td>

                    <td className="py-3 px-3 font-mono font-medium text-[var(--ink)] whitespace-nowrap">
                      {formatBytes(b.size_bytes)}
                    </td>

                    <td className="py-3 px-3 whitespace-nowrap">
                      <div className="flex items-center gap-1.5">
                        {b.includes_files && (
                          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-medium bg-emerald-500/10 text-emerald-600 border border-emerald-500/20">
                            <Folder className="w-2.5 h-2.5" />
                            Berkas
                          </span>
                        )}
                        {b.includes_databases && (
                          <span className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded text-[10px] font-medium bg-purple-500/10 text-purple-600 border border-purple-500/20">
                            <Database className="w-2.5 h-2.5" />
                            Basis Data
                          </span>
                        )}
                      </div>
                    </td>

                    <td className="py-3 px-3 whitespace-nowrap">
                      <span
                        className={`inline-block px-2 py-0.5 rounded-full text-[10px] font-semibold border ${
                          b.status === 'completed'
                            ? 'bg-emerald-500/10 text-emerald-600 border-emerald-500/20'
                            : b.status === 'running' || b.status === 'restoring'
                            ? 'bg-amber-500/10 text-amber-600 border-amber-500/20'
                            : b.status === 'failed'
                            ? 'bg-rose-500/10 text-rose-600 border-rose-500/20'
                            : 'bg-zinc-500/10 text-zinc-500 border-zinc-500/20'
                        }`}
                      >
                        {b.status === 'completed'
                          ? 'Selesai'
                          : b.status === 'running'
                          ? 'Diproses'
                          : b.status === 'restoring'
                          ? 'Memulihkan'
                          : b.status === 'failed'
                          ? 'Gagal'
                          : b.status}
                      </span>
                    </td>

                    <td className="py-3 px-3 text-right whitespace-nowrap">
                      <div className="flex items-center justify-end gap-1.5">
                        {/* Download button */}
                        {siteId && (
                          <a
                            href={getBackupDownloadUrl(siteId, b.id)}
                            download={b.filename}
                            className="p-1.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] transition-colors"
                            title="Unduh arsip cadangan"
                            aria-label={`Unduh ${b.filename}`}
                          >
                            <Download className="w-3.5 h-3.5" />
                          </a>
                        )}

                        {/* Restore button */}
                        <button
                          type="button"
                          onClick={() => {
                            setSelectedBackupForRestore(b);
                            setRestoreTargetType('inplace');
                            setRestoreCustomPath('');
                            setRestoreFiles(b.includes_files);
                            setRestoreDatabases(b.includes_databases);
                            setIsRestoreModalOpen(true);
                          }}
                          className="px-2.5 py-1 text-xs font-semibold rounded-[var(--radius-md)] border border-emerald-500/30 bg-emerald-500/10 text-emerald-600 hover:bg-emerald-500/20 transition-colors flex items-center gap-1"
                          title="Pulihkan situs dari cadangan ini"
                        >
                          <RotateCcw className="w-3.5 h-3.5" />
                          <span>Pulihkan</span>
                        </button>

                        {/* Delete button */}
                        <button
                          type="button"
                          onClick={() => setBackupToDelete(b)}
                          className="p-1.5 rounded-[var(--radius-md)] border border-[var(--danger)]/30 bg-[var(--danger)]/10 text-[var(--danger)] hover:bg-[var(--danger)]/20 transition-colors"
                          title="Hapus arsip cadangan"
                          aria-label={`Hapus ${b.filename}`}
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* ===================================================================== */}
      {/* MODAL 1: CREATE BACKUP                                                */}
      {/* ===================================================================== */}
      {isCreateModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4 backdrop-blur-xs"
          onDismiss={() => setIsCreateModalOpen(false)}
        >
          <div className="w-full max-w-md rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] p-6 shadow-xl dialog-panel">
            <div className="flex items-center justify-between pb-3 border-b border-[var(--line)] dialog-header">
              <h3 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
                <Archive className="w-4 h-4 text-[var(--primary)]" />
                Buat Cadangan Baru
              </h3>
              <button
                type="button"
                onClick={() => setIsCreateModalOpen(false)}
                className="text-[var(--ink-muted)] hover:text-[var(--ink)] p-1 rounded"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <form onSubmit={handleCreateBackup} className="mt-4 text-xs dialog-form">
              <div className="dialog-body space-y-4">
                <div>
                  <label className="block text-[var(--ink-muted)] font-medium mb-1">Label Cadangan (Opsional)</label>
                  <input
                    type="text"
                    value={createLabel}
                    onChange={(e) => setCreateLabel(e.target.value)}
                    placeholder="Misal: Sebelum update v2.0"
                    maxLength={64}
                    className="w-full px-3 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                  />
                  <p className="text-[11px] text-[var(--ink-muted)] mt-1">
                    Maksimal 64 karakter (huruf, angka, spasi, tanda minus, underscore).
                  </p>
                </div>

                <div>
                  <label className="block text-[var(--ink-muted)] font-medium mb-1">Format Kompresi</label>
                  <div className="grid grid-cols-2 gap-2">
                    <label
                      className={`flex items-start gap-2 p-2.5 rounded-[var(--radius-md)] border cursor-pointer ${
                        createFormat === 'tar.zst'
                          ? 'border-[var(--primary)] bg-[var(--primary)]/5 text-[var(--ink)]'
                          : 'border-[var(--line)] bg-[var(--surface)] text-[var(--ink-muted)]'
                      }`}
                    >
                      <input
                        type="radio"
                        name="format"
                        checked={createFormat === 'tar.zst'}
                        onChange={() => setCreateFormat('tar.zst')}
                        className="mt-0.5"
                      />
                      <div>
                        <div className="font-semibold text-xs">tar.zst (Default)</div>
                        <div className="text-[10px] mt-0.5">Zstandard super cepat & rasio kompresi tinggi</div>
                      </div>
                    </label>

                    <label
                      className={`flex items-start gap-2 p-2.5 rounded-[var(--radius-md)] border cursor-pointer ${
                        createFormat === 'tar.gz'
                          ? 'border-[var(--primary)] bg-[var(--primary)]/5 text-[var(--ink)]'
                          : 'border-[var(--line)] bg-[var(--surface)] text-[var(--ink-muted)]'
                      }`}
                    >
                      <input
                        type="radio"
                        name="format"
                        checked={createFormat === 'tar.gz'}
                        onChange={() => setCreateFormat('tar.gz')}
                        className="mt-0.5"
                      />
                      <div>
                        <div className="font-semibold text-xs">tar.gz</div>
                        <div className="text-[10px] mt-0.5">Format standar Gzip untuk kompatibilitas umum</div>
                      </div>
                    </label>
                  </div>
                </div>

                <div className="space-y-2 pt-2 border-t border-[var(--line)]">
                  <label className="block text-[var(--ink-muted)] font-medium">Komponen Yang Dicadangkan</label>
                  <label className="flex items-center gap-2 p-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer">
                    <input
                      type="checkbox"
                      checked={createIncludeFiles}
                      onChange={(e) => setCreateIncludeFiles(e.target.checked)}
                      className="rounded text-[var(--primary)]"
                    />
                    <Folder className="w-3.5 h-3.5 text-emerald-500" />
                    <span className="font-medium text-[var(--ink)]">Sertakan Berkas Situs (Docroot)</span>
                  </label>

                  <label className="flex items-center gap-2 p-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer">
                    <input
                      type="checkbox"
                      checked={createIncludeDatabases}
                      onChange={(e) => setCreateIncludeDatabases(e.target.checked)}
                      className="rounded text-[var(--primary)]"
                    />
                    <Database className="w-3.5 h-3.5 text-purple-500" />
                    <span className="font-medium text-[var(--ink)]">Sertakan Basis Data Terkait</span>
                  </label>
                </div>

                <div className="p-3 rounded-[var(--radius-md)] bg-[var(--raised)] border border-[var(--line)] text-[11px] text-[var(--ink-muted)] flex items-start gap-2">
                  <Clock className="w-4 h-4 text-blue-500 shrink-0 mt-0.5" />
                  <span>
                    Cadangan diproses di latar belakang secara aman tanpa membebani web server. Ukuran arsip cadangan
                    dihitung dalam kuota disk tenant.
                  </span>
                </div>
              </div>
              <div className="flex items-center justify-end gap-2 pt-3 border-t border-[var(--line)] dialog-footer">
                <button
                  type="button"
                  onClick={() => setIsCreateModalOpen(false)}
                  disabled={isCreating}
                  className="px-3.5 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] font-medium"
                >
                  Batal
                </button>
                <button
                  type="submit"
                  disabled={isCreating}
                  className="px-4 py-1.5 rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 font-medium flex items-center gap-1.5 shadow-xs"
                >
                  {isCreating ? (
                    <>
                      <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      Memulai...
                    </>
                  ) : (
                    'Mulai Pencadangan'
                  )}
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* ===================================================================== */}
      {/* MODAL 2: RESTORE BACKUP                                               */}
      {/* ===================================================================== */}
      {isRestoreModalOpen && selectedBackupForRestore && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4 backdrop-blur-xs"
          onDismiss={() => setIsRestoreModalOpen(false)}
        >
          <div className="w-full max-w-md rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] p-6 shadow-xl dialog-panel">
            <div className="flex items-center justify-between pb-3 border-b border-[var(--line)] dialog-header">
              <h3 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
                <RotateCcw className="w-4 h-4 text-emerald-500" />
                Pulihkan Cadangan Situs
              </h3>
              <button
                type="button"
                onClick={() => setIsRestoreModalOpen(false)}
                className="text-[var(--ink-muted)] hover:text-[var(--ink)] p-1 rounded"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <form onSubmit={handleRestoreBackup} className="mt-4 text-xs dialog-form">
              <div className="dialog-body space-y-4">
                <div className="p-3 rounded-[var(--radius-md)] bg-rose-500/10 border border-rose-500/20 text-rose-600 text-[11px] flex items-start gap-2">
                  <AlertCircle className="w-4 h-4 shrink-0 mt-0.5" />
                  <span>
                    <strong>Perhatian:</strong> Memulihkan cadangan akan menimpa berkas situs dan skema database yang
                    ada saat ini. Pastikan Anda telah membuat cadangan terbaru sebelum melanjutkan.
                  </span>
                </div>

                <div>
                  <label className="block text-[var(--ink-muted)] font-medium mb-1">Berkas Cadangan</label>
                  <div className="px-3 py-2 rounded-[var(--radius-md)] bg-[var(--raised)] border border-[var(--line)] font-mono text-[11px] text-[var(--ink)]">
                    {selectedBackupForRestore.filename} ({formatBytes(selectedBackupForRestore.size_bytes)})
                  </div>
                </div>

                <div>
                  <label className="block text-[var(--ink-muted)] font-medium mb-1">Lokasi Pemulihan Berkas</label>
                  <div className="space-y-2">
                    <label
                      className={`flex items-start gap-2 p-2.5 rounded-[var(--radius-md)] border cursor-pointer ${
                        restoreTargetType === 'inplace'
                          ? 'border-[var(--primary)] bg-[var(--primary)]/5 text-[var(--ink)]'
                          : 'border-[var(--line)] bg-[var(--surface)] text-[var(--ink-muted)]'
                      }`}
                    >
                      <input
                        type="radio"
                        name="restoreTarget"
                        checked={restoreTargetType === 'inplace'}
                        onChange={() => setRestoreTargetType('inplace')}
                        className="mt-0.5"
                      />
                      <div>
                        <div className="font-semibold text-xs">Situs Utama (In-place)</div>
                        <div className="text-[10px] mt-0.5">Menimpa berkas situs langsung di folder aslinya</div>
                      </div>
                    </label>

                    <label
                      className={`flex items-start gap-2 p-2.5 rounded-[var(--radius-md)] border cursor-pointer ${
                        restoreTargetType === 'custom'
                          ? 'border-[var(--primary)] bg-[var(--primary)]/5 text-[var(--ink)]'
                          : 'border-[var(--line)] bg-[var(--surface)] text-[var(--ink-muted)]'
                      }`}
                    >
                      <input
                        type="radio"
                        name="restoreTarget"
                        checked={restoreTargetType === 'custom'}
                        onChange={() => setRestoreTargetType('custom')}
                        className="mt-0.5"
                      />
                      <div className="flex-1">
                        <div className="font-semibold text-xs">Folder Tujuan Kustom</div>
                        <div className="text-[10px] mt-0.5">
                          Ekstrak ke subfolder relatif (misal: staging atau test-restore)
                        </div>
                        {restoreTargetType === 'custom' && (
                          <input
                            type="text"
                            value={restoreCustomPath}
                            onChange={(e) => setRestoreCustomPath(e.target.value)}
                            placeholder="staging/restored-site"
                            required
                            className="mt-2 w-full px-2.5 py-1 rounded border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] font-mono text-[11px]"
                          />
                        )}
                      </div>
                    </label>
                  </div>
                </div>

                <div className="space-y-2 pt-2 border-t border-[var(--line)]">
                  <label className="block text-[var(--ink-muted)] font-medium">Komponen Yang Dipulihkan</label>
                  <label className="flex items-center gap-2 p-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer">
                    <input
                      type="checkbox"
                      checked={restoreFiles}
                      onChange={(e) => setRestoreFiles(e.target.checked)}
                      disabled={!selectedBackupForRestore.includes_files}
                      className="rounded text-[var(--primary)]"
                    />
                    <Folder className="w-3.5 h-3.5 text-emerald-500" />
                    <span className="font-medium text-[var(--ink)]">Pulihkan Berkas Situs</span>
                  </label>

                  <label className="flex items-center gap-2 p-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer">
                    <input
                      type="checkbox"
                      checked={restoreDatabases}
                      onChange={(e) => setRestoreDatabases(e.target.checked)}
                      disabled={!selectedBackupForRestore.includes_databases}
                      className="rounded text-[var(--primary)]"
                    />
                    <Database className="w-3.5 h-3.5 text-purple-500" />
                    <span className="font-medium text-[var(--ink)]">Pulihkan Basis Data</span>
                  </label>
                </div>
              </div>
              <div className="flex items-center justify-end gap-2 pt-3 border-t border-[var(--line)] dialog-footer">
                <button
                  type="button"
                  onClick={() => setIsRestoreModalOpen(false)}
                  disabled={isRestoring}
                  className="px-3.5 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] font-medium"
                >
                  Batal
                </button>
                <button
                  type="submit"
                  disabled={isRestoring}
                  className="px-4 py-1.5 rounded-[var(--radius-md)] bg-emerald-600 text-white hover:bg-emerald-700 font-medium flex items-center gap-1.5 shadow-xs"
                >
                  {isRestoring ? (
                    <>
                      <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      Memulihkan...
                    </>
                  ) : (
                    'Mulai Pemulihan'
                  )}
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* ===================================================================== */}
      {/* MODAL 3: SCHEDULE & RETENTION                                         */}
      {/* ===================================================================== */}
      {isScheduleModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4 backdrop-blur-xs"
          onDismiss={() => setIsScheduleModalOpen(false)}
        >
          <div className="w-full max-w-lg rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] p-6 shadow-xl max-h-[90vh] overflow-y-auto dialog-panel">
            <div className="flex items-center justify-between pb-3 border-b border-[var(--line)] dialog-header">
              <h3 className="text-sm font-semibold text-[var(--ink)] flex items-center gap-2">
                <Calendar className="w-4 h-4 text-indigo-500" />
                Pengaturan Jadwal Cadangan & Retensi
              </h3>
              <button
                type="button"
                onClick={() => setIsScheduleModalOpen(false)}
                className="text-[var(--ink-muted)] hover:text-[var(--ink)] p-1 rounded"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <form onSubmit={handleSaveSchedule} className="mt-4 text-xs dialog-form">
              <div className="dialog-body space-y-4">
                {/* Enable Toggle */}
                <div className="flex items-center justify-between p-3 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)]">
                  <div>
                    <div className="font-semibold text-xs text-[var(--ink)]">Aktifkan Pencadangan Otomatis</div>
                    <div className="text-[11px] text-[var(--ink-muted)] mt-0.5">
                      Jalankan pencadangan berkala di latar belakang sesuai frekuensi
                    </div>
                  </div>
                  <input
                    type="checkbox"
                    checked={scheduleEnabled}
                    onChange={(e) => setScheduleEnabled(e.target.checked)}
                    className="w-4 h-4 rounded text-[var(--primary)] cursor-pointer"
                  />
                </div>

                {scheduleEnabled && (
                  <>
                    {/* Frequency & Time */}
                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                      <div>
                        <label className="block text-[var(--ink-muted)] font-medium mb-1">Frekuensi Cadangan</label>
                        <select
                          value={scheduleFrequency}
                          onChange={(e) => setScheduleFrequency(e.target.value as BackupScheduleFrequency)}
                          className="w-full px-3 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                        >
                          <option value="daily">Harian (Setiap Hari)</option>
                          <option value="weekly">Mingguan (Setiap Minggu)</option>
                          <option value="monthly">Bulanan (Setiap Bulan)</option>
                        </select>
                      </div>

                      <div>
                        <label className="block text-[var(--ink-muted)] font-medium mb-1">
                          Waktu Eksekusi (Jam : Menit)
                        </label>
                        <div className="flex items-center gap-2">
                          <input
                            type="number"
                            min={0}
                            max={23}
                            value={scheduleHour}
                            onChange={(e) => setScheduleHour(Number(e.target.value))}
                            className="w-16 px-2 py-1.5 text-center rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] font-mono"
                            title="Jam (0-23)"
                          />
                          <span className="font-bold">:</span>
                          <input
                            type="number"
                            min={0}
                            max={59}
                            value={scheduleMinute}
                            onChange={(e) => setScheduleMinute(Number(e.target.value))}
                            className="w-16 px-2 py-1.5 text-center rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] font-mono"
                            title="Menit (0-59)"
                          />
                          <span className="text-[11px] text-[var(--ink-muted)]">WIB / Waktu Server</span>
                        </div>
                      </div>
                    </div>

                    {/* Retention Count */}
                    <div>
                      <label className="block text-[var(--ink-muted)] font-medium mb-1">Batas Retensi Cadangan</label>
                      <input
                        type="number"
                        min={1}
                        max={100}
                        value={scheduleRetention}
                        onChange={(e) => setScheduleRetention(Number(e.target.value))}
                        className="w-24 px-3 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] font-mono"
                      />
                      <p className="text-[11px] text-[var(--ink-muted)] mt-1">
                        Jumlah arsip terbaru yang disimpan. Arsip tertua akan dipruning otomatis ketika batas
                        tercapai.
                      </p>
                    </div>

                    {/* Content options */}
                    <div className="space-y-2 pt-2 border-t border-[var(--line)]">
                      <label className="block text-[var(--ink-muted)] font-medium">Komponen Cadangan Otomatis</label>
                      <div className="grid grid-cols-2 gap-2">
                        <label className="flex items-center gap-2 p-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] cursor-pointer">
                          <input
                            type="checkbox"
                            checked={scheduleIncludeFiles}
                            onChange={(e) => setScheduleIncludeFiles(e.target.checked)}
                            className="rounded text-[var(--primary)]"
                          />
                          <Folder className="w-3.5 h-3.5 text-emerald-500" />
                          <span className="text-[11px] font-medium text-[var(--ink)]">Berkas Situs</span>
                        </label>

                        <label className="flex items-center gap-2 p-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] cursor-pointer">
                          <input
                            type="checkbox"
                            checked={scheduleIncludeDatabases}
                            onChange={(e) => setScheduleIncludeDatabases(e.target.checked)}
                            className="rounded text-[var(--primary)]"
                          />
                          <Database className="w-3.5 h-3.5 text-purple-500" />
                          <span className="text-[11px] font-medium text-[var(--ink)]">Basis Data</span>
                        </label>
                      </div>
                    </div>

                    {/* Storage destination */}
                    <div className="pt-2 border-t border-[var(--line)]">
                      <label className="block text-[var(--ink-muted)] font-medium mb-1.5">
                        Tujuan Penyimpanan Arsip
                      </label>
                      <div className="grid grid-cols-2 gap-2">
                        <label
                          className={`flex items-start gap-2 p-2.5 rounded-[var(--radius-md)] border cursor-pointer ${
                            scheduleStorageType === 'local'
                              ? 'border-[var(--primary)] bg-[var(--primary)]/5 text-[var(--ink)]'
                              : 'border-[var(--line)] bg-[var(--surface)] text-[var(--ink-muted)]'
                          }`}
                        >
                          <input
                            type="radio"
                            name="storageType"
                            checked={scheduleStorageType === 'local'}
                            onChange={() => setScheduleStorageType('local')}
                            className="mt-0.5"
                          />
                          <div>
                            <div className="font-semibold text-xs">Penyimpanan Lokal</div>
                            <div className="text-[10px] mt-0.5">Tersimpan di direktori ~/backups host VM</div>
                          </div>
                        </label>

                        <label
                          className={`flex items-start gap-2 p-2.5 rounded-[var(--radius-md)] border cursor-pointer ${
                            scheduleStorageType === 's3'
                              ? 'border-[var(--primary)] bg-[var(--primary)]/5 text-[var(--ink)]'
                              : 'border-[var(--line)] bg-[var(--surface)] text-[var(--ink-muted)]'
                          }`}
                        >
                          <input
                            type="radio"
                            name="storageType"
                            checked={scheduleStorageType === 's3'}
                            onChange={() => setScheduleStorageType('s3')}
                            className="mt-0.5"
                          />
                          <div>
                            <div className="font-semibold text-xs">S3 Remote Storage</div>
                            <div className="text-[10px] mt-0.5">Kirim ke AWS S3, Cloudflare R2, MinIO</div>
                          </div>
                        </label>
                      </div>

                      {/* S3 configuration fields */}
                      {scheduleStorageType === 's3' && (
                        <div className="mt-3 p-3 rounded-[var(--radius-md)] bg-[var(--raised)] border border-[var(--line)] space-y-2.5">
                          <div>
                            <label className="block text-[var(--ink-muted)] font-medium mb-1">S3 Endpoint URL</label>
                            <input
                              type="text"
                              value={s3Endpoint}
                              onChange={(e) => setS3Endpoint(e.target.value)}
                              placeholder="https://s3.ap-southeast-1.amazonaws.com"
                              required={scheduleStorageType === 's3'}
                              className="w-full px-2.5 py-1 rounded border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)]"
                            />
                          </div>

                          <div className="grid grid-cols-2 gap-2">
                            <div>
                              <label className="block text-[var(--ink-muted)] font-medium mb-1">Bucket</label>
                              <input
                                type="text"
                                value={s3Bucket}
                                onChange={(e) => setS3Bucket(e.target.value)}
                                placeholder="my-backup-bucket"
                                required={scheduleStorageType === 's3'}
                                className="w-full px-2.5 py-1 rounded border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)]"
                              />
                            </div>

                            <div>
                              <label className="block text-[var(--ink-muted)] font-medium mb-1">Region</label>
                              <input
                                type="text"
                                value={s3Region}
                                onChange={(e) => setS3Region(e.target.value)}
                                placeholder="us-east-1"
                                className="w-full px-2.5 py-1 rounded border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)]"
                              />
                            </div>
                          </div>

                          <div className="grid grid-cols-2 gap-2">
                            <div>
                              <label className="block text-[var(--ink-muted)] font-medium mb-1">Access Key</label>
                              <input
                                type="text"
                                value={s3AccessKey}
                                onChange={(e) => setS3AccessKey(e.target.value)}
                                placeholder="AKIA..."
                                required={scheduleStorageType === 's3'}
                                className="w-full px-2.5 py-1 rounded border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] font-mono"
                              />
                            </div>

                            <div>
                              <label className="block text-[var(--ink-muted)] font-medium mb-1">Secret Key</label>
                              <input
                                type="password"
                                value={s3SecretKey}
                                onChange={(e) => setS3SecretKey(e.target.value)}
                                placeholder="Kunci rahasia S3"
                                className="w-full px-2.5 py-1 rounded border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] font-mono"
                              />
                            </div>
                          </div>

                          <div>
                            <label className="block text-[var(--ink-muted)] font-medium mb-1">
                              Prefix Folder (Opsional)
                            </label>
                            <input
                              type="text"
                              value={s3Prefix}
                              onChange={(e) => setS3Prefix(e.target.value)}
                              placeholder="vpanel/backups"
                              className="w-full px-2.5 py-1 rounded border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)]"
                            />
                          </div>
                        </div>
                      )}
                    </div>
                  </>
                )}
              </div>
              <div className="flex items-center justify-end gap-2 pt-3 border-t border-[var(--line)] dialog-footer">
                <button
                  type="button"
                  onClick={() => setIsScheduleModalOpen(false)}
                  disabled={isSavingSchedule}
                  className="px-3.5 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] font-medium"
                >
                  Batal
                </button>
                <button
                  type="submit"
                  disabled={isSavingSchedule}
                  className="px-4 py-1.5 rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 font-medium flex items-center gap-1.5 shadow-xs"
                >
                  {isSavingSchedule ? (
                    <>
                      <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      Menyimpan...
                    </>
                  ) : (
                    'Simpan Pengaturan'
                  )}
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* ===================================================================== */}
      {/* DIALOG 4: DELETE CONFIRMATION                                         */}
      {/* ===================================================================== */}
      {backupToDelete && (
        <ConfirmDangerDialog
          isOpen={true}
          title="Hapus Berkas Cadangan"
          description={`Apakah Anda yakin ingin menghapus arsip cadangan "${backupToDelete.filename}"? Berkas cadangan fisik di host dan basis data akan dihapus secara permanen.`}
          confirmMatchText={backupToDelete.filename}
          confirmButtonLabel="Hapus Cadangan"
          onConfirm={handleDeleteBackup}
          onClose={() => setBackupToDelete(null)}
          isLoading={isDeleting}
        />
      )}
    </div>
  );
};
