import React, { useState, useEffect } from 'react';
import {
  Activity,
  RefreshCw,
  AlertTriangle,
  CheckCircle2,
  XCircle,
} from 'lucide-react';
import { fetchSitePhpStatus } from '../../api/client';
import { SitePhpStatusResponse } from '../../types/api';

export interface PhpStatusCardProps {
  siteId: number;
}

export const PhpStatusCard: React.FC<PhpStatusCardProps> = ({ siteId }) => {
  const [statusData, setStatusData] = useState<SitePhpStatusResponse | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);

  const loadStatus = async () => {
    setIsLoading(true);
    setError(null);
    try {
      const data = await fetchSitePhpStatus(siteId);
      setStatusData(data);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memuat status PHP-FPM';
      setError(msg);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    loadStatus();
    const interval = setInterval(loadStatus, 15000);
    return () => clearInterval(interval);
  }, [siteId]);

  if (isLoading && !statusData) {
    return (
      <div className="p-6 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-md)] text-center text-xs text-[var(--ink-muted)]">
        Memuat status runtime PHP-FPM...
      </div>
    );
  }

  if (error && !statusData) {
    return (
      <div className="p-4 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-md)] flex items-center justify-between">
        <div className="flex items-center gap-2 text-xs text-red-500">
          <AlertTriangle className="w-4 h-4 shrink-0" />
          <span>{error}</span>
        </div>
        <button
          onClick={loadStatus}
          className="text-xs px-2.5 py-1 rounded bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface-input)] transition-colors flex items-center gap-1.5"
        >
          <RefreshCw className="w-3.5 h-3.5" />
          Coba Lagi
        </button>
      </div>
    );
  }

  if (!statusData) return null;

  const metrics = statusData.metrics;

  return (
    <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-md)] p-5 space-y-4">
      <div className="flex items-center justify-between border-b border-[var(--line)] pb-3">
        <div className="flex items-center gap-2">
          <Activity className="w-4 h-4 text-[var(--primary)]" />
          <h4 className="text-sm font-semibold text-[var(--ink)]">
            Status Layanan PHP-FPM ({statusData.php_version})
          </h4>
        </div>
        <div className="flex items-center gap-2">
          <span
            className={`inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full font-medium ${
              statusData.is_active
                ? 'bg-emerald-100 text-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-300'
                : 'bg-red-100 text-red-800 dark:bg-red-950/40 dark:text-red-300'
            }`}
          >
            {statusData.is_active ? (
              <>
                <CheckCircle2 className="w-3 h-3 text-emerald-600 dark:text-emerald-400" />
                Aktif
              </>
            ) : (
              <>
                <XCircle className="w-3 h-3 text-red-600 dark:text-red-400" />
                Tidak Aktif
              </>
            )}
          </span>
          <button
            onClick={loadStatus}
            disabled={isLoading}
            title="Segarkan Metrik FPM"
            className="p-1 rounded text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] transition-colors"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? 'animate-spin' : ''}`} />
          </button>
        </div>
      </div>

      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
          <div className="text-[10px] text-[var(--ink-muted)] font-medium uppercase tracking-wider">
            Worker Aktif
          </div>
          <div className="text-base font-semibold text-[var(--ink)] mt-0.5">
            {metrics ? metrics.active_processes : '-'}
            <span className="text-xs font-normal text-[var(--ink-muted)] ml-1">
              / {metrics ? metrics.total_processes : '-'}
            </span>
          </div>
        </div>

        <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
          <div className="text-[10px] text-[var(--ink-muted)] font-medium uppercase tracking-wider">
            Worker Idle
          </div>
          <div className="text-base font-semibold text-[var(--ink)] mt-0.5">
            {metrics ? metrics.idle_processes : '-'}
          </div>
        </div>

        <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
          <div className="text-[10px] text-[var(--ink-muted)] font-medium uppercase tracking-wider">
            Listen Queue
          </div>
          <div className="text-base font-semibold text-[var(--ink)] mt-0.5">
            {metrics ? metrics.listen_queue : '-'}
            <span className="text-xs font-normal text-[var(--ink-muted)] ml-1">
              (Maks {metrics ? metrics.max_listen_queue : '-'})
            </span>
          </div>
        </div>

        <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
          <div className="text-[10px] text-[var(--ink-muted)] font-medium uppercase tracking-wider">
            Slow Requests
          </div>
          <div className={`text-base font-semibold mt-0.5 ${metrics && metrics.slow_requests > 0 ? 'text-amber-500' : 'text-[var(--ink)]'}`}>
            {metrics ? metrics.slow_requests : '-'}
          </div>
        </div>
      </div>

      <div className="flex flex-wrap items-center justify-between text-[11px] text-[var(--ink-muted)] pt-1">
        <span>Unit: <code>{statusData.service_name}</code></span>
        <span>
          Socket:{' '}
          <span className={statusData.socket_exists ? 'text-emerald-600 font-medium' : 'text-red-500 font-medium'}>
            {statusData.socket_exists ? 'Tersedia' : 'Tidak Ditemukan'}
          </span>
        </span>
      </div>
    </div>
  );
};
export default PhpStatusCard;
