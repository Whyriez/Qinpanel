import { navigate, RouteLink, useBrowserRoute } from '../lib/router';
import React, { useState, useEffect, useCallback } from 'react';
import {
  ArrowLeft,
  User,
  Activity,
  Cpu,
  HardDrive,
  LogIn,
  PauseCircle,
  PlayCircle,
  RefreshCw,
  AlertTriangle,
  Layers,
  Terminal,
  ShieldCheck,
} from 'lucide-react';
import {
  TenantDetailResponse,
  TenantResourceUsage,
  TenantShellPolicyResponse,
} from '../types/api';
import {
  fetchTenantDetail,
  fetchTenantResources,
  suspendTenant,
  unsuspendTenant,
  deleteTenant,
  impersonateTenant,
  fetchTenantShellPolicy,
  updateTenantShellPolicy,
} from '../api/client';
import { StatusPill } from '../components/ui/StatusPill';
import { ConfirmDangerDialog } from '../components/ui/ConfirmDangerDialog';
import { useAuth } from '../context/AuthContext';
import '../styles/tenants.css';

export interface TenantDetailPageProps {
  tenantId: number;
  onBack: () => void;
}

export const TenantDetailPage: React.FC<TenantDetailPageProps> = ({
  tenantId,
  onBack,
}) => {
  const { user } = useAuth();
  const [tenantData, setTenantData] = useState<TenantDetailResponse | null>(null);
  const [resources, setResources] = useState<TenantResourceUsage | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isLoadingResources, setIsLoadingResources] = useState(false);
  const route = useBrowserRoute();
  const shellPolicyCard = React.useRef<HTMLDivElement>(null);
  const activeTab = route.pathname.split('/')[3] || 'overview';
  const setActiveTab = (tab: string) => navigate(`/tenants/${tenantId}/${tab}`);


  const [shellPolicyData, setShellPolicyData] = useState<TenantShellPolicyResponse | null>(null);
  const [selectedShellPolicy, setSelectedShellPolicy] = useState<string>('inherit');
  const [isSavingPolicy, setIsSavingPolicy] = useState(false);

  const [isDeleting, setIsDeleting] = useState(false);
  const [isDeleteDialogOpen, setIsDeleteDialogOpen] = useState(false);
  const [toastMessage, setToastMessage] = useState<{ text: string; type: 'success' | 'error' } | null>(null);

  const loadTenant = useCallback(async () => {
    setIsLoading(true);
    try {
      const detail = await fetchTenantDetail(tenantId);
      setTenantData(detail);
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({ text: e.message || 'Gagal memuat detail tenant', type: 'error' });
    } finally {
      setIsLoading(false);
    }
  }, [tenantId]);

  const loadResources = useCallback(async () => {
    setIsLoadingResources(true);
    try {
      const res = await fetchTenantResources(tenantId);
      setResources(res);
    } catch (err: unknown) {
      const e = err as Error;
      console.warn('Could not read tenant live resources:', e.message);
    } finally {
      setIsLoadingResources(false);
    }
  }, [tenantId]);

  const loadShellPolicy = useCallback(async () => {
    try {
      const pol = await fetchTenantShellPolicy(tenantId);
      setShellPolicyData(pol);
      setSelectedShellPolicy(pol.shell_policy ?? 'inherit');
    } catch (err: unknown) {
      console.warn('Could not read tenant shell policy:', (err as Error).message);
    }
  }, [tenantId]);

  useEffect(() => {
    loadTenant();
    loadResources();
    loadShellPolicy();
  }, [loadTenant, loadResources, loadShellPolicy]);

  useEffect(() => {
    if (!isLoading && route.search.get('section') === 'shell') {
      shellPolicyCard.current?.scrollIntoView({ block: 'start' });
      shellPolicyCard.current?.focus({ preventScroll: true });
    }
  }, [isLoading, route.url, route.search]);

  const handleSaveShellPolicy = async () => {
    setIsSavingPolicy(true);
    try {
      const policyToSet = selectedShellPolicy === 'inherit' ? null : selectedShellPolicy;
      const res = await updateTenantShellPolicy(tenantId, policyToSet);
      setShellPolicyData(res);
      setSelectedShellPolicy(res.shell_policy ?? 'inherit');
      setToastMessage({ text: 'Kebijakan shell tenant berhasil disimpan', type: 'success' });
      loadTenant();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({ text: e.message || 'Gagal memperbarui kebijakan shell', type: 'error' });
    } finally {
      setIsSavingPolicy(false);
    }
  };

  const handleToggleSuspend = async () => {
    if (!tenantData) return;
    try {
      if (tenantData.tenant.status === 'active') {
        await suspendTenant(tenantId);
        setToastMessage({ text: 'Tenant berhasil ditangguhkan', type: 'success' });
      } else {
        await unsuspendTenant(tenantId);
        setToastMessage({ text: 'Tenant berhasil diaktifkan kembali', type: 'success' });
      }
      loadTenant();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({ text: e.message || 'Gagal mengubah status tenant', type: 'error' });
    }
  };

  const handleImpersonate = async () => {
    try {
      await impersonateTenant(tenantId);
      window.location.reload();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({ text: e.message || 'Gagal impersonasi tenant', type: 'error' });
    }
  };

  const handleConfirmDelete = async (purgeFiles?: boolean) => {
    setIsDeleting(true);
    try {
      await deleteTenant(tenantId, purgeFiles);
      setToastMessage({ text: 'Tenant berhasil dihapus permanen', type: 'success' });
      setIsDeleteDialogOpen(false);
      onBack();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({ text: e.message || 'Gagal menghapus tenant', type: 'error' });
    } finally {
      setIsDeleting(false);
    }
  };

  const formatBytes = (bytes: number): string => {
    if (!bytes || bytes === 0) return '0 B';
    if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
    if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${bytes} B`;
  };

  if (isLoading || !tenantData) {
    return (
      <div className="p-8 text-center text-xs text-[var(--ink-muted)]">
        Memuat data tenant...
      </div>
    );
  }

  const { tenant, plan } = tenantData;

  // Calculate percentages
  const memLimit = plan?.memory_limit_bytes || 0;
  const memCurrent = resources?.metrics.memory_current_bytes || 0;
  const memPercent = memLimit > 0 ? Math.min(100, Math.round((memCurrent / memLimit) * 100)) : 0;

  const diskLimit = resources?.quota?.bytes_limit || plan?.disk_limit_bytes || 0;
  const diskUsed = resources?.quota?.bytes_used || 0;
  const diskPercent = diskLimit > 0 ? Math.min(100, Math.round((diskUsed / diskLimit) * 100)) : 0;

  const pidsLimit = plan?.tasks_max || 0;
  const pidsCurrent = resources?.metrics.pids_current || 0;
  const pidsPercent = pidsLimit > 0 ? Math.min(100, Math.round((pidsCurrent / pidsLimit) * 100)) : 0;

  return (
    <div className="tenant-page space-y-6" data-testid="tenant-detail-page">
      {/* Toast */}
      {toastMessage && (
        <div
          className={`p-3.5 rounded-[var(--radius-sm)] text-xs flex items-center justify-between border ${
            toastMessage.type === 'success'
              ? 'bg-[var(--success)]/10 text-[var(--success)] border-[var(--success)]/30'
              : 'bg-[var(--danger)]/10 text-[var(--danger)] border-[var(--danger)]/30'
          }`}
        >
          <span>{toastMessage.text}</span>
          <button onClick={() => setToastMessage(null)} className="cursor-pointer font-bold px-1">
            ✕
          </button>
        </div>
      )}

      {/* Navigation & Header */}
      <div className="tenant-page-header">
        <div className="flex items-start gap-3 min-w-0">
          <button
            onClick={onBack}
            className="w-11 shrink-0 flex items-center justify-center rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] cursor-pointer"
            title="Kembali ke Daftar Tenant"
          >
            <ArrowLeft className="w-4 h-4" />
          </button>

          <div className="min-w-0">
            <div className="tenant-detail-heading">
              <h1 className="text-xl font-bold tracking-tight text-[var(--ink)]">
                {tenant.username}
              </h1>
              <span className="font-mono text-xs text-[var(--primary)] px-2 py-0.5 rounded-[var(--radius-sm)] bg-[var(--primary)]/10">
                {tenant.linux_username}
              </span>
              <StatusPill
                variant={tenant.status === 'active' ? 'success' : 'warning'}
                label={tenant.status === 'active' ? 'Aktif' : 'Ditangguhkan'}
              />
            </div>
            <p className="text-xs text-[var(--ink-muted)] mt-0.5">
              UID Linux: <span className="font-mono font-semibold">{tenant.linux_uid || '—'}</span> &bull;{' '}
              {tenant.email || 'Tanpa email terdaftar'}
            </p>
          </div>
        </div>

        {/* Header Action Buttons */}
        <div className="tenant-header-actions">
          {user?.role === 'admin' && (
            <button
              onClick={handleImpersonate}
              className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] cursor-pointer"
              title="Masuk sebagai tenant ini untuk membantu konfigurasi"
            >
              <LogIn className="w-3.5 h-3.5" />
              <span>Impersonasi</span>
            </button>
          )}

          <button
            onClick={handleToggleSuspend}
            className={`flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--raised)] cursor-pointer ${
              tenant.status === 'active' ? 'text-[var(--ink)]' : 'text-[var(--success)]'
            }`}
          >
            {tenant.status === 'active' ? (
              <>
                <PauseCircle className="w-3.5 h-3.5 text-[var(--accent)]" />
                <span>Tangguhkan</span>
              </>
            ) : (
              <>
                <PlayCircle className="w-3.5 h-3.5 text-[var(--success)]" />
                <span>Aktifkan</span>
              </>
            )}
          </button>
        </div>
      </div>

      {/* Tabs Navigation */}
      <nav className="tenant-detail-tabs" aria-label="Bagian detail tenant">
        {[
          { id: 'overview', label: 'Ringkasan', icon: User },
          { id: 'resources', label: 'Penggunaan Resource (Live)', icon: Activity },
          { id: 'danger', label: 'Area Berbahaya', icon: AlertTriangle },
        ].map((tab) => {
          const Icon = tab.icon;
          const isActive = activeTab === tab.id;
          return (
            <button
              key={tab.id}
              aria-current={isActive ? 'page' : undefined}
              onClick={() => setActiveTab(tab.id as 'overview' | 'resources' | 'danger')}
              className={`flex items-center gap-2 pb-3 font-semibold transition-colors cursor-pointer border-b-2 -mb-[1px] ${
                isActive
                  ? 'border-[var(--primary)] text-[var(--primary)]'
                  : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
              }`}
            >
              <Icon className="w-4 h-4" />
              <span>{tab.label}</span>
            </button>
          );
        })}
      </nav>

      {/* Tab: Ringkasan (Overview) */}
      {activeTab === 'overview' && (
        <div className="space-y-6">
          <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
            <div className="p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] space-y-3">
              <h3 className="text-xs font-bold text-[var(--ink)] border-b border-[var(--line)] pb-2 flex items-center gap-2">
                <User className="w-4 h-4 text-[var(--primary)]" />
                <span>Identitas & Sistem Akun</span>
              </h3>
              <div className="tenant-detail-fields space-y-2 text-xs">
                <div className="flex justify-between py-1 border-b border-[var(--line)]/50">
                  <span className="text-[var(--ink-muted)]">Slug Pengguna:</span>
                  <span className="font-mono font-semibold text-[var(--ink)]">{tenant.username}</span>
                </div>
                <div className="flex justify-between py-1 border-b border-[var(--line)]/50">
                  <span className="text-[var(--ink-muted)]">User Linux Host:</span>
                  <span className="font-mono font-semibold text-[var(--primary)]">{tenant.linux_username || '—'}</span>
                </div>
                <div className="flex justify-between py-1 border-b border-[var(--line)]/50">
                  <span className="text-[var(--ink-muted)]">UID Linux:</span>
                  <span className="font-mono text-[var(--ink)]">{tenant.linux_uid || '—'}</span>
                </div>
                <div className="flex justify-between py-1 border-b border-[var(--line)]/50">
                  <span className="text-[var(--ink-muted)]">Direktori Home:</span>
                  <span className="font-mono text-[var(--ink)]">/home/{tenant.linux_username}</span>
                </div>
                <div className="flex justify-between py-1 border-b border-[var(--line)]/50">
                  <span className="text-[var(--ink-muted)]">Terdaftar Pada:</span>
                  <span className="text-[var(--ink)]">
                    {new Date(tenant.created_at * 1000).toLocaleString('id-ID')}
                  </span>
                </div>
                <div className="flex justify-between py-1">
                  <span className="text-[var(--ink-muted)]">Terakhir Diperbarui:</span>
                  <span className="text-[var(--ink)]">
                    {new Date(tenant.updated_at * 1000).toLocaleString('id-ID')}
                  </span>
                </div>
              </div>
            </div>

            <div className="p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] space-y-3">
              <h3 className="text-xs font-bold text-[var(--ink)] border-b border-[var(--line)] pb-2 flex items-center gap-2">
                <Layers className="w-4 h-4 text-[var(--primary)]" />
                <span>Paket Hosting & Batasan ({plan?.name || 'Tanpa Paket'})</span>
              </h3>
              {plan ? (
                <div className="tenant-detail-fields space-y-2 text-xs">
                  <div className="flex justify-between py-1 border-b border-[var(--line)]/50">
                    <span className="text-[var(--ink-muted)]">Batas CPU Quota:</span>
                    <span className="font-mono font-semibold text-[var(--ink)]">{plan.cpu_quota_percent}%</span>
                  </div>
                  <div className="flex justify-between py-1 border-b border-[var(--line)]/50">
                    <span className="text-[var(--ink-muted)]">Batas RAM (Memory):</span>
                    <span className="font-mono font-semibold text-[var(--ink)]">{formatBytes(plan.memory_limit_bytes)}</span>
                  </div>
                  <div className="flex justify-between py-1 border-b border-[var(--line)]/50">
                    <span className="text-[var(--ink-muted)]">Batas Kuota Disk:</span>
                    <span className="font-mono font-semibold text-[var(--ink)]">{formatBytes(plan.disk_limit_bytes)}</span>
                  </div>
                  <div className="flex justify-between py-1 border-b border-[var(--line)]/50">
                    <span className="text-[var(--ink-muted)]">Batas Proses (TasksMax):</span>
                    <span className="font-mono text-[var(--ink)]">{plan.tasks_max} proses</span>
                  </div>
                  <div className="flex justify-between py-1 border-b border-[var(--line)]/50">
                    <span className="text-[var(--ink-muted)]">Shell Policy Paket:</span>
                    <span className="font-mono text-[var(--primary)]">{plan.shell_policy}</span>
                  </div>
                  <div className="flex justify-between py-1">
                    <span className="text-[var(--ink-muted)]">Maksimal Domain / Web:</span>
                    <span className="text-[var(--ink)]">{plan.max_sites} situs ({plan.max_domains} domain)</span>
                  </div>
                </div>
              ) : (
                <div className="p-4 text-center text-xs text-[var(--ink-muted)]">
                  Tenant tidak memiliki paket hosting aktif.
                </div>
              )}
            </div>
          </div>

          {/* Kartu Kebijakan Shell & Akses Terminal (P6-T6) */}
          <div ref={shellPolicyCard} tabIndex={-1} aria-label="Kebijakan shell dan akses terminal" className="p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] space-y-4">
            <div className="tenant-hardening-header pb-3 border-b border-[var(--line)]">
              <div className="flex items-center gap-2 min-w-0">
                <Terminal className="w-4 h-4 text-[var(--primary)]" />
                <h3 className="text-xs font-bold text-[var(--ink)]">
                  Kebijakan Shell & Isolasi Akses Terminal
                </h3>
              </div>
              <div className="tenant-shell-badges">
                <StatusPill
                  variant={
                    (shellPolicyData?.effective_policy ?? plan?.shell_policy ?? 'none') === 'full'
                      ? 'success'
                      : (shellPolicyData?.effective_policy ?? plan?.shell_policy ?? 'none') === 'sftp'
                      ? 'warning'
                      : 'danger'
                  }
                  label={
                    (shellPolicyData?.effective_policy ?? plan?.shell_policy ?? 'none') === 'full'
                      ? 'full (Akses Shell & Web Terminal)'
                      : (shellPolicyData?.effective_policy ?? plan?.shell_policy ?? 'none') === 'sftp'
                      ? 'sftp (SFTP-Only via internal-sftp)'
                      : 'none (Tanpa Akses Shell)'
                  }
                />
                <span className="text-[10px] px-2 py-0.5 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface-sunken)] text-[var(--ink-muted)]">
                  {shellPolicyData?.inherited_from_plan
                    ? 'Mewarisi dari Paket'
                    : 'Override Khusus Tenant'}
                </span>
              </div>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-xs">
              <div className="space-y-2 text-[var(--ink-muted)]">
                <p>
                  Aturan shell mengontrol kemampuan akun tenant saat mengakses server melalui protokol SSH maupun Web Terminal:
                </p>
                <ul className="list-disc list-inside space-y-1 text-[11px]">
                  <li>
                    <strong className="text-[var(--ink)]">none:</strong> Pengguna dialihkan ke <code className="font-mono">/usr/sbin/nologin</code>. Akses SSH, SFTP, dan Web Terminal ditutup.
                  </li>
                  <li>
                    <strong className="text-[var(--ink)]">sftp:</strong> Diisolasi via drop-in <code className="font-mono">50-panel.conf</code> ke <code className="font-mono">internal-sftp</code> dengan ChrootDirectory <code className="font-mono">/home</code>. Shell interaktif dan Web Terminal diblokir.
                  </li>
                  <li>
                    <strong className="text-[var(--ink)]">full:</strong> Pengguna mendapatkan <code className="font-mono">/bin/bash</code>, dapat menggunakan Web Terminal dan SSH interaktif dengan isolasi cgroup TasksMax.
                  </li>
                </ul>
              </div>

              {user?.role === 'admin' ? (
                <div className="p-3 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface-sunken)] space-y-3">
                  <div className="flex items-center gap-1.5 text-xs font-semibold text-[var(--ink)]">
                    <ShieldCheck className="w-3.5 h-3.5 text-[var(--primary)]" />
                    <span>Kelola Override Kebijakan Shell (Admin)</span>
                  </div>
                  <div className="space-y-1.5">
                    <label htmlFor="tenant-shell-policy" className="text-[11px] text-[var(--ink-muted)] block">
                      Pilih kebijakan shell untuk tenant ini
                    </label>
                    <select
                      id="tenant-shell-policy"
                      value={selectedShellPolicy}
                      onChange={(e) => setSelectedShellPolicy(e.target.value)}
                      className="w-full text-xs px-2.5 py-1.5 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] focus:outline-hidden focus:ring-1 focus:ring-[var(--primary)]"
                    >
                      <option value="inherit">
                        Mewarisi dari Paket ({plan?.shell_policy || 'none'})
                      </option>
                      <option value="none">none (Kunci total akses terminal & SFTP)</option>
                      <option value="sftp">sftp (Hanya SFTP chroot, tanpa terminal)</option>
                      <option value="full">full (Akses shell interaktif & web terminal)</option>
                    </select>
                  </div>
                  <div className="flex flex-wrap items-center justify-end gap-3 pt-1">
                    {shellPolicyData?.effective_policy === 'full' && <RouteLink href={`/terminal?tenant=${tenantId}`} className="text-xs font-medium text-[var(--primary)]">Buka terminal</RouteLink>}
                    <button
                      onClick={handleSaveShellPolicy}
                      disabled={isSavingPolicy}
                      className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-sm)] bg-[var(--primary)] text-white hover:opacity-90 disabled:opacity-50 cursor-pointer shadow-xs flex items-center gap-1.5"
                    >
                      {isSavingPolicy ? (
                        <>
                          <RefreshCw className="w-3 h-3 animate-spin" />
                          <span>Menerapkan...</span>
                        </>
                      ) : (
                        <span>Simpan Kebijakan Shell</span>
                      )}
                    </button>
                  </div>
                </div>
              ) : (
                <div className="p-3 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface-sunken)] flex items-center">
                  <span className="text-[11px] text-[var(--ink-muted)]">
                    Hubungi administrator hosting jika Anda memerlukan izin perubahan kebijakan shell akun.
                  </span>
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Tab: Penggunaan Resource (Live Cgroup v2 & Quota) */}
      {activeTab === 'resources' && (
        <div className="space-y-6">
          <div className="tenant-hardening-header">
            <div className="text-xs text-[var(--ink-muted)]">
              Data dibaca langsung dari kernel cgroup v2 slice (<code className="font-mono text-[var(--ink)]">panel-u{tenant.linux_uid}.slice</code>) dan kuota filesystem Linux.
            </div>
            <button
              onClick={loadResources}
              disabled={isLoadingResources}
              className="flex items-center gap-1.5 px-3 py-1 text-xs font-medium rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--raised)] text-[var(--ink)] cursor-pointer disabled:opacity-50"
            >
              <RefreshCw className={`w-3.5 h-3.5 ${isLoadingResources ? 'animate-spin' : ''}`} />
              <span>Segarkan Metrik</span>
            </button>
          </div>

          <div className="tenant-resource-grid grid grid-cols-1 gap-4">
            {/* Memory Current vs Peak */}
            <div className="p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] space-y-3">
              <div className="flex items-center justify-between text-xs font-semibold text-[var(--ink)]">
                <span className="flex items-center gap-1.5">
                  <Activity className="w-4 h-4 text-[var(--primary)]" />
                  <span>Memori RAM (Cgroup v2)</span>
                </span>
                <span className="font-mono">{memPercent}%</span>
              </div>
              <div className="w-full bg-[var(--raised)] h-2 rounded-full overflow-hidden border border-[var(--line)]">
                <div
                  className={`h-full transition-all duration-300 ${
                    memPercent > 85 ? 'bg-[var(--danger)]' : memPercent > 65 ? 'bg-[var(--accent)]' : 'bg-[var(--primary)]'
                  }`}
                  style={{ width: `${memPercent}%` }}
                />
              </div>
              <div className="flex flex-wrap gap-2 justify-between text-[11px] text-[var(--ink-muted)] font-mono">
                <span>Saat ini: {formatBytes(memCurrent)}</span>
                <span>Batas: {formatBytes(memLimit)}</span>
              </div>
              {resources?.metrics.memory_peak_bytes && (
                <div className="text-[10px] text-[var(--ink-muted)] font-mono pt-1 border-t border-[var(--line)]/50">
                  Peak penggunaan: {formatBytes(resources.metrics.memory_peak_bytes)}
                </div>
              )}
            </div>

            {/* Disk Quota */}
            <div className="p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] space-y-3">
              <div className="flex items-center justify-between text-xs font-semibold text-[var(--ink)]">
                <span className="flex items-center gap-1.5">
                  <HardDrive className="w-4 h-4 text-[var(--primary)]" />
                  <span>Kuota Penyimpanan Berkas</span>
                </span>
                <span className="font-mono">{diskPercent}%</span>
              </div>
              <div className="w-full bg-[var(--raised)] h-2 rounded-full overflow-hidden border border-[var(--line)]">
                <div
                  className={`h-full transition-all duration-300 ${
                    diskPercent > 85 ? 'bg-[var(--danger)]' : diskPercent > 65 ? 'bg-[var(--accent)]' : 'bg-[var(--primary)]'
                  }`}
                  style={{ width: `${diskPercent}%` }}
                />
              </div>
              <div className="flex flex-wrap gap-2 justify-between text-[11px] text-[var(--ink-muted)] font-mono">
                <span>Terpakai: {formatBytes(diskUsed)}</span>
                <span>Limit: {formatBytes(diskLimit)}</span>
              </div>
              <div className="text-[10px] text-[var(--ink-muted)] font-mono pt-1 border-t border-[var(--line)]/50 flex flex-wrap gap-2 justify-between">
                <span>Backend: {resources?.quota?.backend || 'ext4/fallback'}</span>
                <span>Inodes: {resources?.quota?.inodes_used || 0} files</span>
              </div>
            </div>

            {/* PIDs (TasksMax) */}
            <div className="p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] space-y-3">
              <div className="flex items-center justify-between text-xs font-semibold text-[var(--ink)]">
                <span className="flex items-center gap-1.5">
                  <Cpu className="w-4 h-4 text-[var(--primary)]" />
                  <span>Proses Aktif (PIDs)</span>
                </span>
                <span className="font-mono">{pidsPercent}%</span>
              </div>
              <div className="w-full bg-[var(--raised)] h-2 rounded-full overflow-hidden border border-[var(--line)]">
                <div
                  className={`h-full transition-all duration-300 ${
                    pidsPercent > 85 ? 'bg-[var(--danger)]' : pidsPercent > 65 ? 'bg-[var(--accent)]' : 'bg-[var(--primary)]'
                  }`}
                  style={{ width: `${pidsPercent}%` }}
                />
              </div>
              <div className="flex flex-wrap gap-2 justify-between text-[11px] text-[var(--ink-muted)] font-mono">
                <span>Proses berjalan: {pidsCurrent}</span>
                <span>Maks: {pidsLimit} tasks</span>
              </div>
              {resources?.metrics.pids_peak && (
                <div className="text-[10px] text-[var(--ink-muted)] font-mono pt-1 border-t border-[var(--line)]/50">
                  Peak proses: {resources.metrics.pids_peak}
                </div>
              )}
            </div>
          </div>

          {/* Detailed Statistics Table */}
          <div className="p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] space-y-3">
            <h4 className="text-xs font-bold text-[var(--ink)]">Rincian Status Slice Linux</h4>
            <div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-4 text-xs font-mono">
              <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
                <span className="text-[var(--ink-muted)] block text-[11px]">Status Slice</span>
                <span className="font-bold text-[var(--primary)]">
                  {resources?.metrics.exists ? 'Aktif' : 'Non-aktif / Standby'}
                </span>
              </div>
              <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
                <span className="text-[var(--ink-muted)] block text-[11px]">Total Waktu CPU</span>
                <span className="font-bold text-[var(--ink)]">
                  {((resources?.metrics.cpu_usage_usec || 0) / 1_000_000).toFixed(2)} detik
                </span>
              </div>
              <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
                <span className="text-[var(--ink-muted)] block text-[11px]">I/O Baca (Read)</span>
                <span className="font-bold text-[var(--ink)]">
                  {formatBytes(resources?.metrics.io_read_bytes || 0)}
                </span>
              </div>
              <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
                <span className="text-[var(--ink-muted)] block text-[11px]">I/O Tulis (Write)</span>
                <span className="font-bold text-[var(--ink)]">
                  {formatBytes(resources?.metrics.io_write_bytes || 0)}
                </span>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Tab: Danger Zone */}
      {activeTab === 'danger' && (
        <div className="p-5 rounded-[var(--radius-md)] border border-[var(--danger)]/30 bg-[var(--danger)]/5 space-y-5">
          <div>
            <h3 className="text-sm font-bold text-[var(--danger)] flex items-center gap-2">
              <AlertTriangle className="w-4 h-4" />
              <span>Area Berbahaya (Danger Zone)</span>
            </h3>
            <p className="text-xs text-[var(--ink-muted)] mt-1">
              Tindakan di area ini berdampak langsung pada operasional situs dan akun pengguna Linux host.
            </p>
          </div>

          <div className="space-y-4 pt-2 border-t border-[var(--danger)]/20">
            {/* Suspend / Unsuspend */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 p-3.5 rounded-[var(--radius-sm)] bg-[var(--surface)] border border-[var(--line)]">
              <div>
                <h4 className="text-xs font-bold text-[var(--ink)]">
                  {tenant.status === 'active' ? 'Tangguhkan Tenant Ini' : 'Aktifkan Kembali Tenant'}
                </h4>
                <p className="text-[11px] text-[var(--ink-muted)] mt-0.5">
                  {tenant.status === 'active'
                    ? 'Mematikan seluruh proses aktif tenant dan mengunci shell menjadi nologin.'
                    : 'Mengembalikan akses login dan memulihkan shell pengguna.'}
                </p>
              </div>
              <button
                onClick={handleToggleSuspend}
                className={`px-3 py-1.5 rounded-[var(--radius-sm)] text-xs font-semibold cursor-pointer shrink-0 border ${
                  tenant.status === 'active'
                    ? 'border-[var(--accent)] text-[var(--accent)] hover:bg-[var(--accent)]/10'
                    : 'border-[var(--success)] text-[var(--success)] hover:bg-[var(--success)]/10'
                }`}
              >
                {tenant.status === 'active' ? 'Tangguhkan Akun' : 'Aktifkan Akun'}
              </button>
            </div>

            {/* Delete Tenant */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 p-3.5 rounded-[var(--radius-sm)] bg-[var(--surface)] border border-[var(--danger)]/30">
              <div>
                <h4 className="text-xs font-bold text-[var(--danger)]">Hapus Tenant Permanen</h4>
                <p className="text-[11px] text-[var(--ink-muted)] mt-0.5">
                  Menghapus akun Linux, dropin cgroup, kuota disk, serta seluruh konfigurasi situs milik tenant ini.
                </p>
              </div>
              <button
                onClick={() => setIsDeleteDialogOpen(true)}
                className="px-3 py-1.5 rounded-[var(--radius-sm)] bg-[var(--danger)] text-white text-xs font-semibold hover:opacity-90 cursor-pointer shrink-0"
              >
                Hapus Tenant...
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Confirm Danger Modal */}
      <ConfirmDangerDialog
        isOpen={isDeleteDialogOpen}
        onClose={() => setIsDeleteDialogOpen(false)}
        onConfirm={handleConfirmDelete}
        title={`Hapus Tenant '${tenant.username}'?`}
        description={`Penghapusan akun tenant '${tenant.username}' (${tenant.linux_username}) bersifat permanen.`}
        confirmMatchText={tenant.username}
        confirmButtonLabel="Hapus Tenant Permanen"
        hasPurgeOption={true}
        purgeOptionLabel="Hapus seluruh berkas direktori home (/home/p_<slug>) dan database"
        isLoading={isDeleting}
      />
    </div>
  );
};
