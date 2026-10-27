import React, { useState, useEffect, useCallback } from 'react';
import {
  Database,
  HardDrive,
  Trash2,
  CheckCircle2,
  AlertTriangle,
  AlertOctagon,
  RefreshCw,
  Settings2,
  ChevronDown,
  ChevronUp,
  ShieldAlert,
  Sparkles,
  Info,
} from 'lucide-react';
import {
  fetchAnalyticsRetention,
  updateAnalyticsRetention,
  triggerAnalyticsMaintenance,
  checkAnalyticsIntegrity,
} from '../../api/client';
import {
  AnalyticsRetentionResponse,
  RetentionMaintenanceReportDto,
  IntegrityCheckReportDto,
} from '../../types/api';

export interface AnalyticsRetentionCardProps {
  className?: string;
  initialData?: AnalyticsRetentionResponse;
  onRefreshParent?: () => void;
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const val = bytes / Math.pow(1024, i);
  return `${val.toFixed(i === 0 ? 0 : 2)} ${units[i]}`;
}

function formatNumber(num: number): string {
  return new Intl.NumberFormat('id-ID').format(num);
}

function formatTimestamp(ts: number): string {
  if (!ts) return '-';
  const d = new Date(ts * 1000);
  return d.toLocaleString('id-ID', {
    dateStyle: 'medium',
    timeStyle: 'medium',
  });
}

export const AnalyticsRetentionCard: React.FC<AnalyticsRetentionCardProps> = ({
  className = '',
  initialData,
  onRefreshParent,
}) => {
  const [data, setData] = useState<AnalyticsRetentionResponse | null>(initialData ?? null);
  const [loading, setLoading] = useState<boolean>(initialData === undefined);
  const [error, setError] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);

  // Form State
  const [rawHours, setRawHours] = useState<number>(() =>
    initialData ? Math.max(1, Math.round(initialData.config.raw_retention_secs / 3600)) : 2
  );
  const [rollup1mDays, setRollup1mDays] = useState<number>(() =>
    initialData ? Math.max(1, Math.round(initialData.config.rollup_1m_retention_secs / 86400)) : 14
  );
  const [rollup1hDays, setRollup1hDays] = useState<number>(() =>
    initialData ? Math.max(1, Math.round(initialData.config.rollup_1h_retention_secs / 86400)) : 400
  );
  const [rollup1dDays, setRollup1dDays] = useState<string>(() =>
    initialData?.config.rollup_1d_retention_secs
      ? String(Math.round(initialData.config.rollup_1d_retention_secs / 86400))
      : ''
  );
  const [securityDays, setSecurityDays] = useState<number>(() =>
    initialData ? Math.max(1, Math.round(initialData.config.security_events_retention_secs / 86400)) : 7
  );
  const [maxDbMb, setMaxDbMb] = useState<string>(() =>
    initialData?.config.max_db_size_bytes
      ? String(Math.round(initialData.config.max_db_size_bytes / (1024 * 1024)))
      : ''
  );
  const [autoVacuum, setAutoVacuum] = useState<boolean>(() =>
    initialData ? initialData.config.auto_vacuum : true
  );
  const [savingConfig, setSavingConfig] = useState<boolean>(false);

  // Operational State
  const [forceVacuum, setForceVacuum] = useState<boolean>(false);
  const [runningMaintenance, setRunningMaintenance] = useState<boolean>(false);
  const [maintenanceReport, setMaintenanceReport] = useState<RetentionMaintenanceReportDto | null>(
    () => initialData?.last_report ?? null
  );

  const [checkingIntegrity, setCheckingIntegrity] = useState<boolean>(false);
  const [integrityReport, setIntegrityReport] = useState<IntegrityCheckReportDto | null>(
    () => initialData?.integrity ?? null
  );

  const [tablesExpanded, setTablesExpanded] = useState<boolean>(false);

  const syncFormFromData = (res: AnalyticsRetentionResponse) => {
    setRawHours(Math.max(1, Math.round(res.config.raw_retention_secs / 3600)));
    setRollup1mDays(Math.max(1, Math.round(res.config.rollup_1m_retention_secs / 86400)));
    setRollup1hDays(Math.max(1, Math.round(res.config.rollup_1h_retention_secs / 86400)));
    if (res.config.rollup_1d_retention_secs) {
      setRollup1dDays(String(Math.round(res.config.rollup_1d_retention_secs / 86400)));
    } else {
      setRollup1dDays('');
    }
    setSecurityDays(Math.max(1, Math.round(res.config.security_events_retention_secs / 86400)));
    if (res.config.max_db_size_bytes) {
      setMaxDbMb(String(Math.round(res.config.max_db_size_bytes / (1024 * 1024))));
    } else {
      setMaxDbMb('');
    }
    setAutoVacuum(res.config.auto_vacuum);
    if (res.last_report) {
      setMaintenanceReport(res.last_report);
    }
    if (res.integrity) {
      setIntegrityReport(res.integrity);
    }
  };

  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetchAnalyticsRetention();
      setData(res);
      syncFormFromData(res);
    } catch (err: unknown) {
      const e = err as Error;
      setError(e.message || 'Gagal memuat status retensi analitik');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (initialData === undefined) {
      loadData();
    }
  }, [loadData, initialData]);

  const handleSaveConfig = async (e: React.FormEvent) => {
    e.preventDefault();
    setSavingConfig(true);
    setError(null);
    setSuccessMsg(null);
    try {
      const req = {
        raw_retention_secs: rawHours * 3600,
        rollup_1m_retention_secs: rollup1mDays * 86400,
        rollup_1h_retention_secs: rollup1hDays * 86400,
        rollup_1d_retention_secs: rollup1dDays.trim() ? Number(rollup1dDays) * 86400 : null,
        security_events_retention_secs: securityDays * 86400,
        max_db_size_bytes: maxDbMb.trim() ? Number(maxDbMb) * 1024 * 1024 : null,
        auto_vacuum: autoVacuum,
      };
      const res = await updateAnalyticsRetention(req);
      setData(res);
      syncFormFromData(res);
      setSuccessMsg('Konfigurasi retensi berhasil disimpan');
      if (onRefreshParent) onRefreshParent();
    } catch (err: unknown) {
      const e = err as Error;
      setError(e.message || 'Gagal menyimpan konfigurasi retensi');
    } finally {
      setSavingConfig(false);
    }
  };

  const handleTriggerMaintenance = async () => {
    setRunningMaintenance(true);
    setError(null);
    setSuccessMsg(null);
    try {
      const rep = await triggerAnalyticsMaintenance({
        force_vacuum: forceVacuum,
      });
      setMaintenanceReport(rep);
      setSuccessMsg(
        `Pemeliharaan selesai: ${formatBytes(rep.bytes_reclaimed)} dibebaskan (${formatNumber(
          rep.pruned_raw_records +
            rep.pruned_rollup_1m_records +
            rep.pruned_rollup_1h_records +
            rep.pruned_rollup_1d_records +
            rep.pruned_security_events +
            rep.pruned_stale_tailer_records
        )} baris dipangkas)`
      );
      // Refresh storage metrics
      const updated = await fetchAnalyticsRetention();
      setData(updated);
      syncFormFromData(updated);
      if (onRefreshParent) onRefreshParent();
    } catch (err: unknown) {
      const e = err as Error;
      setError(e.message || 'Gagal menjalankan pemeliharaan basis data');
    } finally {
      setRunningMaintenance(false);
    }
  };

  const handleCheckIntegrity = async () => {
    setCheckingIntegrity(true);
    setError(null);
    try {
      const rep = await checkAnalyticsIntegrity();
      setIntegrityReport(rep);
      if (rep.is_healthy) {
        setSuccessMsg('Pemeriksaan integritas SQLite selesai: Basis data dalam kondisi prima (Sehat)');
      } else {
        setError(`Ditemukan masalah integritas: ${rep.errors.join('; ')}`);
      }
    } catch (err: unknown) {
      const e = err as Error;
      setError(e.message || 'Gagal memeriksa integritas basis data');
    } finally {
      setCheckingIntegrity(false);
    }
  };

  return (
    <div
      className={`p-5 bg-[var(--surface)] border border-[var(--line)] rounded-2xl shadow-xs space-y-6 ${className}`}
    >
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 pb-4 border-b border-[var(--line)]">
        <div>
          <div className="flex items-center gap-2">
            <Database className="w-5 h-5 text-[var(--primary)]" />
            <h2 className="text-base font-semibold text-[var(--ink)]">
              Siklus Data & Retensi Basis Data
            </h2>
          </div>
          <p className="text-xs text-[var(--ink-muted)] mt-1">
            Pengelolaan retensi multi-resolusi (mentah, 1m, 1h, 1d), batas penyimpanan disk, pemangkasan otomatis, dan pemadatan SQLite
          </p>
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={loadData}
            disabled={loading}
            className="px-3 py-1.5 rounded-lg border border-[var(--line)] bg-[var(--bg)] text-xs font-medium text-[var(--ink)] hover:bg-[var(--surface-hover)] transition-colors flex items-center gap-1.5"
            title="Segarkan data"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin text-[var(--primary)]' : ''}`} />
            Segarkan
          </button>
        </div>
      </div>

      {/* Alerts */}
      {error && (
        <div role="alert" className="p-3.5 bg-rose-500/10 border border-rose-500/20 rounded-xl text-xs text-rose-500 flex items-start gap-2.5">
          <AlertTriangle className="w-4 h-4 shrink-0 mt-0.5" />
          <div className="flex-1">{error}</div>
        </div>
      )}

      {successMsg && (
        <div className="p-3.5 bg-emerald-500/10 border border-emerald-500/20 rounded-xl text-xs text-emerald-600 dark:text-emerald-400 flex items-start gap-2.5">
          <CheckCircle2 className="w-4 h-4 shrink-0 mt-0.5" />
          <div className="flex-1">{successMsg}</div>
        </div>
      )}

      {/* Storage Summary Cards */}
      {data && (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
          {/* Main DB Size */}
          <div className="p-4 bg-[var(--bg)] border border-[var(--line)] rounded-xl space-y-1">
            <div className="flex items-center justify-between text-xs text-[var(--ink-muted)]">
              <span>Ukuran Basis Data</span>
              <HardDrive className="w-4 h-4 text-[var(--primary)]" />
            </div>
            <div className="text-lg font-bold text-[var(--ink)] font-mono">
              {formatBytes(data.storage.main_db_size_bytes)}
            </div>
            <div className="text-[11px] text-[var(--ink-muted)] flex items-center justify-between">
              <span>WAL: {formatBytes(data.storage.wal_size_bytes)}</span>
              <span>Total: {formatBytes(data.storage.total_size_bytes)}</span>
            </div>
          </div>

          {/* SQLite Pages & Freelist */}
          <div className="p-4 bg-[var(--bg)] border border-[var(--line)] rounded-xl space-y-1">
            <div className="flex items-center justify-between text-xs text-[var(--ink-muted)]">
              <span>Halaman & Ruang Bebas</span>
              <Database className="w-4 h-4 text-[var(--ink-muted)]" />
            </div>
            <div className="text-lg font-bold text-[var(--ink)] font-mono">
              {formatBytes(data.storage.free_bytes)}
            </div>
            <div className="text-[11px] text-[var(--ink-muted)] flex items-center justify-between">
              <span>{formatNumber(data.storage.freelist_count)} halaman bebas</span>
              <span>Ukuran per page: {data.storage.page_size} B</span>
            </div>
          </div>

          {/* Fragmentation Ratio */}
          <div className="p-4 bg-[var(--bg)] border border-[var(--line)] rounded-xl space-y-1">
            <div className="flex items-center justify-between text-xs text-[var(--ink-muted)]">
              <span>Rasio Fragmentasi</span>
              <Sparkles className="w-4 h-4 text-amber-500" />
            </div>
            <div className="text-lg font-bold text-[var(--ink)] font-mono">
              {(data.storage.fragmentation_ratio * 100).toFixed(1)}%
            </div>
            <div className="text-[11px] text-[var(--ink-muted)]">
              {data.storage.fragmentation_ratio > 0.2 ? (
                <span className="text-amber-500 font-medium">&gt; 20% (Disarankan VACUUM)</span>
              ) : (
                <span className="text-emerald-500 font-medium">Optimal (&le; 20%)</span>
              )}
            </div>
          </div>

          {/* Integrity Health Status */}
          <div className="p-4 bg-[var(--bg)] border border-[var(--line)] rounded-xl space-y-1">
            <div className="flex items-center justify-between text-xs text-[var(--ink-muted)]">
              <span>Status Integritas SQLite</span>
              <ShieldAlert className="w-4 h-4 text-emerald-500" />
            </div>
            <div className="flex items-center gap-2 pt-0.5">
              {integrityReport ? (
                integrityReport.is_healthy ? (
                  <span className="px-2.5 py-1 text-xs font-semibold rounded-full bg-emerald-500/10 text-emerald-500 border border-emerald-500/20 flex items-center gap-1.5">
                    <CheckCircle2 className="w-3.5 h-3.5" />
                    Sehat (Valid)
                  </span>
                ) : (
                  <span className="px-2.5 py-1 text-xs font-semibold rounded-full bg-rose-500/10 text-rose-500 border border-rose-500/20 flex items-center gap-1.5">
                    <AlertOctagon className="w-3.5 h-3.5" />
                    Korup ({integrityReport.errors.length} Isu)
                  </span>
                )
              ) : (
                <span className="px-2.5 py-1 text-xs font-semibold rounded-full bg-blue-500/10 text-blue-500 border border-blue-500/20">
                  Belum Diuji
                </span>
              )}
            </div>
            <div className="text-[11px] text-[var(--ink-muted)] truncate">
              {integrityReport?.checked_at
                ? `Diuji: ${formatTimestamp(integrityReport.checked_at)}`
                : 'Uji manual melalui tombol di bawah'}
            </div>
          </div>
        </div>
      )}

      {/* Last Maintenance Report Summary */}
      {maintenanceReport && (
        <div className="p-4 bg-[var(--bg)] border border-[var(--line)] rounded-xl space-y-2 text-xs">
          <div className="flex items-center justify-between font-semibold text-[var(--ink)]">
            <div className="flex items-center gap-2">
              <Info className="w-4 h-4 text-[var(--primary)]" />
              <span>Ringkasan Pembersihan Terakhir</span>
              {maintenanceReport.is_emergency_pruning && (
                <span className="px-2 py-0.5 rounded text-[10px] font-bold bg-rose-500/10 text-rose-500 border border-rose-500/20">
                  DARURAT (Batas Disk)
                </span>
              )}
            </div>
            <span className="text-[var(--ink-muted)] text-[11px]">
              {formatTimestamp(maintenanceReport.completed_at)}
            </span>
          </div>

          <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 pt-1 text-[var(--ink-muted)]">
            <div>
              <span className="block text-[10px] uppercase font-bold text-[var(--ink)]">Ruang Bebas</span>
              <span className="font-mono text-emerald-500 font-semibold">
                {formatBytes(maintenanceReport.bytes_reclaimed)}
              </span>
            </div>
            <div>
              <span className="block text-[10px] uppercase font-bold text-[var(--ink)]">Data Mentah Dipangkas</span>
              <span className="font-mono">{formatNumber(maintenanceReport.pruned_raw_records)} baris</span>
            </div>
            <div>
              <span className="block text-[10px] uppercase font-bold text-[var(--ink)]">Rollup 1m / 1h Dipangkas</span>
              <span className="font-mono">
                {formatNumber(maintenanceReport.pruned_rollup_1m_records + maintenanceReport.pruned_rollup_1h_records)} baris
              </span>
            </div>
            <div>
              <span className="block text-[10px] uppercase font-bold text-[var(--ink)]">Keamanan & Tailer</span>
              <span className="font-mono">
                {formatNumber(
                  maintenanceReport.pruned_security_events + maintenanceReport.pruned_stale_tailer_records
                )} baris
              </span>
            </div>
          </div>
        </div>
      )}

      {/* Main Grid: Settings & Operations */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6 pt-2">
        {/* Retention Policy Form */}
        <form
          onSubmit={handleSaveConfig}
          className="p-4 bg-[var(--bg)] border border-[var(--line)] rounded-xl space-y-4"
        >
          <div className="flex items-center gap-2 pb-2 border-b border-[var(--line)]">
            <Settings2 className="w-4 h-4 text-[var(--primary)]" />
            <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--ink)]">
              Konfigurasi Kebijakan Retensi
            </h3>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 text-xs">
            <div>
              <label htmlFor="retention-raw-hours" className="block text-[11px] font-semibold text-[var(--ink)] mb-1">
                Data Mentah (10 Detik)
              </label>
              <div className="flex items-center gap-1.5">
                <input
                  id="retention-raw-hours"
                  type="number"
                  min="1"
                  max="168"
                  value={rawHours}
                  onChange={(e) => setRawHours(Number(e.target.value))}
                  className="w-full px-2.5 py-1.5 rounded-lg border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                />
                <span className="text-[var(--ink-muted)] text-[11px]">Jam</span>
              </div>
              <p className="text-[10px] text-[var(--ink-muted)] mt-0.5">Bawaan: 2 jam (resolusi 10s)</p>
            </div>

            <div>
              <label htmlFor="retention-rollup-1m" className="block text-[11px] font-semibold text-[var(--ink)] mb-1">
                Rollup 1 Menit
              </label>
              <div className="flex items-center gap-1.5">
                <input
                  id="retention-rollup-1m"
                  type="number"
                  min="1"
                  max="90"
                  value={rollup1mDays}
                  onChange={(e) => setRollup1mDays(Number(e.target.value))}
                  className="w-full px-2.5 py-1.5 rounded-lg border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                />
                <span className="text-[var(--ink-muted)] text-[11px]">Hari</span>
              </div>
              <p className="text-[10px] text-[var(--ink-muted)] mt-0.5">Bawaan: 14 hari</p>
            </div>

            <div>
              <label htmlFor="retention-rollup-1h" className="block text-[11px] font-semibold text-[var(--ink)] mb-1">
                Rollup 1 Jam
              </label>
              <div className="flex items-center gap-1.5">
                <input
                  id="retention-rollup-1h"
                  type="number"
                  min="30"
                  max="1000"
                  value={rollup1hDays}
                  onChange={(e) => setRollup1hDays(Number(e.target.value))}
                  className="w-full px-2.5 py-1.5 rounded-lg border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                />
                <span className="text-[var(--ink-muted)] text-[11px]">Hari</span>
              </div>
              <p className="text-[10px] text-[var(--ink-muted)] mt-0.5">Bawaan: 400 hari (~13 bulan)</p>
            </div>

            <div>
              <label htmlFor="retention-security-days" className="block text-[11px] font-semibold text-[var(--ink)] mb-1">
                Event Keamanan & Probing
              </label>
              <div className="flex items-center gap-1.5">
                <input
                  id="retention-security-days"
                  type="number"
                  min="1"
                  max="180"
                  value={securityDays}
                  onChange={(e) => setSecurityDays(Number(e.target.value))}
                  className="w-full px-2.5 py-1.5 rounded-lg border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                />
                <span className="text-[var(--ink-muted)] text-[11px]">Hari</span>
              </div>
              <p className="text-[10px] text-[var(--ink-muted)] mt-0.5">Bawaan: 7 hari</p>
            </div>

            <div>
              <label htmlFor="retention-rollup-1d" className="block text-[11px] font-semibold text-[var(--ink)] mb-1">
                Rollup 1 Hari (Opsional)
              </label>
              <div className="flex items-center gap-1.5">
                <input
                  id="retention-rollup-1d"
                  type="number"
                  min="1"
                  placeholder="Kosong = Selamanya"
                  value={rollup1dDays}
                  onChange={(e) => setRollup1dDays(e.target.value)}
                  className="w-full px-2.5 py-1.5 rounded-lg border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                />
                <span className="text-[var(--ink-muted)] text-[11px]">Hari</span>
              </div>
              <p className="text-[10px] text-[var(--ink-muted)] mt-0.5">Kosongkan bila disimpan permanen</p>
            </div>

            <div>
              <label htmlFor="retention-max-db" className="block text-[11px] font-semibold text-[var(--ink)] mb-1">
                Batas Ukuran DB (Guardrail)
              </label>
              <div className="flex items-center gap-1.5">
                <input
                  id="retention-max-db"
                  type="number"
                  min="50"
                  placeholder="Kosong = Tanpa batas"
                  value={maxDbMb}
                  onChange={(e) => setMaxDbMb(e.target.value)}
                  className="w-full px-2.5 py-1.5 rounded-lg border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] focus:outline-hidden focus:border-[var(--primary)]"
                />
                <span className="text-[var(--ink-muted)] text-[11px]">MB</span>
              </div>
              <p className="text-[10px] text-[var(--ink-muted)] mt-0.5">Pangkas agresif bila melebihi batas</p>
            </div>
          </div>

          <div className="pt-2 border-t border-[var(--line)] flex items-center justify-between">
            <label className="flex items-center gap-2 text-xs font-medium text-[var(--ink)] cursor-pointer">
              <input
                type="checkbox"
                checked={autoVacuum}
                onChange={(e) => setAutoVacuum(e.target.checked)}
                className="rounded border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
              />
              <span>Jalankan Auto-VACUUM saat fragmentasi &gt; 20%</span>
            </label>

            <button
              type="submit"
              disabled={savingConfig}
              className="px-4 py-1.5 rounded-lg bg-[var(--primary)] text-white text-xs font-semibold hover:opacity-90 transition-opacity disabled:opacity-50"
            >
              {savingConfig ? 'Menyimpan...' : 'Simpan Pengaturan'}
            </button>
          </div>
        </form>

        {/* Operational Maintenance & Integrity Actions */}
        <div className="p-4 bg-[var(--bg)] border border-[var(--line)] rounded-xl space-y-4 flex flex-col justify-between">
          <div>
            <div className="flex items-center gap-2 pb-2 border-b border-[var(--line)]">
              <Trash2 className="w-4 h-4 text-[var(--primary)]" />
              <h3 className="text-xs font-bold uppercase tracking-wider text-[var(--ink)]">
                Aksi Pemeliharaan & Integritas
              </h3>
            </div>
            <p className="text-xs text-[var(--ink-muted)] mt-2">
              Lakukan pembersihan data usang secara manual di luar jadwal worker background, checkpoint jurnal WAL, atau uji integritas struktur SQLite.
            </p>

            {/* Options */}
            <div className="mt-4 space-y-2 text-xs">
              <p className="text-[var(--ink-muted)]">Pembersihan mencakup checkpoint jurnal WAL ke berkas utama (TRUNCATE).</p>

              <label className="flex items-center gap-2 font-medium text-[var(--ink)] cursor-pointer">
                <input
                  type="checkbox"
                  checked={forceVacuum}
                  onChange={(e) => setForceVacuum(e.target.checked)}
                  className="rounded border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
                />
                <span>Paksa pemadatan basis data (VACUUM penuh)</span>
              </label>
            </div>
          </div>

          <div className="pt-4 border-t border-[var(--line)] flex flex-wrap items-center gap-3">
            <button
              type="button"
              onClick={handleTriggerMaintenance}
              disabled={runningMaintenance}
              className="px-4 py-2 rounded-lg bg-[var(--primary)] text-white text-xs font-semibold hover:opacity-90 transition-opacity disabled:opacity-50 flex items-center gap-1.5 shadow-xs"
            >
              <RefreshCw className={`w-3.5 h-3.5 ${runningMaintenance ? 'animate-spin' : ''}`} />
              {runningMaintenance ? 'Memelihara...' : 'Jalankan Pembersihan & Optimasi'}
            </button>

            <button
              type="button"
              onClick={handleCheckIntegrity}
              disabled={checkingIntegrity}
              className="px-3.5 py-2 rounded-lg border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] text-xs font-semibold hover:bg-[var(--surface-hover)] transition-colors disabled:opacity-50 flex items-center gap-1.5"
            >
              <ShieldAlert className={`w-3.5 h-3.5 ${checkingIntegrity ? 'animate-spin text-[var(--primary)]' : ''}`} />
              {checkingIntegrity ? 'Menguji...' : 'Uji Integritas Basis Data'}
            </button>
          </div>
        </div>
      </div>

      {/* Table Statistics Collapsible Section */}
      {data && data.storage.tables.length > 0 && (
        <div className="pt-2 border-t border-[var(--line)]">
          <button
            type="button"
            onClick={() => setTablesExpanded(!tablesExpanded)}
            className="w-full flex items-center justify-between p-3 rounded-xl bg-[var(--bg)] border border-[var(--line)] text-xs font-semibold text-[var(--ink)] hover:bg-[var(--surface-hover)] transition-colors"
          >
            <div className="flex items-center gap-2">
              <Database className="w-4 h-4 text-[var(--primary)]" />
              <span>Rincian Baris Data ({data.storage.tables.length} Tabel Basis Data)</span>
            </div>
            <div className="flex items-center gap-1.5 text-[var(--ink-muted)]">
              <span>{tablesExpanded ? 'Sembunyikan' : 'Tampilkan'}</span>
              {tablesExpanded ? <ChevronUp className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}
            </div>
          </button>

          {tablesExpanded && (
            <div className="mt-3 overflow-x-auto rounded-xl border border-[var(--line)] bg-[var(--bg)]">
              <table className="w-full text-xs text-left">
                <thead className="bg-[var(--surface)] text-[var(--ink-muted)] uppercase text-[10px] font-semibold border-b border-[var(--line)]">
                  <tr>
                    <th scope="col" className="px-4 py-2.5">Nama Tabel</th>
                    <th scope="col" className="px-4 py-2.5 text-right">Jumlah Baris</th>
                    <th scope="col" className="px-4 py-2.5 text-right">Kategori Resolusi</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--line)]">
                  {data.storage.tables.map((tbl) => {
                    let cat = 'Data Sistem / Konfigurasi';
                    if (tbl.table_name.includes('_raw')) cat = 'Resolusi Mentah (10s)';
                    else if (tbl.table_name.includes('_1m')) cat = 'Rollup 1 Menit';
                    else if (tbl.table_name.includes('_1h')) cat = 'Rollup 1 Jam';
                    else if (tbl.table_name.includes('_1d')) cat = 'Rollup 1 Hari';
                    else if (tbl.table_name.includes('security') || tbl.table_name.includes('banned')) cat = 'Keamanan & Pencegahan';
                    else if (tbl.table_name.includes('checkpoint')) cat = 'Log Tailer State';

                    return (
                      <tr key={tbl.table_name} className="hover:bg-[var(--surface)] transition-colors">
                        <td className="px-4 py-2 font-mono font-medium text-[var(--ink)]">
                          {tbl.table_name}
                        </td>
                        <td className="px-4 py-2 text-right font-mono text-[var(--ink)]">
                          {formatNumber(tbl.row_count)}
                        </td>
                        <td className="px-4 py-2 text-right text-[11px] text-[var(--ink-muted)]">
                          {cat}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
        </div>
      )}
    </div>
  );
};
