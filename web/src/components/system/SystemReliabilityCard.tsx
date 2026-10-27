import React, { useEffect, useState } from 'react';
import {
  fetchSystemDoctorReport,
  fixSystemDoctor,
  fetchSystemReconcileReport,
  applySystemReconcile,
  fetchSystemBackups,
  createSystemBackup,
  restoreSystemBackup,
} from '../../api/client';
import type {
  DoctorReport,
  ReconcileReport,
  SystemBackupSummary,
} from '../../types/api';
import {
  Activity,
  AlertTriangle,
  Archive,
  CheckCircle2,
  FileCheck2,
  FolderSync,
  HelpCircle,
  Play,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
  Wrench,
  XCircle,
} from 'lucide-react';

interface SystemReliabilityCardProps {
  onRefresh?: () => void;
}

export const SystemReliabilityCard: React.FC<SystemReliabilityCardProps> = ({
  onRefresh,
}) => {
  const [activeTab, setActiveTab] = useState<'doctor' | 'reconcile' | 'backup'>('doctor');
  const [loading, setLoading] = useState(false);
  const [actionLoading, setActionLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  // Doctor state
  const [doctorReport, setDoctorReport] = useState<DoctorReport | null>(null);

  // Reconcile state
  const [reconcileReport, setReconcileReport] = useState<ReconcileReport | null>(null);

  // Backup state
  const [backups, setBackups] = useState<SystemBackupSummary[]>([]);
  const [backupFilename, setBackupFilename] = useState('');

  const loadDoctor = async () => {
    try {
      setLoading(true);
      setError(null);
      const res = await fetchSystemDoctorReport();
      setDoctorReport(res);
      if (onRefresh) onRefresh();
    } catch (err: any) {
      setError(err?.message || 'Gagal memuat laporan diagnosis sistem.');
    } finally {
      setLoading(false);
    }
  };

  const loadReconcile = async () => {
    try {
      setLoading(true);
      setError(null);
      const res = await fetchSystemReconcileReport();
      setReconcileReport(res);
    } catch (err: any) {
      setError(err?.message || 'Gagal memuat status rekonsiliasi host.');
    } finally {
      setLoading(false);
    }
  };

  const loadBackups = async () => {
    try {
      setLoading(true);
      setError(null);
      const res = await fetchSystemBackups();
      setBackups(res);
    } catch (err: any) {
      setError(err?.message || 'Gagal memuat daftar cadangan panel.');
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (activeTab === 'doctor') {
      loadDoctor();
    } else if (activeTab === 'reconcile') {
      loadReconcile();
    } else if (activeTab === 'backup') {
      loadBackups();
    }
  }, [activeTab]);

  const handleFixDoctor = async () => {
    try {
      setActionLoading(true);
      setError(null);
      const res = await fixSystemDoctor();
      setDoctorReport(res);
      setSuccessMessage('Perbaikan sistem otomatis berhasil dijalankan.');
    } catch (err: any) {
      setError(err?.message || 'Gagal menjalankan perbaikan otomatis.');
    } finally {
      setActionLoading(false);
    }
  };

  const handleApplyReconcile = async () => {
    try {
      setActionLoading(true);
      setError(null);
      const res = await applySystemReconcile();
      setReconcileReport(res);
      setSuccessMessage('Rekonsiliasi konfigurasi host berhasil diselaraskan.');
    } catch (err: any) {
      setError(err?.message || 'Gagal menyelaraskan konfigurasi sistem.');
    } finally {
      setActionLoading(false);
    }
  };

  const handleCreateBackup = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      setActionLoading(true);
      setError(null);
      const res = await createSystemBackup(
        backupFilename.trim() ? { filename: backupFilename.trim() } : undefined
      );
      setSuccessMessage(`Cadangan sistem berhasil dibuat: ${res.filename}`);
      setBackupFilename('');
      await loadBackups();
    } catch (err: any) {
      setError(err?.message || 'Gagal membuat cadangan sistem baru.');
    } finally {
      setActionLoading(false);
    }
  };

  const handleRestoreBackup = async (filepath: string) => {
    if (!window.confirm(`Yakin ingin memulihkan snapshot sistem dari ${filepath}? Tindakan ini akan menimpa basis data yang ada.`)) {
      return;
    }
    try {
      setActionLoading(true);
      setError(null);
      const res = await restoreSystemBackup({ filepath });
      if (res.is_valid) {
        setSuccessMessage('Pemulihan cadangan sistem berhasil diverifikasi dan diterapkan.');
      } else {
        setError(`Pemulihan gagal: ${res.errors.join(', ')}`);
      }
    } catch (err: any) {
      setError(err?.message || 'Gagal memulihkan snapshot sistem.');
    } finally {
      setActionLoading(false);
    }
  };

  const formatBytes = (bytes: number) => {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`;
  };

  const formatTimestamp = (ts: number) => {
    if (!ts) return '-';
    return new Date(ts * 1000).toLocaleString('id-ID', {
      day: 'numeric',
      month: 'short',
      year: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  };

  return (
    <div className="bg-[var(--surface)] text-[var(--ink)] rounded-[var(--radius-md)] border border-[var(--line)] overflow-hidden" data-testid="system-reliability-card">
      {/* Header */}
      <div className="p-6 border-b border-[var(--line)] flex flex-col md:flex-row md:items-center justify-between gap-4">
        <div className="flex items-center gap-3">
          <div className="p-2.5 rounded-lg bg-[var(--primary)]/10 text-[var(--primary)]">
            <Activity className="w-6 h-6" />
          </div>
          <div>
            <h2 className="text-xl font-semibold tracking-tight text-[var(--ink)]">
              Keandalan Sistem & Diagnostik
            </h2>
            <p className="text-sm text-[var(--ink-muted)]">
              Diagnosis mandiri host, deteksi deviasi status (reconcile), dan pencadangan panel.
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={() => {
              if (activeTab === 'doctor') loadDoctor();
              if (activeTab === 'reconcile') loadReconcile();
              if (activeTab === 'backup') loadBackups();
            }}
            disabled={loading || actionLoading}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md border border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--surface-sunken)] text-[var(--ink)] transition-colors disabled:opacity-50"
            title="Segarkan data"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
            Segarkan
          </button>
        </div>
      </div>

      {/* Tabs */}
      <div className="settings-section-tabs flex flex-wrap border-b border-[var(--line)] bg-[var(--raised)] px-6 gap-2">
        <button
          onClick={() => setActiveTab('doctor')}
          className={`py-3 px-4 text-sm font-medium border-b-2 flex items-center gap-2 transition-colors ${
            activeTab === 'doctor'
              ? 'border-[var(--primary)] text-[var(--primary)]'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <Activity className="w-4 h-4" />
          Doctor (Diagnosis)
        </button>
        <button
          onClick={() => setActiveTab('reconcile')}
          className={`py-3 px-4 text-sm font-medium border-b-2 flex items-center gap-2 transition-colors ${
            activeTab === 'reconcile'
              ? 'border-[var(--primary)] text-[var(--primary)]'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <FolderSync className="w-4 h-4" />
          Rekonsiliasi Drift
        </button>
        <button
          onClick={() => setActiveTab('backup')}
          className={`py-3 px-4 text-sm font-medium border-b-2 flex items-center gap-2 transition-colors ${
            activeTab === 'backup'
              ? 'border-[var(--primary)] text-[var(--primary)]'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <Archive className="w-4 h-4" />
          Cadangan Panel
        </button>
      </div>

      {/* Alerts */}
      {error && (
        <div className="m-6 p-4 rounded-lg bg-[var(--danger)]/10 border border-[var(--danger)]/20 text-[var(--danger)] text-sm flex items-start gap-3">
          <ShieldAlert className="w-5 h-5 shrink-0 mt-0.5" />
          <div className="flex-1">{error}</div>
        </div>
      )}

      {successMessage && (
        <div className="m-6 p-4 rounded-lg bg-emerald-500/10 border border-emerald-500/20 text-emerald-600 dark:text-emerald-400 text-sm flex items-start gap-3">
          <CheckCircle2 className="w-5 h-5 shrink-0 mt-0.5" />
          <div className="flex-1">{successMessage}</div>
        </div>
      )}

      {/* Tab 1: Doctor */}
      {activeTab === 'doctor' && (
        <div className="p-6 space-y-6">
          {doctorReport && (
            <>
              {/* Summary Metrics */}
              <div className="grid grid-cols-1 sm:grid-cols-4 gap-4">
                <div className="p-4 rounded-lg bg-[var(--surface)] border border-[var(--line)]">
                  <div className="text-xs text-[var(--ink-muted)] font-medium uppercase tracking-wider mb-1">
                    Status Host
                  </div>
                  <div className="flex items-center gap-2">
                    {doctorReport.overall_status === 'pass' && (
                      <>
                        <ShieldCheck className="w-5 h-5 text-emerald-500" />
                        <span className="text-lg font-bold text-emerald-600 dark:text-emerald-400">
                          Sehat (Pass)
                        </span>
                      </>
                    )}
                    {doctorReport.overall_status === 'warn' && (
                      <>
                        <AlertTriangle className="w-5 h-5 text-amber-500" />
                        <span className="text-lg font-bold text-amber-600 dark:text-amber-400">
                          Peringatan (Warn)
                        </span>
                      </>
                    )}
                    {doctorReport.overall_status === 'fail' && (
                      <>
                        <XCircle className="w-5 h-5 text-[var(--danger)]" />
                        <span className="text-lg font-bold text-[var(--danger)]">
                          Bermasalah (Fail)
                        </span>
                      </>
                    )}
                  </div>
                </div>

                <div className="p-4 rounded-lg bg-[var(--surface)] border border-[var(--line)]">
                  <div className="text-xs text-[var(--ink-muted)] font-medium uppercase tracking-wider mb-1">
                    Pemeriksaan Lolos
                  </div>
                  <div className="text-2xl font-bold text-[var(--ink)]">
                    {doctorReport.total_passed}
                  </div>
                </div>

                <div className="p-4 rounded-lg bg-[var(--surface)] border border-[var(--line)]">
                  <div className="text-xs text-[var(--ink-muted)] font-medium uppercase tracking-wider mb-1">
                    Peringatan
                  </div>
                  <div className="text-2xl font-bold text-amber-600 dark:text-amber-400">
                    {doctorReport.total_warnings}
                  </div>
                </div>

                <div className="p-4 rounded-lg bg-[var(--surface)] border border-[var(--line)]">
                  <div className="text-xs text-[var(--ink-muted)] font-medium uppercase tracking-wider mb-1">
                    Kegagalan Kritis
                  </div>
                  <div className="text-2xl font-bold text-[var(--danger)]">
                    {doctorReport.total_failures}
                  </div>
                </div>
              </div>

              {/* Action Bar */}
              <div className="flex flex-wrap items-center justify-between gap-3 p-4 rounded-lg bg-[var(--raised)] border border-[var(--line)]">
                <div className="text-xs text-[var(--ink-muted)]">
                  Terakhir diperiksa: {formatTimestamp(doctorReport.timestamp)}
                </div>
                <button
                  onClick={handleFixDoctor}
                  disabled={actionLoading}
                  className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-md bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] transition-colors disabled:opacity-50"
                  data-testid="doctor-fix-button"
                >
                  <Wrench className="w-4 h-4" />
                  {actionLoading ? 'Memperbaiki...' : 'Perbaiki Otomatis (--fix)'}
                </button>
              </div>

              {/* Checks Checklist */}
              <div className="space-y-3">
                <h3 className="text-sm font-semibold text-[var(--ink)] uppercase tracking-wider">
                  Daftar Parameter Kesehatan Sistem
                </h3>
                <div className="divide-y divide-[var(--line)] rounded-lg border border-[var(--line)] overflow-hidden">
                  {doctorReport.checks.map((check) => (
                    <div
                      key={check.id}
                      className="p-4 flex flex-col sm:flex-row sm:items-center justify-between gap-3 hover:bg-[var(--raised)] transition-colors"
                      data-testid={`doctor-check-${check.id}`}
                    >
                      <div className="space-y-1">
                        <div className="flex items-center gap-2">
                          {check.status === 'pass' && (
                            <CheckCircle2 className="w-4 h-4 text-emerald-500 shrink-0" />
                          )}
                          {check.status === 'warn' && (
                            <AlertTriangle className="w-4 h-4 text-amber-500 shrink-0" />
                          )}
                          {check.status === 'fail' && (
                            <XCircle className="w-4 h-4 text-[var(--danger)] shrink-0" />
                          )}
                          <span className="font-medium text-sm text-[var(--ink)]">
                            {check.name}
                          </span>
                          <span className="text-[10px] uppercase px-1.5 py-0.5 rounded bg-[var(--raised)] text-[var(--ink-muted)] font-mono">
                            {check.category}
                          </span>
                        </div>
                        <p className="text-xs text-[var(--ink-muted)] pl-6">
                          {check.message}
                        </p>
                        {check.remediation_hint && (
                          <div className="text-xs text-amber-600 dark:text-amber-400 pl-6 flex items-center gap-1.5">
                            <HelpCircle className="w-3.5 h-3.5" />
                            Petunjuk: {check.remediation_hint}
                          </div>
                        )}
                      </div>

                      <div className="flex items-center gap-2 sm:self-center pl-6 sm:pl-0">
                        {check.fixable && (
                          <span className="text-[11px] font-medium px-2 py-0.5 rounded bg-[var(--primary)]/10 text-[var(--primary)]">
                            Dapat Diperbaiki
                          </span>
                        )}
                        <span
                          className={`text-xs font-semibold px-2.5 py-1 rounded-full uppercase tracking-wider ${
                            check.status === 'pass'
                              ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
                              : check.status === 'warn'
                              ? 'bg-amber-500/10 text-amber-600 dark:text-amber-400'
                              : 'bg-[var(--danger)]/10 text-[var(--danger)]'
                          }`}
                        >
                          {check.status}
                        </span>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </>
          )}
        </div>
      )}

      {/* Tab 2: Reconcile */}
      {activeTab === 'reconcile' && (
        <div className="p-6 space-y-6">
          {reconcileReport && (
            <>
              {/* Reconcile Metric Summary */}
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
                <div className="p-4 rounded-lg bg-[var(--surface)] border border-[var(--line)]">
                  <div className="text-xs text-[var(--ink-muted)] font-medium uppercase tracking-wider mb-1">
                    Status Sinkronisasi
                  </div>
                  <div className="flex items-center gap-2">
                    {reconcileReport.is_clean ? (
                      <>
                        <CheckCircle2 className="w-5 h-5 text-emerald-500" />
                        <span className="text-lg font-bold text-emerald-600 dark:text-emerald-400">
                          Sinkron (Bersih)
                        </span>
                      </>
                    ) : (
                      <>
                        <AlertTriangle className="w-5 h-5 text-amber-500" />
                        <span className="text-lg font-bold text-amber-600 dark:text-amber-400">
                          Ada Deviasi (Drift)
                        </span>
                      </>
                    )}
                  </div>
                </div>

                <div className="p-4 rounded-lg bg-[var(--surface)] border border-[var(--line)]">
                  <div className="text-xs text-[var(--ink-muted)] font-medium uppercase tracking-wider mb-1">
                    Sumber Daya Terverifikasi
                  </div>
                  <div className="text-2xl font-bold text-[var(--ink)]">
                    {reconcileReport.total_resources_checked}
                  </div>
                </div>

                <div className="p-4 rounded-lg bg-[var(--surface)] border border-[var(--line)]">
                  <div className="text-xs text-[var(--ink-muted)] font-medium uppercase tracking-wider mb-1">
                    Total Deviasi
                  </div>
                  <div className="text-2xl font-bold text-amber-600 dark:text-amber-400">
                    {reconcileReport.drifts.length}
                  </div>
                </div>
              </div>

              {/* Action Bar */}
              <div className="flex flex-wrap items-center justify-between gap-3 p-4 rounded-lg bg-[var(--raised)] border border-[var(--line)]">
                <div className="text-xs text-[var(--ink-muted)]">
                  Terakhir dikalkulasi: {formatTimestamp(reconcileReport.timestamp)}
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  <button
                    onClick={loadReconcile}
                    disabled={actionLoading || loading}
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md border border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--surface-sunken)] text-[var(--ink)] transition-colors disabled:opacity-50"
                  >
                    Deteksi Deviasi (Dry Run)
                  </button>
                  <button
                    onClick={handleApplyReconcile}
                    disabled={actionLoading || reconcileReport.is_clean}
                    className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-md bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] transition-colors disabled:opacity-50"
                    data-testid="reconcile-apply-button"
                  >
                    <Play className="w-4 h-4" />
                    {actionLoading ? 'Menyelaraskan...' : 'Terapkan Rekonsiliasi Otomatis'}
                  </button>
                </div>
              </div>

              {/* Drifts Table */}
              <div className="space-y-3">
                <h3 className="text-sm font-semibold text-[var(--ink)] uppercase tracking-wider">
                  Daftar Deviasi Status (Drift Items)
                </h3>
                {reconcileReport.drifts.length === 0 ? (
                  <div className="p-8 text-center rounded-lg border border-dashed border-[var(--line)] text-[var(--ink-muted)]">
                    <CheckCircle2 className="w-8 h-8 text-emerald-500 mx-auto mb-2" />
                    <p className="font-medium text-[var(--ink)]">Semua status host dan basis data telah sinkron.</p>
                    <p className="text-xs mt-1">Tidak ditemukan berkas vhost yatim, konfigurasi hilang, atau izin docroot yang tidak selaras.</p>
                  </div>
                ) : (
                  <div className="divide-y divide-[var(--line)] rounded-lg border border-[var(--line)] overflow-hidden">
                    {reconcileReport.drifts.map((drift) => (
                      <div key={drift.id} className="p-4 space-y-2 hover:bg-[var(--raised)] transition-colors">
                        <div className="flex items-center justify-between">
                          <span className="font-mono text-xs font-semibold px-2 py-0.5 rounded bg-amber-500/10 text-amber-600 dark:text-amber-400">
                            {drift.drift_type}
                          </span>
                          <span className="text-xs font-medium text-[var(--ink-muted)]">
                            {drift.resource_type}: {drift.resource_id}
                          </span>
                        </div>
                        <p className="text-sm text-[var(--ink)]">{drift.description}</p>
                        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-xs pt-1">
                          <div className="p-2 rounded bg-[var(--raised)]">
                            <span className="text-[var(--ink-muted)] font-medium">Diharapkan: </span>
                            <span className="text-[var(--ink)]">{drift.desired_state}</span>
                          </div>
                          <div className="p-2 rounded bg-[var(--raised)]">
                            <span className="text-[var(--ink-muted)] font-medium">Aktual di host: </span>
                            <span className="text-[var(--danger)] font-mono">{drift.actual_state}</span>
                          </div>
                        </div>
                        <div className="text-xs text-[var(--primary)] font-medium flex items-center gap-1.5 pt-1">
                          <Wrench className="w-3.5 h-3.5" />
                          Aksi Pemulihan: {drift.remediation_action}
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </>
          )}
        </div>
      )}

      {/* Tab 3: Backups */}
      {activeTab === 'backup' && (
        <div className="p-6 space-y-6">
          {/* Create Backup Form */}
          <p className="text-sm text-[var(--ink-muted)]">Snapshot ini hanya untuk database/config panel. Untuk pindah server beserta website dan database aplikasi, gunakan <a className="underline" href="/settings?section=server_backup">Backup dan migrasi</a>.</p>
          <form onSubmit={handleCreateBackup} className="p-4 rounded-lg bg-[var(--raised)] border border-[var(--line)] flex flex-col sm:flex-row gap-3 items-stretch sm:items-center">
            <div className="flex-1">
              <input
                type="text"
                placeholder="Nama berkas cadangan opsional (mis. panel-backup-manual.tar.gz)"
                value={backupFilename}
                onChange={(e) => setBackupFilename(e.target.value)}
                className="w-full px-3 py-2 text-sm rounded-md border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)]"
              />
            </div>
            <button
              type="submit"
              disabled={actionLoading}
              className="inline-flex items-center justify-center gap-2 px-4 py-2 text-sm font-medium rounded-md bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] transition-colors disabled:opacity-50 shrink-0"
              data-testid="create-backup-button"
            >
              <Archive className="w-4 h-4" />
              {actionLoading ? 'Membuat...' : 'Buat Snapshot Cadangan'}
            </button>
          </form>

          {/* Backup List */}
          <div className="space-y-3">
            <h3 className="text-sm font-semibold text-[var(--ink)] uppercase tracking-wider">
              Daftar Snapshot Cadangan Panel
            </h3>

            {backups.length === 0 ? (
              <div className="p-8 text-center rounded-lg border border-dashed border-[var(--line)] text-[var(--ink-muted)]">
                <Archive className="w-8 h-8 text-[var(--ink-muted)] mx-auto mb-2" />
                <p className="font-medium text-[var(--ink)]">Belum ada snapshot cadangan sistem.</p>
                <p className="text-xs mt-1">Buat cadangan pertama untuk mengamankan SQLite panel dan konfigurasi server.</p>
              </div>
            ) : (
              <div className="divide-y divide-[var(--line)] rounded-lg border border-[var(--line)] overflow-hidden">
                {backups.map((b) => (
                  <div
                    key={b.filename}
                    className="p-4 flex flex-col md:flex-row md:items-center justify-between gap-4 hover:bg-[var(--raised)] transition-colors"
                  >
                    <div className="space-y-1">
                      <div className="flex items-center gap-2">
                        <FileCheck2 className="w-4 h-4 text-[var(--primary)] shrink-0" />
                        <span className="font-mono text-sm font-semibold text-[var(--ink)]">
                          {b.filename}
                        </span>
                        {b.verified && (
                          <span className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-600 dark:text-emerald-400">
                            Terverifikasi
                          </span>
                        )}
                      </div>
                      <p className="text-xs text-[var(--ink-muted)] font-mono truncate max-w-xl">
                        {b.filepath}
                      </p>
                      <div className="flex flex-wrap items-center gap-4 text-xs text-[var(--ink-muted)] pt-1">
                        <span>Ukuran: {formatBytes(b.size_bytes)}</span>
                        <span>Dibuat: {formatTimestamp(b.created_at)}</span>
                        {b.checksum_sha256 && (
                          <span className="font-mono truncate max-w-xs" title={b.checksum_sha256}>
                            SHA256: {b.checksum_sha256.substring(0, 12)}...
                          </span>
                        )}
                      </div>
                    </div>

                    <div className="flex items-center gap-2 shrink-0">
                      <button
                        onClick={() => handleRestoreBackup(b.filepath)}
                        disabled={actionLoading}
                        className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md border border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--surface-sunken)] text-[var(--ink)] transition-colors disabled:opacity-50"
                      >
                        <RefreshCw className="w-3.5 h-3.5" />
                        Pulihkan
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};
