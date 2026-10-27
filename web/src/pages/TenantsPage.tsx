import React, { useState, useEffect, useCallback } from 'react';
import {
  UserPlus,
  Search,
  Filter,
  RefreshCw,
  Eye,
  LogIn,
  PauseCircle,
  PlayCircle,
  Trash2,
  ShieldCheck,
  Layers,
  CheckCircle2,
  XCircle,
} from 'lucide-react';
import { TenantListItem, HardeningStatus } from '../types/api';
import {
  fetchTenants,
  suspendTenant,
  unsuspendTenant,
  deleteTenant,
  impersonateTenant,
  fetchHardeningStatus,
  applyHardening,
} from '../api/client';
import { StatusPill } from '../components/ui/StatusPill';
import { TenantWizardDialog } from '../components/tenants/TenantWizardDialog';
import { ConfirmDangerDialog } from '../components/ui/ConfirmDangerDialog';
import { useAuth } from '../context/AuthContext';
import '../styles/tenants.css';

export interface TenantsPageProps {
  onSelectTenant?: (id: number) => void;
  onNavigateToPlans?: () => void;
}

export const TenantsPage: React.FC<TenantsPageProps> = ({
  onSelectTenant = () => {},
  onNavigateToPlans = () => {},
}) => {
  const { user } = useAuth();
  const [tenants, setTenants] = useState<TenantListItem[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [searchTerm, setSearchTerm] = useState('');
  const [statusFilter, setStatusFilter] = useState<'all' | 'active' | 'suspended'>('all');

  // Hardening diagnostic state
  const [hardening, setHardening] = useState<HardeningStatus | null>(null);
  const [isApplyingHardening, setIsApplyingHardening] = useState(false);

  // Modals state
  const [isWizardOpen, setIsWizardOpen] = useState(false);
  const [tenantToDelete, setTenantToDelete] = useState<TenantListItem | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const [toastMessage, setToastMessage] = useState<{ text: string; type: 'success' | 'error' } | null>(null);

  const loadData = useCallback(async () => {
    setIsLoading(true);
    try {
      const [tenantsList, hardeningStatus] = await Promise.all([
        fetchTenants({
          search: searchTerm || undefined,
          status: statusFilter !== 'all' ? statusFilter : undefined,
        }),
        fetchHardeningStatus().catch(() => null),
      ]);
      setTenants(tenantsList);
      if (hardeningStatus) setHardening(hardeningStatus);
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({ text: e.message || 'Gagal memuat daftar tenant', type: 'error' });
    } finally {
      setIsLoading(false);
    }
  }, [searchTerm, statusFilter]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  const handleApplyHardening = async () => {
    setIsApplyingHardening(true);
    try {
      const res = await applyHardening({
        enable_hidepid: true,
        ensure_tenant_group: true,
      });
      setToastMessage({ text: res.message || 'Penguatan host berhasil diterapkan', type: 'success' });
      const updated = await fetchHardeningStatus();
      setHardening(updated);
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({ text: e.message || 'Gagal menerapkan penguatan host', type: 'error' });
    } finally {
      setIsApplyingHardening(false);
    }
  };

  const handleToggleSuspend = async (tenant: TenantListItem) => {
    try {
      if (tenant.status === 'active') {
        await suspendTenant(tenant.id);
        setToastMessage({ text: `Tenant '${tenant.username}' berhasil ditangguhkan`, type: 'success' });
      } else {
        await unsuspendTenant(tenant.id);
        setToastMessage({ text: `Tenant '${tenant.username}' berhasil diaktifkan kembali`, type: 'success' });
      }
      loadData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({ text: e.message || 'Gagal mengubah status tenant', type: 'error' });
    }
  };

  const handleImpersonate = async (tenant: TenantListItem) => {
    try {
      await impersonateTenant(tenant.id);
      window.location.reload();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({ text: e.message || 'Gagal mengimpersonasi tenant', type: 'error' });
    }
  };

  const handleConfirmDelete = async (purgeFiles?: boolean) => {
    if (!tenantToDelete) return;
    setIsDeleting(true);
    try {
      await deleteTenant(tenantToDelete.id, purgeFiles);
      setToastMessage({ text: `Tenant '${tenantToDelete.username}' berhasil dihapus`, type: 'success' });
      setTenantToDelete(null);
      loadData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({ text: e.message || 'Gagal menghapus tenant', type: 'error' });
    } finally {
      setIsDeleting(false);
    }
  };

  const renderActions = (tenant: TenantListItem, mobile = false) => (
    <div className={mobile ? 'tenant-card-actions' : 'tenant-table-actions'}>
      <button type="button" onClick={() => onSelectTenant(tenant.id)} aria-label={`Buka detail tenant ${tenant.username}`} title="Buka detail tenant">
        <Eye aria-hidden="true" /><span>Detail tenant</span>
      </button>
      {user?.role === 'admin' && (
        <button type="button" onClick={() => handleImpersonate(tenant)} aria-label={`Impersonasi tenant ${tenant.username}`} title="Impersonasi akun ini">
          <LogIn aria-hidden="true" /><span>Impersonasi</span>
        </button>
      )}
      <button type="button" onClick={() => handleToggleSuspend(tenant)} aria-label={`${tenant.status === 'active' ? 'Tangguhkan' : 'Aktifkan kembali'} tenant ${tenant.username}`} title={tenant.status === 'active' ? 'Tangguhkan tenant' : 'Aktifkan kembali tenant'}>
        {tenant.status === 'active' ? <PauseCircle aria-hidden="true" /> : <PlayCircle aria-hidden="true" />}
        <span>{tenant.status === 'active' ? 'Tangguhkan' : 'Aktifkan kembali'}</span>
      </button>
      <button type="button" onClick={() => setTenantToDelete(tenant)} className="tenant-delete" aria-label={`Hapus tenant ${tenant.username}`} title="Hapus tenant">
        <Trash2 aria-hidden="true" /><span>Hapus tenant</span>
      </button>
    </div>
  );

  return (
    <div className="tenant-page space-y-6" data-testid="tenants-page">
      {/* Toast Alert */}
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

      {/* Page Header */}
      <div className="tenant-page-header">
        <div className="min-w-0">
          <h1 className="text-xl font-bold tracking-tight text-[var(--ink)]">
            Manajemen Tenant & Isolasi
          </h1>
          <p className="text-xs text-[var(--ink-muted)] mt-1">
            Setiap tenant diisolasi penuh dalam 1 akun Linux sistem dengan kontrol cgroup v2 dan batasan kuota berkas.
          </p>
        </div>

        <div className="tenant-header-actions">
          <button
            onClick={onNavigateToPlans}
            className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface)] hover:bg-[var(--raised)] text-[var(--ink)] cursor-pointer"
          >
            <Layers className="w-3.5 h-3.5 text-[var(--primary)]" />
            <span>Paket Hosting</span>
          </button>
          <button
            onClick={() => setIsWizardOpen(true)}
            className="flex items-center gap-1.5 px-3.5 py-1.5 text-xs font-semibold rounded-[var(--radius-sm)] bg-[var(--primary)] text-white hover:opacity-90 shadow-xs cursor-pointer"
          >
            <UserPlus className="w-3.5 h-3.5" />
            <span>Tambah Tenant</span>
          </button>
        </div>
      </div>

      {/* Hardening Status Card (PRD 5.3 Host Hardening & Prerequisites) */}
      {hardening && (
        <div className="p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] space-y-3">
          <div className="tenant-hardening-header">
            <div className="tenant-hardening-title text-xs font-bold text-[var(--ink)] min-w-0">
              <ShieldCheck className="w-4 h-4 text-[var(--primary)] shrink-0" />
              <span>Diagnostik Pengerasan Host & Isolasi OS</span>
              {hardening.overall_ok ? (
                <span className="text-[11px] font-normal px-2 py-0.5 rounded-full bg-[var(--success)]/10 text-[var(--success)] border border-[var(--success)]/20">
                  Semua Prasyarat Terpenuhi
                </span>
              ) : (
                <span className="text-[11px] font-normal px-2 py-0.5 rounded-full bg-[var(--danger)]/10 text-[var(--danger)] border border-[var(--danger)]/20">
                  Perlu Tindakan Pengerasan
                </span>
              )}
            </div>

            {!hardening.overall_ok && (
              <button
                onClick={handleApplyHardening}
                disabled={isApplyingHardening}
                className="px-3 py-1 text-xs font-semibold rounded-[var(--radius-sm)] bg-[var(--primary)] text-[var(--primary-foreground)] hover:opacity-90 cursor-pointer disabled:opacity-50"
              >
                {isApplyingHardening ? 'Menerapkan...' : 'Terapkan Penguatan Host'}
              </button>
            )}
          </div>

          <div className="tenant-hardening-checks grid gap-3 pt-3 border-t border-[var(--line)]/60 text-xs">
            <div className="flex items-center gap-1.5">
              {hardening.cgroup_v2.ok ? (
                <CheckCircle2 className="w-3.5 h-3.5 text-[var(--success)] shrink-0" />
              ) : (
                <XCircle className="w-3.5 h-3.5 text-[var(--danger)] shrink-0" />
              )}
              <span className="text-[var(--ink-muted)] truncate" title={hardening.cgroup_v2.message}>
                cgroup v2
              </span>
            </div>

            <div className="flex items-center gap-1.5">
              {hardening.hidepid.ok ? (
                <CheckCircle2 className="w-3.5 h-3.5 text-[var(--success)] shrink-0" />
              ) : (
                <XCircle className="w-3.5 h-3.5 text-[var(--danger)] shrink-0" />
              )}
              <span className="text-[var(--ink-muted)] truncate" title={hardening.hidepid.message}>
                hidepid=2 /proc
              </span>
            </div>

            <div className="flex items-center gap-1.5">
              {hardening.tenant_group.ok ? (
                <CheckCircle2 className="w-3.5 h-3.5 text-[var(--success)] shrink-0" />
              ) : (
                <XCircle className="w-3.5 h-3.5 text-[var(--danger)] shrink-0" />
              )}
              <span className="text-[var(--ink-muted)] truncate" title={hardening.tenant_group.message}>
                Grup panel-tenants
              </span>
            </div>

            <div className="flex items-center gap-1.5">
              {hardening.quota.ok ? (
                <CheckCircle2 className="w-3.5 h-3.5 text-[var(--success)] shrink-0" />
              ) : (
                <XCircle className="w-3.5 h-3.5 text-[var(--danger)] shrink-0" />
              )}
              <span className="text-[var(--ink-muted)] truncate" title={hardening.quota.message}>
                Dukungan Kuota FS
              </span>
            </div>

            <div className="flex items-center gap-1.5">
              {hardening.acl.ok ? (
                <CheckCircle2 className="w-3.5 h-3.5 text-[var(--success)] shrink-0" />
              ) : (
                <XCircle className="w-3.5 h-3.5 text-[var(--danger)] shrink-0" />
              )}
              <span className="text-[var(--ink-muted)] truncate" title={hardening.acl.message}>
                POSIX ACL
              </span>
            </div>
          </div>
        </div>
      )}

      {/* Filters and Search Bar */}
      <div className="tenant-filters p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)]">
        <div className="tenant-search relative">
          <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-[var(--ink-muted)]" />
          <input
            type="text"
            aria-label="Cari tenant berdasarkan slug atau email"
            placeholder="Cari nama slug atau email..."
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            className="w-full pl-8 pr-3 py-1.5 text-xs rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          />
        </div>

        <div className="tenant-status-filter">
          <div className="tenant-filter-label flex items-center gap-1.5 text-xs text-[var(--ink-muted)]" id="tenant-status-label">
            <Filter className="w-3.5 h-3.5" aria-hidden="true" />
            <span>Status tenant</span>
          </div>

          <div role="group" aria-labelledby="tenant-status-label" className="tenant-filter-options rounded-[var(--radius-sm)] border border-[var(--line)] p-0.5 bg-[var(--bg)] text-xs">
            {(['all', 'active', 'suspended'] as const).map((st) => (
              <button
                key={st}
                aria-pressed={statusFilter === st}
                onClick={() => setStatusFilter(st)}
                className={`px-2.5 py-1 rounded-[var(--radius-sm)] capitalize font-medium cursor-pointer transition-colors ${
                  statusFilter === st
                    ? 'bg-[var(--surface)] text-[var(--primary)] font-semibold shadow-xs'
                    : 'text-[var(--ink-muted)] hover:text-[var(--ink)]'
                }`}
              >
                {st === 'all' ? 'Semua' : st === 'active' ? 'Aktif' : 'Ditangguhkan'}
              </button>
            ))}
          </div>

          <button
            onClick={loadData}
            className="p-1.5 rounded-[var(--radius-sm)] border border-[var(--line)] text-[var(--ink-muted)] hover:bg-[var(--raised)] hover:text-[var(--ink)] cursor-pointer ml-1"
            title="Muat Ulang"
            aria-label="Muat ulang daftar tenant"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? 'animate-spin' : ''}`} />
          </button>
        </div>
      </div>

      <section className="tenant-mobile-list" aria-label="Daftar tenant" aria-busy={isLoading}>
        {tenants.length === 0 ? (
          <p role="status" className="p-6 text-sm text-center text-[var(--ink-muted)] border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--surface)]">
            {isLoading ? 'Memuat daftar tenant...' : 'Tidak ada tenant yang cocok dengan kriteria pencarian.'}
          </p>
        ) : (
          <ul className="space-y-4">
            {tenants.map((tenant) => (
              <li key={tenant.id} className="tenant-card p-4 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)]">
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <div className="min-w-0 flex-1">
                    <h2 className="text-sm font-semibold text-[var(--ink)]">{tenant.username}</h2>
                    <p className="text-xs text-[var(--ink-muted)] mt-1">{tenant.email || 'Tanpa email terdaftar'}</p>
                  </div>
                  <StatusPill variant={tenant.status === 'active' ? 'success' : 'warning'} label={tenant.status === 'active' ? 'Aktif' : 'Ditangguhkan'} />
                </div>
                <dl className="tenant-card-fields">
                  <div><dt>User Linux / UID</dt><dd className="font-mono">{tenant.linux_username || '—'} <span className="text-[var(--ink-muted)]">/ {tenant.linux_uid ?? '—'}</span></dd></div>
                  <div><dt>Paket hosting</dt><dd>{tenant.plan_name || 'Tanpa paket'}</dd></div>
                  <div><dt>Terdaftar</dt><dd>{new Date(tenant.created_at * 1000).toLocaleDateString('id-ID', { day: 'numeric', month: 'short', year: 'numeric' })}</dd></div>
                </dl>
                {renderActions(tenant, true)}
              </li>
            ))}
          </ul>
        )}
      </section>

      {/* Keep the complete table on screens with room for all six columns. */}
      <div className="tenant-desktop-list rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] overflow-hidden" aria-busy={isLoading}>
        <div className="overflow-x-auto">
          <table aria-label="Daftar tenant" className="w-full text-left text-xs">
            <thead className="bg-[var(--raised)] border-b border-[var(--line)] text-[var(--ink-muted)] uppercase tracking-wider text-[11px] font-semibold">
              <tr>
                <th className="py-3 px-4">Tenant</th>
                <th className="py-3 px-4">User Linux / UID</th>
                <th className="py-3 px-4">Paket Hosting</th>
                <th className="py-3 px-4">Status</th>
                <th className="py-3 px-4">Terdaftar</th>
                <th className="py-3 px-4 text-right">Aksi</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[var(--line)]">
              {isLoading && tenants.length === 0 ? (
                <tr>
                  <td colSpan={6} className="py-8 text-center text-xs text-[var(--ink-muted)]">
                    Memuat daftar tenant...
                  </td>
                </tr>
              ) : tenants.length === 0 ? (
                <tr>
                  <td colSpan={6} className="py-10 text-center text-xs text-[var(--ink-muted)]">
                    Tidak ada tenant yang cocok dengan kriteria pencarian.
                  </td>
                </tr>
              ) : (
                tenants.map((t) => (
                  <tr key={t.id} className="hover:bg-[var(--raised)]/50 transition-colors">
                    <td className="py-3 px-4">
                      <div className="font-semibold text-[var(--ink)]">{t.username}</div>
                      <div className="text-[11px] text-[var(--ink-muted)]">{t.email || '—'}</div>
                    </td>

                    <td className="py-3 px-4 font-mono text-[11px]">
                      <div className="text-[var(--primary)] font-semibold">{t.linux_username || '—'}</div>
                      <div className="text-[10px] text-[var(--ink-muted)]">UID: {t.linux_uid || '—'}</div>
                    </td>

                    <td className="py-3 px-4">
                      <span className="px-2 py-0.5 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-xs font-medium text-[var(--ink)]">
                        {t.plan_name || 'Tanpa Paket'}
                      </span>
                    </td>

                    <td className="py-3 px-4">
                      <StatusPill
                        variant={t.status === 'active' ? 'success' : 'warning'}
                        label={t.status === 'active' ? 'Aktif' : 'Ditangguhkan'}
                      />
                    </td>

                    <td className="py-3 px-4 text-[var(--ink-muted)]">
                      {new Date(t.created_at * 1000).toLocaleDateString('id-ID', {
                        day: 'numeric',
                        month: 'short',
                        year: 'numeric',
                      })}
                    </td>

                    <td className="py-3 px-4 text-right">
                      {renderActions(t)}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </div>

      {/* Tenant Wizard Modal */}
      <TenantWizardDialog
        isOpen={isWizardOpen}
        onClose={() => setIsWizardOpen(false)}
        onSuccess={(created) => {
          setToastMessage({
            text: `Tenant '${created.tenant.username}' (${created.tenant.linux_username}) berhasil dibuat`,
            type: 'success',
          });
          loadData();
        }}
      />

      {/* Confirm Danger Modal for Tenant Deletion */}
      <ConfirmDangerDialog
        isOpen={Boolean(tenantToDelete)}
        onClose={() => setTenantToDelete(null)}
        onConfirm={handleConfirmDelete}
        title={`Hapus Tenant '${tenantToDelete?.username}'?`}
        description={`Penghapusan ini akan menghapus akun Linux sistem (${tenantToDelete?.linux_username}), slice cgroup v2, dan kuota disk terkait. Anda juga dapat memilih untuk menghapus direktori berkas secara permanen.`}
        confirmMatchText={tenantToDelete?.username || ''}
        confirmButtonLabel="Hapus Tenant Permanen"
        hasPurgeOption={true}
        purgeOptionLabel="Hapus bersih seluruh file direktori home (/home/p_<slug>) dan database"
        isLoading={isDeleting}
      />
    </div>
  );
};
