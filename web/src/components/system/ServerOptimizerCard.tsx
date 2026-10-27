import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useCallback } from 'react';
import {
  Cpu,
  Server,
  Zap,
  RotateCcw,
  CheckCircle2,
  AlertTriangle,
  RefreshCw,
  Sliders,
  ShieldCheck,
  Activity,
  Layers,
  Clock,
  Sparkles,
} from 'lucide-react';
import {
  fetchOptimizerStatus,
  applyOptimizerProfile,
  rollbackOptimizerProfile,
} from '../../api/client';
import {
  OptimizerStatusResponseDto,
  OptimizerProfileDto,
  OptimizerActionResponseDto,
} from '../../types/api';

export interface ServerOptimizerCardProps {
  className?: string;
  initialData?: OptimizerStatusResponseDto;
  onRefreshParent?: () => void;
}

function formatBytesMb(mb: number): string {
  if (mb >= 1024) {
    return `${(mb / 1024).toFixed(1)} GB`;
  }
  return `${mb} MB`;
}

function formatTimestamp(ts: number): string {
  if (!ts) return '-';
  const d = new Date(ts * 1000);
  return d.toLocaleString('id-ID', {
    dateStyle: 'medium',
    timeStyle: 'medium',
  });
}

const AVAILABLE_PROFILES: { id: string; name: string; desc: string }[] = [
  {
    id: 'micro',
    name: 'Micro VPS (< 2 GB RAM)',
    desc: 'Tuning ultra-konservatif untuk meminimalkan alokasi memori.',
  },
  {
    id: 'standard',
    name: 'Standard (2 - 4 GB RAM)',
    desc: 'Tuning seimbang untuk performa stabil pada VPS menengah.',
  },
  {
    id: 'high_performance',
    name: 'High Performance (4 - 16 GB RAM)',
    desc: 'Tuning agresif untuk lalu lintas tinggi dan throughput maksimal.',
  },
  {
    id: 'dedicated_large',
    name: 'Dedicated Large (> 16 GB RAM)',
    desc: 'Tuning enterprise throughput ultra-tinggi untuk bare-metal / server besar.',
  },
];

export const ServerOptimizerCard: React.FC<ServerOptimizerCardProps> = ({
  className = '',
  initialData,
  onRefreshParent,
}) => {
  const [data, setData] = useState<OptimizerStatusResponseDto | null>(initialData ?? null);
  const [loading, setLoading] = useState<boolean>(initialData === undefined);
  const [error, setError] = useState<string | null>(null);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);

  // Selected profile to inspect/apply
  const [selectedProfileId, setSelectedProfileId] = useState<string>(
    initialData ? initialData.active_profile || initialData.recommended_profile : 'standard'
  );

  // Active configuration tab
  const [activeTab, setActiveTab] = useState<'nginx' | 'sysctl' | 'php' | 'runtimes'>('nginx');

  // Confirmation modals
  const [isApplyModalOpen, setIsApplyModalOpen] = useState(false);
  const [isRollbackModalOpen, setIsRollbackModalOpen] = useState(false);
  const [actionInProgress, setActionInProgress] = useState(false);

  const loadStatus = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetchOptimizerStatus();
      setData(res);
      if (!selectedProfileId) {
        setSelectedProfileId(res.active_profile || res.recommended_profile);
      }
    } catch (err: any) {
      setError(err?.message || 'Gagal memuat status pengoptimal server');
    } finally {
      setLoading(false);
    }
  }, [selectedProfileId]);

  useEffect(() => {
    if (!initialData) {
      loadStatus();
    } else {
      setData(initialData);
      setSelectedProfileId(initialData.active_profile || initialData.recommended_profile);
    }
  }, [initialData, loadStatus]);

  // Profile currently displayed in detail view
  const profileDetails: OptimizerProfileDto | null =
    data?.current_settings.profile_type.toLowerCase() === selectedProfileId.toLowerCase()
      ? data.current_settings
      : data?.recommended_settings.profile_type.toLowerCase() === selectedProfileId.toLowerCase()
      ? data.recommended_settings
      : data?.current_settings || null;

  const handleApply = async () => {
    setActionInProgress(true);
    setError(null);
    setSuccessMsg(null);
    try {
      const res: OptimizerActionResponseDto = await applyOptimizerProfile({
        profile: selectedProfileId,
      });

      setSuccessMsg(
        `Profil tuning ${res.profile || selectedProfileId} berhasil diterapkan! ${res.message}`
      );
      setIsApplyModalOpen(false);
      await loadStatus();
      if (onRefreshParent) {
        onRefreshParent();
      }
    } catch (err: any) {
      setError(err?.message || 'Gagal menerapkan profil tuning');
    } finally {
      setActionInProgress(false);
    }
  };

  const handleRollback = async () => {
    setActionInProgress(true);
    setError(null);
    setSuccessMsg(null);
    try {
      const res: OptimizerActionResponseDto = await rollbackOptimizerProfile();
      setSuccessMsg(
        `Rollback berhasil dilakukan. Konfigurasi dikembalikan ke snapshot sebelumnya: ${res.message}`
      );
      setIsRollbackModalOpen(false);
      await loadStatus();
      if (onRefreshParent) {
        onRefreshParent();
      }
    } catch (err: any) {
      setError(err?.message || 'Gagal melakukan pemulihan konfigurasi cadangan');
    } finally {
      setActionInProgress(false);
    }
  };

  return (
    <div
      className={`border border-[var(--line)] bg-[var(--surface)] rounded-[var(--radius-md)] p-6 space-y-6 ${className}`}
      data-testid="server-optimizer-card"
    >
      {/* Top Header & Quick Actions */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 pb-4 border-b border-[var(--line)]">
        <div>
          <div className="flex items-center gap-2">
            <Sliders className="w-5 h-5 text-[var(--primary)]" />
            <h2 className="text-base font-semibold text-[var(--ink)]">
              Pengoptimal Server & Tuning Performa
            </h2>
          </div>
          <p className="text-xs text-[var(--ink-muted)] mt-1">
            Penyesuaian profil otomatis untuk Nginx, Kernel sysctl, PHP-FPM, dan runtime berdasarkan spesifikasi hardware server.
          </p>
        </div>

        <div className="flex items-center gap-2 flex-wrap">
          <button
            type="button"
            onClick={loadStatus}
            disabled={loading}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--surface-sunken)] disabled:opacity-50 transition-colors"
            title="Muat ulang status tuning"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? 'animate-spin' : ''}`} />
            Segarkan
          </button>

          {data?.has_backup && (
            <button
              type="button"
              onClick={() => setIsRollbackModalOpen(true)}
              disabled={actionInProgress}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-400 hover:bg-amber-500/20 disabled:opacity-50 transition-colors"
              title="Pulihkan konfigurasi sebelum apply terakhir"
            >
              <RotateCcw className="w-3.5 h-3.5" />
              Pulihkan Cadangan
            </button>
          )}
        </div>
      </div>

      {/* Alert Messages */}
      {error && (
        <div
          role="alert"
          className="flex items-start gap-3 p-3 text-xs rounded-[var(--radius-md)] bg-rose-500/10 border border-rose-500/20 text-rose-600 dark:text-rose-400"
        >
          <AlertTriangle className="w-4 h-4 shrink-0 mt-0.5" />
          <div className="flex-1">
            <span className="font-semibold">Kesalahan:</span> {error}
          </div>
        </div>
      )}

      {successMsg && (
        <div
          role="status"
          className="flex items-start gap-3 p-3 text-xs rounded-[var(--radius-md)] bg-emerald-500/10 border border-emerald-500/20 text-emerald-600 dark:text-emerald-400"
        >
          <CheckCircle2 className="w-4 h-4 shrink-0 mt-0.5" />
          <div className="flex-1">
            <span className="font-semibold">Berhasil:</span> {successMsg}
          </div>
        </div>
      )}

      {/* Hardware Specs Overview */}
      {data && (
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
          <div className="p-3 bg-[var(--surface-sunken)] border border-[var(--line)] rounded-[var(--radius-md)] flex flex-col justify-between">
            <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
              <span>CPU Core</span>
              <Cpu className="w-3.5 h-3.5" />
            </div>
            <div className="text-base font-semibold text-[var(--ink)]">
              {`${data.hardware.vcpu_count} vCPU`}
            </div>
            <div className="text-[10px] text-[var(--ink-muted)] truncate">
              Core Processor Host
            </div>
          </div>

          <div className="p-3 bg-[var(--surface-sunken)] border border-[var(--line)] rounded-[var(--radius-md)] flex flex-col justify-between">
            <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
              <span>Total Memori</span>
              <Activity className="w-3.5 h-3.5" />
            </div>
            <div className="text-base font-semibold text-[var(--ink)]">
              {formatBytesMb(data.hardware.ram_total_mb)}
            </div>
            <div className="text-[10px] text-[var(--ink-muted)]">
              {`Swap: ${formatBytesMb(data.hardware.swap_total_mb)}`}
            </div>
          </div>

          <div className="p-3 bg-[var(--surface-sunken)] border border-[var(--line)] rounded-[var(--radius-md)] flex flex-col justify-between">
            <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
              <span>Sistem Operasi</span>
              <Server className="w-3.5 h-3.5" />
            </div>
            <div className="text-sm font-semibold text-[var(--ink)] truncate" title={data.hardware.os_distro}>
              {data.hardware.os_distro}
            </div>
            <div className="text-[10px] text-[var(--ink-muted)] truncate" title={data.hardware.kernel_version}>
              {`Kernel ${data.hardware.kernel_version}`}
            </div>
          </div>

          <div className="p-3 bg-[var(--surface-sunken)] border border-[var(--line)] rounded-[var(--radius-md)] flex flex-col justify-between">
            <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
              <span>Status Tuning</span>
              <ShieldCheck className="w-3.5 h-3.5" />
            </div>
            <div className="text-sm font-semibold text-[var(--ink)] flex items-center gap-1.5">
              {data.active_profile ? (
                <>
                  <span className="w-2 h-2 rounded-full bg-emerald-500 animate-pulse" />
                  <span>{data.active_profile}</span>
                </>
              ) : (
                <>
                  <span className="w-2 h-2 rounded-full bg-slate-400" />
                  <span className="text-[var(--ink-muted)]">Belum Aktif</span>
                </>
              )}
            </div>
            <div className="text-[10px] text-[var(--ink-muted)]">
              {data.applied_at ? `Diperbarui: ${formatTimestamp(data.applied_at)}` : 'Konfigurasi bawaan'}
            </div>
          </div>
        </div>
      )}

      {/* Recommendation Banner */}
      {data && (
        <div className="p-3.5 rounded-[var(--radius-md)] border border-sky-500/20 bg-sky-500/5 flex items-start gap-3">
          <Sparkles className="w-4 h-4 text-sky-500 shrink-0 mt-0.5" />
          <div className="text-xs text-[var(--ink)] leading-relaxed">
            <span className="font-semibold text-sky-600 dark:text-sky-400">
              {`Rekomendasi Cerdas: ${data.recommended_profile}`}
            </span>
            <p className="text-[var(--ink-muted)] mt-0.5">
              Profil {data.recommended_settings.display_name} disarankan berdasarkan kapasitas RAM{' '}
              {formatBytesMb(data.hardware.ram_total_mb)} dan {data.hardware.vcpu_count} vCPU host.
            </p>
          </div>
        </div>
      )}

      {/* Profile Selector Cards */}
      {data && (
        <div>
          <div className="text-xs font-semibold text-[var(--ink)] mb-2 flex items-center justify-between">
            <span>Pilih Profil Konfigurasi:</span>
            <span className="text-[11px] font-normal text-[var(--ink-muted)]">
              Klik profil untuk melihat parameter teknis & menerapkan
            </span>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
            {AVAILABLE_PROFILES.map((profile) => {
              const isSelected = selectedProfileId.toLowerCase() === profile.id.toLowerCase();
              const isActive =
                data.active_profile &&
                data.active_profile.toLowerCase() === profile.id.toLowerCase();
              const isRecommended =
                data.recommended_profile.toLowerCase() === profile.id.toLowerCase();

              return (
                <button
                  key={profile.id}
                  type="button"
                  onClick={() => setSelectedProfileId(profile.id)}
                  className={`text-left p-3.5 rounded-[var(--radius-md)] border transition-all flex flex-col justify-between ${
                    isSelected
                      ? 'border-[var(--primary)] bg-[var(--primary)]/5 ring-1 ring-[var(--primary)]'
                      : 'border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--surface-sunken)]'
                  }`}
                >
                  <div>
                    <div className="flex items-center justify-between gap-1 mb-1">
                      <span className="text-xs font-semibold text-[var(--ink)]">
                        {profile.name}
                      </span>
                      {isActive && (
                        <span className="px-1.5 py-0.5 text-[9px] font-medium rounded bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20">
                          Aktif
                        </span>
                      )}
                    </div>
                    <div className="text-[11px] text-[var(--ink-muted)] mb-2">
                      {profile.desc}
                    </div>
                  </div>

                  <div className="flex items-center justify-between pt-2 border-t border-[var(--line)]/50 text-[10px]">
                    <span className="text-[var(--ink-muted)]">
                      Tingkat: {profile.id}
                    </span>
                    {isRecommended && (
                      <span className="text-sky-600 dark:text-sky-400 font-medium">
                        ★ Disarankan
                      </span>
                    )}
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      )}

      {/* Selected Profile Parameter Tabs */}
      {profileDetails && (
        <div className="border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--surface-sunken)] p-4 space-y-4">
          <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2 border-b border-[var(--line)] pb-3">
            <div className="flex items-center gap-2">
              <Layers className="w-4 h-4 text-[var(--primary)]" />
              <h3 className="text-xs font-semibold text-[var(--ink)]">
                Parameter Profil: {profileDetails.display_name}
              </h3>
            </div>

            {/* Sub-tabs */}
            <div className="settings-section-tabs settings-optimizer-tabs flex items-center gap-1 overflow-x-auto text-xs">
              <button
                type="button"
                onClick={() => setActiveTab('nginx')}
                className={`px-2.5 py-1 rounded-[var(--radius-sm)] transition-colors ${
                  activeTab === 'nginx'
                    ? 'bg-[var(--primary)] text-white font-medium'
                    : 'text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface)]'
                }`}
              >
                Nginx
              </button>
              <button
                type="button"
                onClick={() => setActiveTab('sysctl')}
                className={`px-2.5 py-1 rounded-[var(--radius-sm)] transition-colors ${
                  activeTab === 'sysctl'
                    ? 'bg-[var(--primary)] text-white font-medium'
                    : 'text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface)]'
                }`}
              >
                Kernel Sysctl
              </button>
              <button
                type="button"
                onClick={() => setActiveTab('php')}
                className={`px-2.5 py-1 rounded-[var(--radius-sm)] transition-colors ${
                  activeTab === 'php'
                    ? 'bg-[var(--primary)] text-white font-medium'
                    : 'text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface)]'
                }`}
              >
                PHP-FPM
              </button>
              <button
                type="button"
                onClick={() => setActiveTab('runtimes')}
                className={`px-2.5 py-1 rounded-[var(--radius-sm)] transition-colors ${
                  activeTab === 'runtimes'
                    ? 'bg-[var(--primary)] text-white font-medium'
                    : 'text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface)]'
                }`}
              >
                Runtime Hints
              </button>
            </div>
          </div>

          {/* Tab 1: Nginx Configuration */}
          {activeTab === 'nginx' && (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-3 text-xs">
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">worker_processes</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.nginx.worker_processes}
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">worker_connections</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.nginx.worker_connections}
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">worker_rlimit_nofile</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.nginx.worker_rlimit_nofile}
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">client_body_buffer_size</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.nginx.client_body_buffer_size_kb}k
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">fastcgi_buffers</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.nginx.fastcgi_buffers}
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">open_file_cache</span>
                <span className="font-semibold text-[var(--ink)] font-mono truncate max-w-[200px]">
                  max={profileDetails.nginx.open_file_cache_max} inactive={profileDetails.nginx.open_file_cache_inactive_secs}s
                </span>
              </div>
            </div>
          )}

          {/* Tab 2: Kernel Sysctl */}
          {activeTab === 'sysctl' && (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-3 text-xs">
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">net.core.somaxconn</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.sysctl.somaxconn}
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">net.ipv4.tcp_max_syn_backlog</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.sysctl.tcp_max_syn_backlog}
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">net.ipv4.tcp_tw_reuse</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.sysctl.tcp_tw_reuse}
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">net.ipv4.ip_local_port_range</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.sysctl.ip_local_port_range}
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">vm.swappiness</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.sysctl.swappiness}
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">fs.file-max</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.sysctl.file_max}
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center md:col-span-2">
                <span className="text-[var(--ink-muted)] font-mono">net.core.netdev_max_backlog</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.sysctl.netdev_max_backlog}
                </span>
              </div>
            </div>
          )}

          {/* Tab 3: PHP-FPM */}
          {activeTab === 'php' && (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-3 text-xs">
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">pm</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.php_fpm.pm_type}
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">pm.max_children</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.php_fpm.max_children}
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">pm.process_idle_timeout</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.php_fpm.process_idle_timeout_secs}s
                </span>
              </div>
              <div className="p-2.5 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)] flex justify-between items-center">
                <span className="text-[var(--ink-muted)] font-mono">pm.max_requests</span>
                <span className="font-semibold text-[var(--ink)] font-mono">
                  {profileDetails.php_fpm.max_requests}
                </span>
              </div>
            </div>
          )}

          {/* Tab 4: Runtime Hints */}
          {activeTab === 'runtimes' && (
            <div className="grid grid-cols-1 md:grid-cols-3 gap-3 text-xs">
              <div className="p-3 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)]">
                <div className="text-[var(--ink-muted)] text-[11px] mb-1">Node.js Memory Limit</div>
                <div className="font-semibold text-[var(--ink)] font-mono text-sm">
                  {profileDetails.hints.recommended_node_max_old_space_mb} MB
                </div>
                <div className="text-[10px] text-[var(--ink-muted)] mt-1">
                  --max-old-space-size
                </div>
              </div>
              <div className="p-3 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)]">
                <div className="text-[var(--ink-muted)] text-[11px] mb-1">Node.js Worker Cluster</div>
                <div className="font-semibold text-[var(--ink)] font-mono text-sm">
                  {profileDetails.hints.recommended_node_workers} Worker
                </div>
                <div className="text-[10px] text-[var(--ink-muted)] mt-1">
                  Direkomendasikan per instance
                </div>
              </div>
              <div className="p-3 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-sm)]">
                <div className="text-[var(--ink-muted)] text-[11px] mb-1">Python Gunicorn / Uvicorn</div>
                <div className="font-semibold text-[var(--ink)] font-mono text-sm">
                  {profileDetails.hints.recommended_python_workers} Worker
                </div>
                <div className="text-[10px] text-[var(--ink-muted)] mt-1">
                  Optimal per core
                </div>
              </div>
            </div>
          )}

          {/* Action Footer */}
          <div className="pt-3 border-t border-[var(--line)] flex flex-col sm:flex-row sm:items-center sm:justify-end gap-3">
            <button
              type="button"
              onClick={() => setIsApplyModalOpen(true)}
              disabled={actionInProgress}
              className="inline-flex items-center justify-center gap-2 px-4 py-2 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 disabled:opacity-50 transition-opacity shadow-sm"
            >
              <Zap className="w-3.5 h-3.5" />
              Terapkan Profil {selectedProfileId}
            </button>
          </div>
        </div>
      )}

      {/* Backup & Rollback Information Footer */}
      {data?.has_backup && (
        <div className="text-xs text-[var(--ink-muted)] flex items-center justify-between pt-2 border-t border-[var(--line)]/50">
          <div className="flex items-center gap-1.5">
            <Clock className="w-3.5 h-3.5" />
            <span>
              Cadangan konfigurasi tersedia dibuat pada {formatTimestamp(data.last_backup_at || 0)}.
            </span>
          </div>
          <span className="text-[11px] text-emerald-600 dark:text-emerald-400">
            Rollback atomik siap
          </span>
        </div>
      )}

      {/* Confirmation Modal: Apply Profile */}
      {isApplyModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm"
          onDismiss={() => setIsApplyModalOpen(false)}
          dismissDisabled={actionInProgress}
        >
          <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] p-6 max-w-md w-full space-y-4 shadow-xl dialog-panel">
            <div className="flex items-center gap-3 text-amber-500 dialog-header">
              <AlertTriangle className="w-6 h-6" />
              <h3 className="text-sm font-semibold text-[var(--ink)]">Konfirmasi Terapkan Profil Tuning</h3>
            </div>
            <div className="dialog-body space-y-4">
              <p className="text-xs text-[var(--ink-muted)] leading-relaxed">
                Anda akan menerapkan profil <strong className="text-[var(--ink)]">{selectedProfileId}</strong>. Sistem
                akan membuat cadangan konfigurasi saat ini, menulis file konfigurasi Nginx dan Kernel sysctl, serta
                memvalidasi sintaks via{' '}
                <code className="bg-[var(--surface-sunken)] px-1 py-0.5 rounded font-mono">nginx -t</code>. Jika
                validasi gagal, konfigurasi akan dibatalkan secara otomatis seketika.
              </p>
            </div>
            <div className="flex items-center justify-end gap-2 pt-2 border-t border-[var(--line)] dialog-footer">
              <button
                type="button"
                onClick={() => setIsApplyModalOpen(false)}
                disabled={actionInProgress}
                className="px-3.5 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-sunken)] transition-colors"
              >
                Batal
              </button>
              <button
                type="button"
                onClick={handleApply}
                disabled={actionInProgress}
                className="inline-flex items-center gap-1.5 px-3.5 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 disabled:opacity-50 transition-opacity"
              >
                {actionInProgress && <RefreshCw className="w-3.5 h-3.5 animate-spin" />}
                Ya, Terapkan Sekarang
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}

      {/* Confirmation Modal: Rollback */}
      {isRollbackModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm"
          onDismiss={() => setIsRollbackModalOpen(false)}
          dismissDisabled={actionInProgress}
        >
          <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] p-6 max-w-md w-full space-y-4 shadow-xl dialog-panel">
            <div className="flex items-center gap-3 text-rose-500 dialog-header">
              <RotateCcw className="w-6 h-6" />
              <h3 className="text-sm font-semibold text-[var(--ink)]">Konfirmasi Pulihkan Cadangan (Rollback)</h3>
            </div>
            <div className="dialog-body space-y-4">
              <p className="text-xs text-[var(--ink-muted)] leading-relaxed">
                Konfigurasi Nginx dan Kernel sysctl akan dikembalikan ke snapshot cadangan sebelum tuning terakhir.
                Layanan Nginx akan direload untuk mengaktifkan kembali konfigurasi sebelumnya.
              </p>
            </div>
            <div className="flex items-center justify-end gap-2 pt-2 border-t border-[var(--line)] dialog-footer">
              <button
                type="button"
                onClick={() => setIsRollbackModalOpen(false)}
                disabled={actionInProgress}
                className="px-3.5 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] hover:bg-[var(--surface-sunken)] transition-colors"
              >
                Batal
              </button>
              <button
                type="button"
                onClick={handleRollback}
                disabled={actionInProgress}
                className="inline-flex items-center gap-1.5 px-3.5 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-rose-600 text-white hover:bg-rose-700 disabled:opacity-50 transition-colors"
              >
                {actionInProgress && <RefreshCw className="w-3.5 h-3.5 animate-spin" />}
                Pulihkan Cadangan
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}
    </div>
  );
};
