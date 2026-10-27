import React from 'react';
import { SiteAppMetricsResponse } from '../../types/api';

export interface AppMetricsCardProps {
  metrics: SiteAppMetricsResponse | null;
  className?: string;
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

export const AppMetricsCard: React.FC<AppMetricsCardProps> = ({
  metrics,
  className = '',
}) => {
  if (!metrics) {
    return (
      <div className={`bg-[var(--surface)] border border-[var(--line)] rounded-xl p-5 text-sm text-[var(--ink-muted)] ${className}`}>
        Memuat metrik runtime aplikasi...
      </div>
    );
  }

  const { runtime, php_fpm, app_process } = metrics;

  return (
    <div className={`bg-[var(--surface)] border border-[var(--line)] rounded-xl p-5 shadow-sm ${className}`}>
      <div className="flex items-center justify-between gap-3 mb-4">
        <div>
          <h3 className="text-base font-semibold text-[var(--ink)]">
            Metrik Runtime Aplikasi
          </h3>
          <p className="text-xs text-[var(--ink-muted)]">
            Kondisi pool proses pekerja dan alokasi sumber daya cgroup v2
          </p>
        </div>
        <span className="px-2.5 py-1 text-xs font-mono font-medium rounded-md bg-[var(--surface-hover)] border border-[var(--line)] text-[var(--primary)] uppercase">
          {runtime}
        </span>
      </div>

      {/* PHP-FPM Pool Stats */}
      {php_fpm && (
        <div className="space-y-4">
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
            <div className="p-3 bg-[var(--bg)] border border-[var(--line)] rounded-lg">
              <div className="text-[11px] text-[var(--ink-muted)] mb-1">Active Workers</div>
              <div className="text-lg font-mono font-semibold text-[var(--ink)]">
                {php_fpm.active_avg.toFixed(1)}{' '}
                <span className="text-xs font-normal text-[var(--ink-muted)]">
                  (max: {php_fpm.active_max})
                </span>
              </div>
            </div>

            <div className="p-3 bg-[var(--bg)] border border-[var(--line)] rounded-lg">
              <div className="text-[11px] text-[var(--ink-muted)] mb-1">Idle Workers</div>
              <div className="text-lg font-mono font-semibold text-[var(--ink)]">
                {php_fpm.idle_avg.toFixed(1)}{' '}
                <span className="text-xs font-normal text-[var(--ink-muted)]">
                  (total: {php_fpm.total_max})
                </span>
              </div>
            </div>

            <div className="p-3 bg-[var(--bg)] border border-[var(--line)] rounded-lg">
              <div className="text-[11px] text-[var(--ink-muted)] mb-1">Antrean (Listen Queue)</div>
              <div
                className={`text-lg font-mono font-semibold ${
                  php_fpm.listen_queue_max > 0 ? 'text-rose-500' : 'text-emerald-500'
                }`}
              >
                {php_fpm.listen_queue_max}
              </div>
            </div>

            <div className="p-3 bg-[var(--bg)] border border-[var(--line)] rounded-lg">
              <div className="text-[11px] text-[var(--ink-muted)] mb-1">Koneksi Diterima</div>
              <div className="text-lg font-mono font-semibold text-[var(--ink)]">
                {php_fpm.accepted_conn_total.toLocaleString()}
              </div>
            </div>
          </div>

          {/* Warnings if pool limits reached */}
          {(php_fpm.max_children_reached_max > 0 || php_fpm.slow_requests_total > 0) && (
            <div className="flex flex-wrap gap-2 text-xs">
              {php_fpm.max_children_reached_max > 0 && (
                <div className="px-3 py-1.5 rounded-lg bg-amber-500/10 border border-amber-500/20 text-amber-500 flex items-center gap-1.5">
                  <span>⚠️</span>
                  <span>
                    Batas <strong>pm.max_children</strong> tercapai ({php_fpm.max_children_reached_max}x) — pertimbangkan menaikkan pool limit.
                  </span>
                </div>
              )}
              {php_fpm.slow_requests_total > 0 && (
                <div className="px-3 py-1.5 rounded-lg bg-rose-500/10 border border-rose-500/20 text-rose-500 flex items-center gap-1.5">
                  <span>⏱️</span>
                  <span>
                    Terdeteksi <strong>{php_fpm.slow_requests_total} slow request</strong> pada pool {php_fpm.pool_name}.
                  </span>
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* Node.js / Python / Process Cgroup Stats */}
      {app_process && (
        <div className="mt-4 pt-4 border-t border-[var(--line)] space-y-3">
          <div className="text-xs font-semibold text-[var(--ink-muted)] uppercase tracking-wider">
            Isolasi Cgroup v2 ({app_process.app_type})
          </div>
          <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
            <div className="p-3 bg-[var(--bg)] border border-[var(--line)] rounded-lg">
              <div className="text-[11px] text-[var(--ink-muted)] mb-1">Penggunaan CPU</div>
              <div className="text-lg font-mono font-semibold text-[var(--ink)]">
                {app_process.cpu_avg.toFixed(1)}%{' '}
                <span className="text-xs font-normal text-[var(--ink-muted)]">
                  (max: {app_process.cpu_max.toFixed(1)}%)
                </span>
              </div>
            </div>

            <div className="p-3 bg-[var(--bg)] border border-[var(--line)] rounded-lg">
              <div className="text-[11px] text-[var(--ink-muted)] mb-1">Penggunaan Memori</div>
              <div className="text-lg font-mono font-semibold text-[var(--ink)]">
                {formatBytes(app_process.memory_current_avg_bytes)}{' '}
                <span className="text-xs font-normal text-[var(--ink-muted)]">
                  (peak: {formatBytes(app_process.memory_peak_max_bytes || app_process.memory_current_max_bytes)})
                </span>
              </div>
            </div>

            <div className="p-3 bg-[var(--bg)] border border-[var(--line)] rounded-lg">
              <div className="text-[11px] text-[var(--ink-muted)] mb-1">Jumlah Task / PID</div>
              <div className="text-lg font-mono font-semibold text-[var(--ink)]">
                {app_process.pids_max}
              </div>
            </div>

            <div className="p-3 bg-[var(--bg)] border border-[var(--line)] rounded-lg">
              <div className="text-[11px] text-[var(--ink-muted)] mb-1">Disk I/O</div>
              <div className="text-xs font-mono font-semibold text-[var(--ink)] space-y-0.5">
                <div>R: {formatBytes(app_process.io_read_avg_bytes_sec)}/s</div>
                <div>W: {formatBytes(app_process.io_write_avg_bytes_sec)}/s</div>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
