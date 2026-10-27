import React, { useState, useEffect, useCallback } from 'react';
import {
  Globe,
  Plus,
  Search,
  Filter,
  RefreshCw,
  Eye,
  ExternalLink,
  Power,
  Trash2,
  Lock,
  LockOpen,
  FileCode2,
  AlertCircle,
  CheckCircle2,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Site } from '../types/api';
import { fetchSites, enableSite, disableSite, deleteSite } from '../api/client';
import { StatusPill } from '../components/ui/StatusPill';
import { PulseStrip } from '../components/ui/PulseStrip';
import { SiteWizardDialog } from '../components/sites/SiteWizardDialog';
import { ConfirmDangerDialog } from '../components/ui/ConfirmDangerDialog';
import { EmptyState } from '../components/ui/EmptyState';

export interface SitesPageProps {
  onSelectSite?: (site: Site) => void;
}

export const SitesPage: React.FC<SitesPageProps> = ({ onSelectSite = () => {} }) => {
  const { t } = useTranslation();
  const [sites, setSites] = useState<Site[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [searchTerm, setSearchTerm] = useState('');
  const [statusFilter, setStatusFilter] = useState<'all' | 'active' | 'disabled'>('all');

  // Modal dialog states
  const [isWizardOpen, setIsWizardOpen] = useState(false);
  const [siteToDelete, setSiteToDelete] = useState<Site | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const [toastMessage, setToastMessage] = useState<{
    text: string;
    type: 'success' | 'error';
  } | null>(null);

  const loadSites = useCallback(async () => {
    setIsLoading(true);
    try {
      const data = await fetchSites();
      setSites(data);
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal memuat daftar situs',
        type: 'error',
      });
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadSites();
  }, [loadSites]);

  // Handle enable/disable toggle
  const handleToggleStatus = async (site: Site) => {
    const isCurrentlyActive = site.status === 'active';
    try {
      if (isCurrentlyActive) {
        await disableSite(site.id);
        setToastMessage({
          text: `Situs ${site.primary_domain} dinonaktifkan`,
          type: 'success',
        });
      } else {
        await enableSite(site.id);
        setToastMessage({
          text: `Situs ${site.primary_domain} diaktifkan`,
          type: 'success',
        });
      }
      loadSites();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal mengubah status situs',
        type: 'error',
      });
    }
  };

  // Handle delete confirmed
  const handleConfirmDelete = async () => {
    if (!siteToDelete) return;
    setIsDeleting(true);
    try {
      await deleteSite(siteToDelete.id);
      setToastMessage({
        text: `Situs ${siteToDelete.primary_domain} berhasil dihapus`,
        type: 'success',
      });
      setSiteToDelete(null);
      loadSites();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal menghapus situs',
        type: 'error',
      });
    } finally {
      setIsDeleting(false);
    }
  };

  // Filter sites
  const filteredSites = sites.filter((site) => {
    const matchesSearch =
      site.primary_domain.toLowerCase().includes(searchTerm.toLowerCase()) ||
      site.domains.some((d) => d.name.toLowerCase().includes(searchTerm.toLowerCase()));
    const matchesStatus =
      statusFilter === 'all' ||
      (statusFilter === 'active' && site.status === 'active') ||
      (statusFilter === 'disabled' && site.status !== 'active');
    return matchesSearch && matchesStatus;
  });

  return (
    <div className="hosting-sites space-y-6">
      {/* Toast Alert */}
      {toastMessage && (
        <div
          className={`p-3 rounded-[var(--radius-md)] text-xs flex items-center justify-between border ${
            toastMessage.type === 'success'
              ? 'bg-[var(--success)]/10 border-[var(--success)]/20 text-[var(--success)]'
              : 'bg-[var(--danger)]/10 border-[var(--danger)]/20 text-[var(--danger)]'
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
            className="text-xs font-semibold hover:underline"
          >
            Tutup
          </button>
        </div>
      )}

      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h1 className="text-xl font-bold tracking-tight text-[var(--ink)]">
            {t('nav.sites')}
          </h1>
          <p className="text-xs text-[var(--ink-muted)] mt-1">
            {t('design.manageWebsites')}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={loadSites}
            disabled={isLoading}
            className="p-2 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] transition-colors disabled:opacity-50"
            title={t('common.refresh')}
          >
            <RefreshCw className={`w-4 h-4 ${isLoading ? 'animate-spin' : ''}`} />
          </button>

          <button
            onClick={() => setIsWizardOpen(true)}
            className="px-4 py-2 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] transition-colors flex items-center gap-1.5 shadow-xs"
          >
            <Plus className="w-4 h-4" />
            {t('sites.newSite')}
          </button>
        </div>
      </div>

      {/* Search and Filters Bar */}
      <div className="site-filters flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3">
        <div className="relative flex-1">
          <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-[var(--ink-muted)]" />
          <input
            type="text"
            aria-label={t('sites.searchPlaceholder')}
            placeholder={t('sites.searchPlaceholder')}
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            className="w-full pl-9 pr-3 py-1.5 text-xs rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          />
        </div>

        <div className="flex items-center gap-2">
          <Filter className="w-4 h-4 text-[var(--ink-muted)] shrink-0" />
          <select
            aria-label={t('sites.table.status')}
            value={statusFilter}
            onChange={(e) =>
              setStatusFilter(e.target.value as 'all' | 'active' | 'disabled')
            }
            className="text-xs py-1.5 px-2.5 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
          >
            <option value="all">{t('sites.statusFilterAll')}</option>
            <option value="active">{t('sites.statusFilterActive')}</option>
            <option value="disabled">{t('sites.statusFilterDisabled')}</option>
          </select>
        </div>
      </div>

      {/* Sites List or Empty State */}
      {isLoading ? (
        <div className="p-12 text-center text-xs text-[var(--ink-muted)] border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--surface)] flex items-center justify-center gap-2">
          <RefreshCw className="w-4 h-4 animate-spin text-[var(--primary)]" />
          Memuat data situs...
        </div>
      ) : filteredSites.length === 0 ? (
        <EmptyState
          icon={Globe}
          title={searchTerm ? 'Situs tidak ditemukan' : t('sites.emptyTitle')}
          description={
            searchTerm
              ? `Tidak ada situs yang cocok dengan pencarian "${searchTerm}".`
              : t('sites.emptyDesc')
          }
          actionLabel={searchTerm ? undefined : t('sites.createFirst')}
          onAction={searchTerm ? undefined : () => setIsWizardOpen(true)}
        />
      ) : (
        <div className="border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--surface)] overflow-hidden shadow-xs">
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs border-collapse">
              <thead>
                <tr className="border-b border-[var(--line)] bg-[var(--raised)] text-[var(--ink-muted)] font-semibold">
                  <th className="py-3 px-4">{t('sites.table.domain')}</th>
                  <th className="py-3 px-4">{t('sites.table.type')}</th>
                  <th className="py-3 px-4">{t('sites.table.activity')}</th>
                  <th className="py-3 px-4">{t('sites.table.status')}</th>
                  <th className="py-3 px-4">{t('sites.table.ssl')}</th>
                  <th className="py-3 px-4 text-right">{t('sites.table.actions')}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--line)] text-[var(--ink)]">
                {filteredSites.map((site) => {
                  const isActive = site.status === 'active';
                  const domainCount = site.domains.length;

                  return (
                    <tr
                      key={site.id}
                      className="hover:bg-[var(--raised)]/50 transition-colors group"
                    >
                      {/* Domain column */}
                      <td className="py-3 px-4">
                        <div className="flex items-center gap-2.5">
                          <div className="w-7 h-7 rounded-[var(--radius-sm)] bg-[var(--primary)]/10 text-[var(--primary)] flex items-center justify-center shrink-0">
                            <Globe className="w-4 h-4" />
                          </div>
                          <div>
                            <button
                              onClick={() => onSelectSite(site)}
                              className="font-semibold text-sm text-[var(--ink)] hover:text-[var(--primary)] hover:underline flex items-center gap-1 text-left"
                            >
                              {site.primary_domain}
                            </button>
                            <div className="text-[11px] text-[var(--ink-muted)] flex items-center gap-1.5 mt-0.5">
                              <span>User ID: {site.user_id}</span>
                              {domainCount > 1 && (
                                <span className="px-1.5 py-0.2 rounded-full bg-[var(--bg)] border border-[var(--line)] text-[10px]">
                                  +{domainCount - 1} alias
                                </span>
                              )}
                            </div>
                          </div>
                        </div>
                      </td>

                      {/* Type column */}
                      <td className="py-3 px-4">
                        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-[var(--radius-sm)] text-[11px] font-medium bg-[var(--bg)] border border-[var(--line)] text-[var(--ink)]">
                          <FileCode2 className="w-3 h-3 text-[var(--primary)]" />
                          {site.kind === 'static' ? 'Static' : site.kind.toUpperCase()}
                          {site.spa_fallback && (
                            <span className="text-[10px] text-[var(--ink-muted)] font-mono">
                              (SPA)
                            </span>
                          )}
                        </span>
                      </td>

                      {/* Activity sparkline */}
                      <td className="py-3 px-4">
                        <PulseStrip
                          siteId={site.id}
                          width={95}
                          height={18}
                          showTooltip={true}
                        />
                      </td>

                      {/* Status */}
                      <td className="py-3 px-4">
                        <StatusPill
                          variant={isActive ? 'success' : 'neutral'}
                          label={isActive ? 'Aktif' : 'Nonaktif'}
                        />
                      </td>

                      {/* SSL Status */}
                      <td className="py-3 px-4">
                        {site.force_https ? (
                          <span className="inline-flex items-center gap-1 text-[11px] font-medium text-[var(--success)]">
                            <Lock className="w-3.5 h-3.5" />
                            HTTPS
                          </span>
                        ) : (
                          <span className="inline-flex items-center gap-1 text-[11px] text-[var(--ink-muted)]">
                            <LockOpen className="w-3.5 h-3.5" />
                            HTTP/S
                          </span>
                        )}
                      </td>

                      {/* Actions */}
                      <td className="py-3 px-4 text-right">
                        <div className="flex items-center justify-end gap-1">
                          {/* Visit site external link */}
                          <a
                            href={`http://${site.primary_domain}`}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="p-1.5 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface)] transition-colors"
                            title="Kunjungi website"
                          >
                            <ExternalLink className="w-3.5 h-3.5" />
                          </a>

                          {/* Detail button */}
                          <button
                            onClick={() => onSelectSite(site)}
                            className="p-1.5 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--primary)] hover:bg-[var(--surface)] transition-colors"
                            title="Buka detail situs"
                          >
                            <Eye className="w-3.5 h-3.5" />
                          </button>

                          {/* Toggle Active/Inactive */}
                          <button
                            onClick={() => handleToggleStatus(site)}
                            className={`p-1.5 rounded-[var(--radius-sm)] transition-colors ${
                              isActive
                                ? 'text-[var(--ink-muted)] hover:text-[var(--accent)] hover:bg-[var(--surface)]'
                                : 'text-[var(--ink-muted)] hover:text-[var(--success)] hover:bg-[var(--surface)]'
                            }`}
                            title={isActive ? 'Nonaktifkan situs' : 'Aktifkan situs'}
                          >
                            <Power className="w-3.5 h-3.5" />
                          </button>

                          {/* Delete site */}
                          <button
                            onClick={() => setSiteToDelete(site)}
                            className="p-1.5 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--danger)] hover:bg-[var(--surface)] transition-colors"
                            title="Hapus situs"
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* Site Creation Wizard Modal */}
      <SiteWizardDialog
        isOpen={isWizardOpen}
        onClose={() => setIsWizardOpen(false)}
        onSuccess={(newSite) => {
          setToastMessage({
            text: `Situs ${newSite.primary_domain} berhasil dibuat dan vhost aktif!`,
            type: 'success',
          });
          loadSites();
        }}
      />

      {/* Danger Confirm Delete Dialog */}
      {siteToDelete && (
        <ConfirmDangerDialog
          isOpen={true}
          title={`Hapus Situs ${siteToDelete.primary_domain}`}
          description={`Tindakan ini akan menghapus vhost Nginx, sertifikat SSL, seluruh domain alias, dan konfigurasi situs ${siteToDelete.primary_domain}. Tindakan ini tidak dapat dibatalkan.`}
          confirmMatchText={siteToDelete.primary_domain}
          confirmButtonLabel="Hapus Situs Permanen"
          isLoading={isDeleting}
          onConfirm={handleConfirmDelete}
          onClose={() => setSiteToDelete(null)}
        />
      )}
    </div>
  );
};
