import { StoragePage } from './StoragePage';
import { panelWebSocketUrl } from '../lib/panelUrl';
import { ModalOverlay } from '../components/ui/ModalOverlay';
import { navigate, RouteLink, useBrowserRoute } from '../lib/router';
import { canonicalSiteUrl, sitePath } from '../lib/siteRoutes';
import React, { useState, useEffect, useCallback, useRef } from 'react';
import {
  ArrowLeft,
  Globe,
  ExternalLink,
  Power,
  Trash2,
  Lock,
  Terminal,
  Settings as SettingsIcon,
  RefreshCw,
  Plus,
  AlertCircle,
  CheckCircle2,
  Shield,
  Upload,
  Layers,
  Search,
  Play,
  Pause,
  Code2,
  Sliders,
  Folder,
  Key,
  Network,
  Clock,
  GitBranch,
  Database,
  Archive,
  BarChart3,
} from 'lucide-react';
import { FileManager } from '../components/files/FileManager';
import { PhpSettingsCard } from '../components/sites/PhpSettingsCard';
import { PhpStatusCard } from '../components/sites/PhpStatusCard';
import { NodeStatusCard } from '../components/sites/NodeStatusCard';
import { NodeSettingsCard } from '../components/sites/NodeSettingsCard';
import { PythonStatusCard } from '../components/sites/PythonStatusCard';
import { PythonSettingsCard } from '../components/sites/PythonSettingsCard';
import { ProxySettingsCard } from '../components/sites/ProxySettingsCard';
import { UnifiedProcessCard } from '../components/sites/UnifiedProcessCard';
import { SiteEnvironmentTab } from '../components/sites/SiteEnvironmentTab';
import { SiteCronTab } from '../components/sites/SiteCronTab';
import { SiteRulesTab } from '../components/sites/SiteRulesTab';
import { SiteDeployTab } from '../components/sites/SiteDeployTab';
import { SiteDatabaseTab } from '../components/sites/SiteDatabaseTab';
import { SiteBackupsTab } from '../components/sites/SiteBackupsTab';
import { SiteOverviewTab } from '../components/sites/SiteOverviewTab';
import { SiteAnalyticsTab } from '../components/sites/SiteAnalyticsTab';
import { PulseStrip } from '../components/ui/PulseStrip';
import { SshConnectModal } from '../components/ssh/SshConnectModal';
import { useAuth } from '../context/AuthContext';
import { useTranslation } from 'react-i18next';
import {
  Site,
  SiteDomain,
  SslStatus,
  AddDomainRequest,
  RedirectMode,
  IssueSslRequest,
  CustomCertRequest,
  PhpVersionStatusItem,
} from '../types/api';
import {
  fetchSite,
  enableSite,
  disableSite,
  deleteSite,
  setForceHttps,
  addSiteDomain,
  deleteSiteDomain,
  fetchSiteSsl,
  issueSiteSsl,
  generateSelfSignedSsl,
  uploadCustomSsl,
  renewSiteSsl,
  deleteSiteSsl,
  fetchSiteLogs,
  fetchPhpVersions,
  switchSitePhpVersion,
} from '../api/client';
import { StatusPill } from '../components/ui/StatusPill';
import { ConfirmDangerDialog } from '../components/ui/ConfirmDangerDialog';

export interface SiteDetailPageProps {
  siteId: number;
  onBack: () => void;
}

type TabType = 'overview' | 'analytics' | 'domains' | 'files' | 'environment' | 'storage' | 'database' | 'ssl' | 'rules' | 'deploy' | 'backups' | 'logs' | 'php' | 'node' | 'python' | 'proxy' | 'cron' | 'settings';

export const SiteDetailPage: React.FC<SiteDetailPageProps> = ({ siteId, onBack }) => {
  const { t } = useTranslation();
  const { user } = useAuth();
  const route = useBrowserRoute();
  const segment = route.pathname.split('/')[3] || 'overview';
  const [site, setSite] = useState<Site | null>(null);
  const handleServiceStatusChange = useCallback((status: string) => {
    if (status !== 'active' && status !== 'suspended') return;
    setSite(current => current && current.id === siteId && current.status !== status ? { ...current, status } : current);
  }, [siteId]);
  const setActiveTab = (tab: TabType) => {
    if (site) navigate(sitePath(site, tab));
  };
  useEffect(() => {
    if (!site) return;
    const target = canonicalSiteUrl(site, route.pathname, window.location.search, window.location.hash);
    if (target !== window.location.pathname + window.location.search + window.location.hash) navigate(target, true);
  }, [site, route.pathname]);
  const runtimeTab = site?.kind === 'nodejs' ? 'node' : site?.kind || 'php';
  const activeTab = ((segment === 'settings' && route.pathname.split('/')[4] === 'runtime') || segment === 'runtime' ? runtimeTab : segment) as TabType;
  const [sslStatus, setSslStatus] = useState<SslStatus | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isSshModalOpen, setIsSshModalOpen] = useState(false);

  // Global toast
  const [toastMessage, setToastMessage] = useState<{
    text: string;
    type: 'success' | 'error';
  } | null>(null);

  // Danger delete site state
  const [isDeleteDialogOpen, setIsDeleteDialogOpen] = useState(false);
  const [isDeletingSite, setIsDeletingSite] = useState(false);

  // Domain Management States
  const [isAddDomainModalOpen, setIsAddDomainModalOpen] = useState(false);
  const [newDomainName, setNewDomainName] = useState('');
  const [newDomainRedirectMode, setNewDomainRedirectMode] = useState<RedirectMode>('none');
  const [newDomainRedirectTarget, setNewDomainRedirectTarget] = useState('');
  const [isSubmittingDomain, setIsSubmittingDomain] = useState(false);
  const [domainToDelete, setDomainToDelete] = useState<SiteDomain | null>(null);

  // SSL Management States
  const [isIssueModalOpen, setIsIssueModalOpen] = useState(false);
  const [issueEmail, setIssueEmail] = useState('');
  const [issueStaging, setIssueStaging] = useState(false);
  const [issueFallbackSelfSigned, setIssueFallbackSelfSigned] = useState(true);
  const [isIssuingSsl, setIsIssuingSsl] = useState(false);

  const [isCustomModalOpen, setIsCustomModalOpen] = useState(false);
  const [customFullchain, setCustomFullchain] = useState('');
  const [customPrivkey, setCustomPrivkey] = useState('');
  const [customAutoRenew, setCustomAutoRenew] = useState(false);
  const [isUploadingCustom, setIsUploadingCustom] = useState(false);
  const [isRenewingSsl, setIsRenewingSsl] = useState(false);

  // PHP Version Switcher States
  const [isSwitchPhpModalOpen, setIsSwitchPhpModalOpen] = useState(false);
  const [availablePhpVersions, setAvailablePhpVersions] = useState<PhpVersionStatusItem[]>([]);
  const [selectedPhpVersion, setSelectedPhpVersion] = useState('');
  const [isLoadingPhpVersions, setIsLoadingPhpVersions] = useState(false);
  const [isSwitchingPhp, setIsSwitchingPhp] = useState(false);

  // Logs & WebSocket Live Tail States
  const [logType, setLogType] = useState<'access' | 'error' | 'php_error' | 'php_slow' | 'app'>('access');
  const [logLinesCount, setLogLinesCount] = useState<number>(100);
  const [logQueryFilter, setLogQueryFilter] = useState('');
  const [logLines, setLogLines] = useState<string[]>([]);
  const [isLiveTailActive, setIsLiveTailActive] = useState(true);
  const [isWsConnected, setIsWsConnected] = useState(false);
  const wsRef = useRef<WebSocket | null>(null);
  const logTerminalRef = useRef<HTMLDivElement | null>(null);

  // Load site and SSL details
  const loadSiteData = useCallback(async () => {
    setIsLoading(true);
    try {
      const [siteData, sslData] = await Promise.all([
        fetchSite(siteId),
        fetchSiteSsl(siteId).catch(() => null),
      ]);
      setSite(siteData);
      if (sslData) setSslStatus(sslData);
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal memuat rincian situs',
        type: 'error',
      });
    } finally {
      setIsLoading(false);
    }
  }, [siteId]);

  useEffect(() => {
    loadSiteData();
  }, [loadSiteData]);

  // Keep the rules editor mounted after saving, including its selected section and feedback.
  const refreshSiteAfterRules = useCallback(async () => {
    try {
      setSite(await fetchSite(siteId));
    } catch {
      setToastMessage({
        text: 'Aturan tersimpan, tetapi detail situs gagal diperbarui. Segarkan halaman.',
        type: 'error',
      });
    }
  }, [siteId]);

  // Handle site enable/disable
  const handleToggleSiteStatus = async () => {
    if (!site) return;
    const isCurrentlyActive = site.status === 'active';
    try {
      if (isCurrentlyActive) {
        await disableSite(site.id);
        setToastMessage({
          text: `Situs ${site.primary_domain} berhasil dinonaktifkan`,
          type: 'success',
        });
      } else {
        await enableSite(site.id);
        setToastMessage({
          text: `Situs ${site.primary_domain} berhasil diaktifkan`,
          type: 'success',
        });
      }
      loadSiteData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal mengubah status situs',
        type: 'error',
      });
    }
  };

  const handleOpenSwitchPhpModal = async () => {
    setIsSwitchPhpModalOpen(true);
    setIsLoadingPhpVersions(true);
    try {
      const res = await fetchPhpVersions();
      const installed = res.versions.filter((v) => v.installed);
      setAvailablePhpVersions(installed);
      setSelectedPhpVersion(site?.version || (installed.length > 0 ? installed[0].version : '8.3'));
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal memuat daftar versi PHP yang terpasang',
        type: 'error',
      });
    } finally {
      setIsLoadingPhpVersions(false);
    }
  };

  const handleSwitchPhpVersion = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!site || !selectedPhpVersion || selectedPhpVersion === site.version) {
      return;
    }
    setIsSwitchingPhp(true);
    try {
      const res = await switchSitePhpVersion(site.id, selectedPhpVersion);
      setToastMessage({
        text: `Versi PHP berhasil dialihkan ke PHP ${res.new_version} tanpa downtime!`,
        type: 'success',
      });
      setIsSwitchPhpModalOpen(false);
      loadSiteData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal mengalihkan versi PHP',
        type: 'error',
      });
    } finally {
      setIsSwitchingPhp(false);
    }
  };

  // Handle delete site
  const handleConfirmDeleteSite = async () => {
    if (!site) return;
    setIsDeletingSite(true);
    try {
      await deleteSite(site.id);
      onBack();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal menghapus situs',
        type: 'error',
      });
      setIsDeletingSite(false);
      setIsDeleteDialogOpen(false);
    }
  };

  // =========================================================================
  // Domain Handlers
  // =========================================================================

  const handleAddDomain = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newDomainName.trim()) return;
    setIsSubmittingDomain(true);
    try {
      const payload: AddDomainRequest = {
        domain: newDomainName.trim().toLowerCase(),
        redirect_mode: newDomainRedirectMode,
        redirect_target: newDomainRedirectTarget || undefined,
        is_primary: false,
      };
      await addSiteDomain(siteId, payload);
      setToastMessage({
        text: `Domain alias ${newDomainName} berhasil ditambahkan!`,
        type: 'success',
      });
      setIsAddDomainModalOpen(false);
      setNewDomainName('');
      setNewDomainRedirectMode('none');
      setNewDomainRedirectTarget('');
      loadSiteData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal menambahkan domain alias',
        type: 'error',
      });
    } finally {
      setIsSubmittingDomain(false);
    }
  };

  const handleDeleteDomain = async () => {
    if (!domainToDelete) return;
    try {
      await deleteSiteDomain(siteId, domainToDelete.id);
      setToastMessage({
        text: `Domain ${domainToDelete.name} berhasil dihapus`,
        type: 'success',
      });
      setDomainToDelete(null);
      loadSiteData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal menghapus domain',
        type: 'error',
      });
    }
  };

  // =========================================================================
  // SSL Handlers
  // =========================================================================

  const handleIssueLetsEncrypt = async (e: React.FormEvent) => {
    e.preventDefault();
    setIsIssuingSsl(true);
    try {
      const payload: IssueSslRequest = {
        email: issueEmail.trim() || undefined,
        staging: issueStaging,
        fallback_self_signed: issueFallbackSelfSigned,
        ocsp_stapling: true,
      };
      const res = await issueSiteSsl(siteId, payload);
      setToastMessage({
        text: res.message || 'Sertifikat Let\'s Encrypt berhasil diterbitkan!',
        type: 'success',
      });
      setIsIssueModalOpen(false);
      loadSiteData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal menerbitkan sertifikat Let\'s Encrypt',
        type: 'error',
      });
    } finally {
      setIsIssuingSsl(false);
    }
  };

  const handleGenerateSelfSigned = async () => {
    try {
      const res = await generateSelfSignedSsl(siteId);
      setToastMessage({
        text: res.message || 'Sertifikat self-signed berhasil dibuat!',
        type: 'success',
      });
      loadSiteData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal membuat sertifikat self-signed',
        type: 'error',
      });
    }
  };

  const handleUploadCustomSsl = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!customFullchain.trim() || !customPrivkey.trim()) {
      setToastMessage({
        text: 'Fullchain sertifikat dan private key wajib diisi',
        type: 'error',
      });
      return;
    }
    setIsUploadingCustom(true);
    try {
      const payload: CustomCertRequest = {
        fullchain_pem: customFullchain.trim(),
        privkey_pem: customPrivkey.trim(),
        auto_renew: customAutoRenew,
      };
      const res = await uploadCustomSsl(siteId, payload);
      setToastMessage({
        text: res.message || 'Sertifikat kustom berhasil dipasang!',
        type: 'success',
      });
      setIsCustomModalOpen(false);
      setCustomFullchain('');
      setCustomPrivkey('');
      loadSiteData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal memasang sertifikat kustom',
        type: 'error',
      });
    } finally {
      setIsUploadingCustom(false);
    }
  };

  const handleRenewSsl = async () => {
    setIsRenewingSsl(true);
    try {
      const res = await renewSiteSsl(siteId);
      setToastMessage({
        text: res.message || 'Sertifikat berhasil diperbarui!',
        type: 'success',
      });
      loadSiteData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal memperbarui sertifikat',
        type: 'error',
      });
    } finally {
      setIsRenewingSsl(false);
    }
  };

  const handleDeleteSsl = async () => {
    if (!confirm('Apakah Anda yakin ingin menghapus sertifikat SSL situs ini? Situs akan kembali menggunakan sertifikat default/self-signed.')) {
      return;
    }
    try {
      const res = await deleteSiteSsl(siteId);
      setToastMessage({
        text: res.message || 'Sertifikat SSL dihapus',
        type: 'success',
      });
      loadSiteData();
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal menghapus sertifikat SSL',
        type: 'error',
      });
    }
  };

  // =========================================================================
  // Logs & WebSocket Live Tail
  // =========================================================================

  const fetchRestLogs = useCallback(async () => {
    try {
      const res = await fetchSiteLogs(
        siteId,
        logType,
        logLinesCount,
        logQueryFilter || undefined
      );
      setLogLines(res.lines);
    } catch (err: unknown) {
      const e = err as Error;
      setToastMessage({
        text: e.message || 'Gagal memuat log server',
        type: 'error',
      });
    }
  }, [siteId, logType, logLinesCount, logQueryFilter]);

  // WebSocket Live Tail management
  useEffect(() => {
    if (activeTab !== 'logs') {
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
        setIsWsConnected(false);
      }
      return;
    }

    if (!isLiveTailActive) {
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
        setIsWsConnected(false);
      }
      fetchRestLogs();
      return;
    }

    // Connect to WebSocket endpoint

    const wsUrl = panelWebSocketUrl(`/ws/logs/${siteId}?log_type=${logType}&lines=${logLinesCount}`);

    let isMounted = true;
    const ws = new WebSocket(wsUrl);
    wsRef.current = ws;

    ws.onopen = () => {
      if (!isMounted) return;
      setIsWsConnected(true);
    };

    ws.onmessage = (event) => {
      if (!isMounted) return;
      try {
        const msg = JSON.parse(event.data);
        if (msg.event === 'initial_backlog' && Array.isArray(msg.lines)) {
          setLogLines(msg.lines);
        } else if (msg.event === 'log_line' && typeof msg.line === 'string') {
          // Check query filter if specified
          if (
            !logQueryFilter ||
            msg.line.toLowerCase().includes(logQueryFilter.toLowerCase())
          ) {
            setLogLines((prev) => [...prev.slice(-999), msg.line]);
          }
        }
      } catch {
        // Raw text line
        setLogLines((prev) => [...prev.slice(-999), event.data]);
      }
    };

    ws.onerror = () => {
      if (!isMounted) return;
      setIsWsConnected(false);
    };

    ws.onclose = () => {
      if (!isMounted) return;
      setIsWsConnected(false);
    };

    return () => {
      isMounted = false;
      if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
        ws.close();
      }
      wsRef.current = null;
    };
  }, [activeTab, siteId, logType, logLinesCount, isLiveTailActive, fetchRestLogs, logQueryFilter]);

  // Auto-scroll logs
  useEffect(() => {
    if (logTerminalRef.current) {
      logTerminalRef.current.scrollTop = logTerminalRef.current.scrollHeight;
    }
  }, [logLines]);

  if (!site && !isLoading) {
    return (
      <div className="site-route-state" role="alert">
        <h1>Detail situs tidak tersedia</h1>
        <p>{toastMessage?.text || 'Situs tidak ditemukan atau Anda tidak memiliki akses.'}</p>
        <div>
          <button onClick={onBack}>Kembali ke daftar situs</button>
          <button onClick={() => void loadSiteData()}>Coba lagi</button>
        </div>
      </div>
    );
  }
  if (isLoading || !site) {
    return (
      <div className="p-12 text-center text-xs text-[var(--ink-muted)] flex items-center justify-center gap-2">
        <RefreshCw className="w-4 h-4 animate-spin text-[var(--primary)]" />
        Memuat detail situs...
      </div>
    );
  }

  const isActive = site.status === 'active';
  const cert = sslStatus?.certificate;

  return (
    <div className="space-y-6">
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

      {/* Top Navigation & Action Header */}
      <div className="flex flex-col xl:flex-row xl:items-center justify-between gap-4 pb-4 border-b border-[var(--line)]">
        <div className="flex items-center gap-3 min-w-0">
          <button
            onClick={onBack}
            className="p-1.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] transition-colors"
            title={t('sites.detail.backToList')}
          >
            <ArrowLeft className="w-4 h-4" />
          </button>

          <div className="min-w-0">
            <div className="flex items-center flex-wrap gap-2">
              <h1 className="text-lg sm:text-xl font-semibold break-words tracking-tight text-[var(--ink)]">
                {site.primary_domain.split('.').map((label, index, labels) => (
                  <React.Fragment key={index}>{label}{index < labels.length - 1 && <>.<wbr /></>}</React.Fragment>
                ))}
              </h1>
              <StatusPill
                variant={isActive ? 'success' : 'neutral'}
                label={isActive ? 'Aktif' : 'Nonaktif'}
              />
              <div className="hidden sm:inline-flex items-center ml-2" title="Aktivitas Real-time (Permintaan)">
                <PulseStrip siteId={site.id} showTooltip={true} width={100} height={20} />
              </div>
            </div>
            <div className="text-xs text-[var(--ink-muted)] flex items-center gap-2 mt-0.5">
              <span>
                {site.kind === 'static'
                  ? 'Situs Statis'
                  : site.kind === 'php'
                  ? `PHP ${site.version || '8.3'}`
                  : site.kind}
              </span>
              {site.force_https && (
                <>
                  <span>&bull;</span>
                  <span className="text-[var(--success)] font-medium flex items-center gap-1">
                    <Lock className="w-3 h-3" /> HTTPS aktif
                  </span>
                </>
              )}
            </div>
          </div>
        </div>

        <div className="flex items-center flex-wrap gap-2">
          {/* SSH / SFTP Connection */}
          <button
            type="button"
            onClick={() => setIsSshModalOpen(true)}
            className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] transition-colors flex items-center gap-1.5"
            title="Koneksi SSH & SFTP ke folder situs"
          >
            <Key className="w-3.5 h-3.5 text-blue-500" />
            <span>SSH / SFTP</span>
          </button>

          {/* PHP Version Switcher Button */}
          {site.kind === 'php' && (
            <button
              type="button"
              onClick={handleOpenSwitchPhpModal}
              className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] transition-colors flex items-center gap-1.5"
              title="Ganti Versi PHP"
            >
              <Code2 className="w-3.5 h-3.5 text-purple-400" />
              <span>Ganti PHP</span>
            </button>
          )}

          {/* External visit site */}
          <a
            href={`${site.force_https ? 'https' : 'http'}://${site.primary_domain}`}
            target="_blank"
            rel="noopener noreferrer"
            className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] transition-colors flex items-center gap-1.5"
          >
            <ExternalLink className="w-3.5 h-3.5" />
            {t('sites.detail.visitSite')}
          </a>

          {/* Enable / Disable */}
          <button
            onClick={handleToggleSiteStatus}
            className={`px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border transition-colors flex items-center gap-1.5 ${
              isActive
                ? 'border-[var(--line)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface)]'
                : 'border-[var(--success)]/30 bg-[var(--success)]/10 text-[var(--success)] hover:bg-[var(--success)]/20'
            }`}
          >
            <Power className="w-3.5 h-3.5" />
            {isActive ? t('sites.detail.disableSite') : t('sites.detail.enableSite')}
          </button>

          {/* Delete Site */}
          <button
            onClick={() => setIsDeleteDialogOpen(true)}
            className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--danger)]/30 bg-[var(--danger)]/10 text-[var(--danger)] hover:bg-[var(--danger)]/20 transition-colors flex items-center gap-1.5"
          >
            <Trash2 className="w-3.5 h-3.5" />
            {t('sites.detail.deleteSite')}
          </button>
        </div>
      </div>

      {/* Tabs Bar */}
      <div className="flex items-center gap-1 border-b border-[var(--line)] text-xs font-medium overflow-x-auto">
        <RouteLink href={sitePath(site, 'overview')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'overview'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <Globe className="w-4 h-4" />
          {t('sites.tabs.overview')}
        </RouteLink>

        <RouteLink href={sitePath(site, 'storage')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${activeTab === 'storage' ? 'border-[var(--primary)] text-[var(--primary)] font-semibold' : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'}`}>
          <Archive className="w-4 h-4" /> Storage S3
        </RouteLink>

        <RouteLink href={sitePath(site, 'analytics')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'analytics'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <BarChart3 className="w-4 h-4 text-blue-500" />
          Analitik
        </RouteLink>

        <RouteLink href={sitePath(site, 'domains')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'domains'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <Layers className="w-4 h-4" />
          {t('sites.tabs.domains')} ({site.domains.length})
        </RouteLink>

        <RouteLink href={sitePath(site, 'files')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'files'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <Folder className="w-4 h-4" />
          Berkas
        </RouteLink>

        <RouteLink href={sitePath(site, 'environment')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'environment'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <Key className="w-4 h-4" />
          {t('sites.tabs.environment', 'Environment')}
        </RouteLink>

        <RouteLink href={sitePath(site, 'database')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'database'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <Database className="w-4 h-4" />
          {t('sites.tabs.database', 'Basis Data')}
        </RouteLink>

        <RouteLink href={sitePath(site, 'ssl')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'ssl'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <Lock className="w-4 h-4" />
          {t('sites.tabs.ssl')}
          {cert && (
            <span className="w-2 h-2 rounded-full bg-[var(--success)] inline-block" />
          )}
        </RouteLink>

        <RouteLink href={sitePath(site, 'rules')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'rules'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <Sliders className="w-4 h-4" />
          {t('sites.tabs.rules', 'Aturan')}
        </RouteLink>

        <RouteLink href={sitePath(site, 'deploy')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'deploy'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <GitBranch className="w-4 h-4" />
          {t('sites.tabs.deploy', 'Deploy')}
        </RouteLink>

        <RouteLink href={sitePath(site, 'backups')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'backups'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <Archive className="w-4 h-4" />
          {t('sites.tabs.backups', 'Cadangan')}
        </RouteLink>

        <RouteLink href={sitePath(site, 'logs')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'logs'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <Terminal className="w-4 h-4" />
          {t('sites.tabs.logs')}
        </RouteLink>

        {site.kind === 'php' && (
          <RouteLink href={sitePath(site, 'php')}
            className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
              activeTab === 'php'
                ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
            }`}
          >
            <Sliders className="w-4 h-4" />
            Pengaturan PHP
          </RouteLink>
        )}

        {(site.kind === 'nodejs' || site.runtime === 'nodejs') && (
          <RouteLink href={sitePath(site, 'node')}
            className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
              activeTab === 'node'
                ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
            }`}
          >
            <Terminal className="w-4 h-4" />
            {t('sites.node.tabTitle')}
          </RouteLink>
        )}

        {(site.kind === 'python' || site.runtime === 'python') && (
          <RouteLink href={sitePath(site, 'python')}
            className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
              activeTab === 'python'
                ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
            }`}
          >
            <Terminal className="w-4 h-4" />
            Aplikasi Python
          </RouteLink>
        )}

        {(site.kind === 'proxy' || site.runtime === 'proxy') && (
          <RouteLink href={sitePath(site, 'proxy')}
            className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
              activeTab === 'proxy'
                ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
            }`}
          >
            <Network className="w-4 h-4" />
            Reverse Proxy
          </RouteLink>
        )}

        <RouteLink href={sitePath(site, 'cron')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'cron'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <Clock className="w-4 h-4" />
          {t('sites.tabs.cron', 'Cron')}
        </RouteLink>

        <RouteLink href={sitePath(site, 'settings')}
          className={`px-4 py-2.5 border-b-2 transition-colors flex items-center gap-2 shrink-0 ${
            activeTab === 'settings'
              ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
              : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
          }`}
        >
          <SettingsIcon className="w-4 h-4" />
          {t('sites.tabs.settings')}
        </RouteLink>
      </div>

      {/* ===================================================================== */}
      {/* TAB 1: OVERVIEW (RINGKASAN FINAL)                                     */}
      {/* ===================================================================== */}
      {activeTab === 'overview' && (
        <SiteOverviewTab
          onStatusChange={handleServiceStatusChange}
          site={site}
          onNavigateTab={(tab) => setActiveTab(tab as TabType)}
          onOpenTerminal={() => setIsSshModalOpen(true)}
        />
      )}

      {/* ===================================================================== */}
      {/* TAB 1.5: ANALYTICS & PULSESTRIP (PHASE 9 - P9-T6)                     */}
      {/* ===================================================================== */}
      {activeTab === 'analytics' && (
        <SiteAnalyticsTab
          siteId={site.id}
          siteDomain={site.primary_domain}
          siteRuntime={site.kind}
        />
      )}

      {/* ===================================================================== */}
      {/* TAB 2: DOMAINS & ALIASES                                              */}
      {/* ===================================================================== */}
      {activeTab === 'storage' && <StoragePage scopeSiteId={site.id} scopeOwnerId={site.user_id} />}

      {activeTab === 'domains' && (
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <p className="text-xs text-[var(--ink-muted)]">
              Kelola domain alias, subdomain, serta pengaturan pengalihan www ↔ non-www.
            </p>
            <button
              onClick={() => setIsAddDomainModalOpen(true)}
              className="px-3.5 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] transition-colors flex items-center gap-1.5 shadow-xs"
            >
              <Plus className="w-3.5 h-3.5" />
              {t('sites.detail.addDomain')}
            </button>
          </div>

          <div className="border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--surface)] overflow-hidden shadow-xs">
            <table className="w-full text-left text-xs border-collapse">
              <thead>
                <tr className="border-b border-[var(--line)] bg-[var(--raised)] text-[var(--ink-muted)] font-semibold">
                  <th className="py-3 px-4">{t('sites.detail.aliasTable.domain')}</th>
                  <th className="py-3 px-4">{t('sites.detail.aliasTable.isPrimary')}</th>
                  <th className="py-3 px-4">{t('sites.detail.aliasTable.redirectMode')}</th>
                  <th className="py-3 px-4">{t('sites.detail.aliasTable.target')}</th>
                  <th className="py-3 px-4 text-right">{t('sites.detail.aliasTable.actions')}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--line)] text-[var(--ink)]">
                {site.domains.map((dom) => (
                  <tr key={dom.id} className="hover:bg-[var(--raised)]/50 transition-colors">
                    <td className="py-3 px-4 font-mono font-semibold">
                      {dom.name}
                      {dom.unicode_name !== dom.name && (
                        <span className="text-[11px] text-[var(--ink-muted)] block font-normal">
                          ({dom.unicode_name})
                        </span>
                      )}
                    </td>
                    <td className="py-3 px-4">
                      {dom.is_primary ? (
                        <span className="px-2 py-0.5 rounded-full bg-[var(--primary)]/10 text-[var(--primary)] font-semibold text-[10px]">
                          Utama
                        </span>
                      ) : (
                        <span className="text-[var(--ink-muted)] text-[11px]">Alias</span>
                      )}
                    </td>
                    <td className="py-3 px-4">
                      <span className="capitalize text-[11px]">
                        {dom.redirect_mode === 'none' && 'Tanpa Pengalihan'}
                        {dom.redirect_mode === 'www_to_non_www' && 'www → non-www'}
                        {dom.redirect_mode === 'non_www_to_www' && 'non-www → www'}
                        {dom.redirect_mode === 'custom_url' && 'URL Kustom'}
                      </span>
                    </td>
                    <td className="py-3 px-4 font-mono text-[11px] text-[var(--ink-muted)]">
                      {dom.redirect_target || '-'}
                    </td>
                    <td className="py-3 px-4 text-right">
                      {!dom.is_primary && (
                        <button
                          onClick={() => setDomainToDelete(dom)}
                          className="p-1 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--danger)] hover:bg-[var(--surface)] transition-colors"
                          title="Hapus alias domain"
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {/* ===================================================================== */}
      {/* TAB 3: SSL CERTIFICATES                                               */}
      {/* ===================================================================== */}
      {activeTab === 'ssl' && (
        <div className="space-y-6">
          {/* SSL Status Card */}
          <div className="p-5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] shadow-xs">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 pb-4 border-b border-[var(--line)]">
              <div className="flex items-center gap-3">
                <div
                  className={`w-9 h-9 rounded-[var(--radius-md)] flex items-center justify-center ${
                    cert
                      ? 'bg-[var(--success)]/10 text-[var(--success)]'
                      : 'bg-[var(--ink-muted)]/10 text-[var(--ink-muted)]'
                  }`}
                >
                  <Lock className="w-5 h-5" />
                </div>
                <div>
                  <h3 className="text-sm font-semibold text-[var(--ink)]">
                    {cert ? t('sites.detail.sslActive') : t('sites.detail.sslNone')}
                  </h3>
                  <p className="text-xs text-[var(--ink-muted)]">
                    {cert
                      ? `Sertifikat diterbitkan oleh ${cert.issuer}`
                      : 'Situs belum memiliki sertifikat SSL yang valid.'}
                  </p>
                </div>
              </div>

              {/* Action buttons */}
              <div className="flex items-center gap-2 flex-wrap">
                <button
                  onClick={() => setIsIssueModalOpen(true)}
                  className="px-3.5 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] transition-colors flex items-center gap-1.5 shadow-xs"
                >
                  <Shield className="w-3.5 h-3.5" />
                  {t('sites.detail.issueLetsEncrypt')}
                </button>

                <button
                  onClick={handleGenerateSelfSigned}
                  className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] transition-colors"
                >
                  {t('sites.detail.generateSelfSigned')}
                </button>

                <button
                  onClick={() => setIsCustomModalOpen(true)}
                  className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] transition-colors flex items-center gap-1.5"
                >
                  <Upload className="w-3.5 h-3.5" />
                  {t('sites.detail.uploadCustom')}
                </button>

                {cert && (
                  <>
                    <button
                      onClick={handleRenewSsl}
                      disabled={isRenewingSsl}
                      className="px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--raised)] transition-colors flex items-center gap-1.5 disabled:opacity-50"
                    >
                      <RefreshCw className={`w-3.5 h-3.5 ${isRenewingSsl ? 'animate-spin' : ''}`} />
                      {t('sites.detail.renewSsl')}
                    </button>

                    <button
                      onClick={handleDeleteSsl}
                      className="p-1.5 rounded-[var(--radius-md)] border border-[var(--danger)]/30 bg-[var(--danger)]/10 text-[var(--danger)] hover:bg-[var(--danger)]/20 transition-colors"
                      title="Hapus sertifikat SSL"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </>
                )}
              </div>
            </div>

            {/* Cert Details if exists */}
            {cert && (
              <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-4 gap-4 mt-4 pt-2 text-xs">
                <div>
                  <span className="text-[var(--ink-muted)] block">{t('sites.detail.issuer')}</span>
                  <span className="font-semibold text-[var(--ink)]">{cert.issuer}</span>
                </div>
                <div>
                  <span className="text-[var(--ink-muted)] block">{t('sites.detail.daysRemaining')}</span>
                  <span
                    className={`font-semibold ${
                      cert.days_remaining < 15 ? 'text-[var(--danger)]' : 'text-[var(--success)]'
                    }`}
                  >
                    {cert.days_remaining} hari
                  </span>
                </div>
                <div>
                  <span className="text-[var(--ink-muted)] block">{t('sites.detail.validUntil')}</span>
                  <span className="text-[var(--ink)] font-mono text-[11px]">
                    {new Date(cert.not_after * 1000).toLocaleDateString('id-ID')}
                  </span>
                </div>
                <div>
                  <span className="text-[var(--ink-muted)] block">{t('sites.detail.autoRenewStatus')}</span>
                  <span className="font-semibold text-[var(--ink)]">
                    {cert.auto_renew ? 'Aktif (30 hari sebelum kedaluwarsa)' : 'Nonaktif'}
                  </span>
                </div>
              </div>
            )}
          </div>
        </div>
      )}

      {/* ===================================================================== */}
      {/* TAB 2.5: FILE MANAGER (BERKAS SITUS)                                  */}
      {/* ===================================================================== */}
      {activeTab === 'files' && (
        <div className="pt-2">
          <FileManager
            tenant={String(site.user_id)}
            siteId={site.id}
            initialPath={site.docroot.replace(/^\/home\/[^/]+(?:\/|$)/, '')}
            title={`Berkas Situs: ${site.primary_domain}`}
          />
        </div>
      )}

      {/* ===================================================================== */}
      {/* TAB: ENVIRONMENT VARIABLES (TERENKRIPSI)                              */}
      {/* ===================================================================== */}
      {activeTab === 'environment' && (
        <div className="pt-2">
          <SiteEnvironmentTab siteId={site.id} onSiteRestarted={loadSiteData} />
        </div>
      )}

      {/* ===================================================================== */}
      {/* TAB 4: SERVER LOGS & WEBSOCKET LIVE TAIL                              */}
      {/* ===================================================================== */}
      {activeTab === 'logs' && (
        <div className="space-y-4">
          {/* Log Controls Header */}
          <div className="flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3 p-3 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)]">
            {/* Toggle Log Type & Live Tail */}
            <div className="flex items-center gap-3">
              <div className="inline-flex rounded-[var(--radius-md)] border border-[var(--line)] p-0.5 bg-[var(--raised)] text-xs">
                <button
                  onClick={() => setLogType('access')}
                  className={`px-3 py-1 rounded-[var(--radius-sm)] transition-colors ${
                    logType === 'access'
                      ? 'bg-[var(--primary)] text-white font-semibold'
                      : 'text-[var(--ink-muted)] hover:text-[var(--ink)]'
                  }`}
                >
                  Nginx Access
                </button>
                <button
                  onClick={() => setLogType('error')}
                  className={`px-3 py-1 rounded-[var(--radius-sm)] transition-colors ${
                    logType === 'error'
                      ? 'bg-[var(--primary)] text-white font-semibold'
                      : 'text-[var(--ink-muted)] hover:text-[var(--ink)]'
                  }`}
                >
                  Nginx Error
                </button>
                {site.kind === 'php' && (
                  <>
                    <button
                      onClick={() => setLogType('php_error')}
                      className={`px-3 py-1 rounded-[var(--radius-sm)] transition-colors ${
                        logType === 'php_error'
                          ? 'bg-[var(--primary)] text-white font-semibold'
                          : 'text-[var(--ink-muted)] hover:text-[var(--ink)]'
                      }`}
                    >
                      PHP Error
                    </button>
                    <button
                      onClick={() => setLogType('php_slow')}
                      className={`px-3 py-1 rounded-[var(--radius-sm)] transition-colors ${
                        logType === 'php_slow'
                          ? 'bg-[var(--primary)] text-white font-semibold'
                          : 'text-[var(--ink-muted)] hover:text-[var(--ink)]'
                      }`}
                    >
                      PHP Slow
                    </button>
                  </>
                )}
                {(site.kind === 'nodejs' ||
                  site.kind === 'python' ||
                  site.runtime === 'nodejs' ||
                  site.runtime === 'python') && (
                  <button
                    onClick={() => setLogType('app')}
                    className={`px-3 py-1 rounded-[var(--radius-sm)] transition-colors ${
                      logType === 'app'
                        ? 'bg-[var(--primary)] text-white font-semibold'
                        : 'text-[var(--ink-muted)] hover:text-[var(--ink)]'
                    }`}
                  >
                    Aplikasi (Stdout/Stderr)
                  </button>
                )}
              </div>

              {/* Live Tail Toggle */}
              <button
                onClick={() => setIsLiveTailActive((prev) => !prev)}
                className={`px-3 py-1 text-xs font-semibold rounded-[var(--radius-md)] border flex items-center gap-1.5 transition-colors ${
                  isLiveTailActive
                    ? 'border-[var(--success)]/30 bg-[var(--success)]/10 text-[var(--success)]'
                    : 'border-[var(--line)] bg-[var(--raised)] text-[var(--ink-muted)]'
                }`}
              >
                {isLiveTailActive ? (
                  <>
                    <Pause className="w-3.5 h-3.5" />
                    {t('sites.detail.pauseStream')}
                  </>
                ) : (
                  <>
                    <Play className="w-3.5 h-3.5" />
                    {t('sites.detail.resumeStream')}
                  </>
                )}
              </button>

              {/* Status Indicator */}
              <div className="flex items-center gap-1.5 text-xs text-[var(--ink-muted)]">
                <span
                  className={`w-2 h-2 rounded-full ${
                    isWsConnected
                      ? 'bg-[var(--success)] animate-pulse'
                      : 'bg-[var(--line)]'
                  }`}
                />
                <span className="text-[11px]">
                  {isWsConnected
                    ? t('sites.detail.liveLogConnected')
                    : t('sites.detail.liveLogDisconnected')}
                </span>
              </div>
            </div>

            {/* Filter Query & Lines Dropdown */}
            <div className="flex items-center gap-2">
              <div className="relative">
                <Search className="w-3.5 h-3.5 absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--ink-muted)]" />
                <input
                  type="text"
                  placeholder={t('sites.detail.filterQueryPlaceholder')}
                  value={logQueryFilter}
                  onChange={(e) => setLogQueryFilter(e.target.value)}
                  className="pl-8 pr-2.5 py-1 text-xs rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] w-48"
                />
              </div>

              <select
                value={logLinesCount}
                onChange={(e) => setLogLinesCount(Number(e.target.value))}
                className="py-1 px-2 text-xs rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
              >
                <option value={50}>50 baris</option>
                <option value={100}>100 baris</option>
                <option value={250}>250 baris</option>
                <option value={500}>500 baris</option>
              </select>

              <button
                onClick={() => setLogLines([])}
                className="px-2.5 py-1 text-xs rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink-muted)] hover:text-[var(--ink)] transition-colors"
                title={t('sites.detail.clearLogs')}
              >
                {t('sites.detail.clearLogs')}
              </button>
            </div>
          </div>

          {/* Log Viewer Terminal */}
          <div
            ref={logTerminalRef}
            className="h-[460px] overflow-y-auto p-4 rounded-[var(--radius-md)] bg-[#0A141A] text-[#D8E6EA] font-mono text-[11px] leading-relaxed border border-[var(--line)] shadow-inner"
          >
            {logLines.length === 0 ? (
              <div className="h-full flex items-center justify-center text-[var(--ink-muted)]">
                {t('sites.detail.noLogs')}
              </div>
            ) : (
              <div className="space-y-1">
                {logLines.map((line, idx) => {
                  let formatted = line;
                  let statusCode: number | null = null;
                  try {
                    // Try parsing JSON format panel_json
                    const parsed = JSON.parse(line);
                    statusCode = parsed.s;
                    formatted = `${parsed.t} [${parsed.m}] ${parsed.u} → ${parsed.s} (${parsed.rt}s, ${parsed.b} bytes) - IP: ${parsed.ip}`;
                  } catch {
                    // Raw string
                  }

                  let statusColor = 'text-inherit';
                  if (statusCode) {
                    if (statusCode >= 200 && statusCode < 300) statusColor = 'text-[var(--success)]';
                    else if (statusCode >= 300 && statusCode < 400) statusColor = 'text-[var(--info)]';
                    else if (statusCode >= 400 && statusCode < 500) statusColor = 'text-[var(--accent)]';
                    else if (statusCode >= 500) statusColor = 'text-[var(--danger)] font-bold';
                  }

                  return (
                    <div key={idx} className="hover:bg-white/5 py-0.5 px-1 rounded break-all">
                      <span className="text-gray-500 select-none mr-2 font-mono text-[10px]">
                        {idx + 1}
                      </span>
                      <span className={statusColor}>{formatted}</span>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      )}

      {/* ===================================================================== */}
      {/* TAB 5: PHP CONFIGURATION                                              */}
      {/* ===================================================================== */}
      {activeTab === 'php' && site.kind === 'php' && (
        <div className="space-y-6">
          <PhpStatusCard siteId={site.id} />
          <PhpSettingsCard
            siteId={site.id}
            phpVersion={site.version || '8.3'}
            onSaved={() => {
              loadSiteData();
            }}
          />
        </div>
      )}

      {/* ===================================================================== */}
      {/* TAB 5b: NODE.JS PROCESS & CONFIGURATION                               */}
      {/* ===================================================================== */}
      {activeTab === 'node' && (site.kind === 'nodejs' || site.runtime === 'nodejs') && (
        <div className="space-y-6">
          <UnifiedProcessCard
            siteId={site.id}
            runtime="nodejs"
            domain={site.primary_domain}
            onRefresh={loadSiteData}
          />
          <NodeStatusCard siteId={site.id} />
          <NodeSettingsCard
            siteId={site.id}
            onSaved={() => {
              loadSiteData();
            }}
          />
        </div>
      )}

      {/* ===================================================================== */}
      {/* TAB 5c: PYTHON PROCESS & CONFIGURATION                                */}
      {/* ===================================================================== */}
      {activeTab === 'python' && (site.kind === 'python' || site.runtime === 'python') && (
        <div className="space-y-6">
          <UnifiedProcessCard
            siteId={site.id}
            runtime="python"
            domain={site.primary_domain}
            onRefresh={loadSiteData}
          />
          <PythonStatusCard siteId={site.id} />
          <PythonSettingsCard
            siteId={site.id}
            onSaved={() => {
              loadSiteData();
            }}
          />
        </div>
      )}

      {/* ===================================================================== */}
      {/* TAB 5d: REVERSE PROXY CONFIGURATION                                   */}
      {/* ===================================================================== */}
      {activeTab === 'proxy' && (site.kind === 'proxy' || site.runtime === 'proxy') && (
        <div className="space-y-6">
          <ProxySettingsCard
            siteId={site.id}
            onSaved={() => {
              loadSiteData();
            }}
          />
        </div>
      )}

      {/* ===================================================================== */}
      {/* TAB 5e: CRON JOBS (SYSTEMD TIMERS)                                    */}
      {/* ===================================================================== */}
      {activeTab === 'cron' && (
        <SiteCronTab siteId={site.id} />
      )}

      {/* ===================================================================== */}
      {/* TAB 5f: ATURAN SITUS & KEAMANAN (P8-T3)                               */}
      {/* ===================================================================== */}
      {activeTab === 'rules' && (
        <SiteRulesTab
          site={site}
          userRole={user?.role}
          onSiteUpdated={refreshSiteAfterRules}
        />
      )}

      {/* ===================================================================== */}
      {/* TAB 5g: GIT DEPLOY & WEBHOOK (P8-T4)                                  */}
      {/* ===================================================================== */}
      {activeTab === 'deploy' && (
        <SiteDeployTab
          siteId={site.id}
          onSiteUpdated={loadSiteData}
        />
      )}

      {/* ===================================================================== */}
      {/* TAB 5h: DATABASE PROVISIONER (P8-T5)                                  */}
      {/* ===================================================================== */}
      {activeTab === 'database' && (
        <SiteDatabaseTab
          siteId={site.id}
          siteDomain={site.primary_domain}
        />
      )}

      {/* ===================================================================== */}
      {/* TAB 5i: BACKUPS & SNAPSHOT RESTORE (P8-T6)                            */}
      {/* ===================================================================== */}
      {activeTab === 'backups' && (
        <SiteBackupsTab
          siteId={site.id}
          siteDomain={site.primary_domain}
        />
      )}

      {/* ===================================================================== */}
      {/* TAB 6: SETTINGS & DANGER ZONE                                         */}
      {/* ===================================================================== */}
      {activeTab === 'settings' && (
        <div className="space-y-6">
          {/* Routing Preferences Card */}
          <div className="border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--surface)] p-5 shadow-xs">
            <h3 className="text-sm font-semibold text-[var(--ink)] mb-3">
              Preferensi Konfigurasi Vhost
            </h3>
            <div className="space-y-4">
              <label className="flex items-center justify-between p-3 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer">
                <div>
                  <span className="text-xs font-semibold text-[var(--ink)] block">
                    Paksa Enkripsi HTTPS (Force HTTPS)
                  </span>
                  <span className="text-[11px] text-[var(--ink-muted)] block mt-0.5">
                    Alihkan otomatis seluruh koneksi HTTP port 80 ke HTTPS port 443 dengan kode status 301.
                  </span>
                </div>
                <input
                  type="checkbox"
                  checked={site.force_https}
                  onChange={async (e) => {
                    try {
                      await setForceHttps(site.id, e.target.checked);
                      setToastMessage({
                        text: `Preferensi Force HTTPS berhasil ${e.target.checked ? 'diaktifkan' : 'dinonaktifkan'}`,
                        type: 'success',
                      });
                      loadSiteData();
                    } catch (err: unknown) {
                      const errObj = err as Error;
                      setToastMessage({
                        text: errObj.message || 'Gagal mengubah preferensi Force HTTPS',
                        type: 'error',
                      });
                    }
                  }}
                  className="rounded-[var(--radius-sm)] border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)] w-4 h-4"
                />
              </label>

              <div className="p-3 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] flex items-center justify-between">
                <div>
                  <span className="text-xs font-semibold text-[var(--ink)] block">
                    Fallback Single Page Application (SPA)
                  </span>
                  <span className="text-[11px] text-[var(--ink-muted)] block mt-0.5">
                    Status saat ini: {site.spa_fallback ? 'Aktif' : 'Nonaktif'}.
                  </span>
                </div>
                <span className="text-xs px-2.5 py-1 rounded bg-[var(--bg)] font-mono">
                  {site.spa_fallback ? 'try_files $uri /index.html' : 'try_files $uri =404'}
                </span>
              </div>
            </div>
          </div>

          {/* Danger Zone */}
          <div className="border border-[var(--danger)]/30 rounded-[var(--radius-md)] bg-[var(--danger)]/5 p-5">
            <h3 className="text-sm font-semibold text-[var(--danger)] mb-1">
              Zona Bahaya
            </h3>
            <p className="text-xs text-[var(--ink-muted)] mb-4">
              Penghapusan situs akan membatalkan vhost Nginx dan menghapus seluruh sertifikat serta konfigurasi terkait.
            </p>
            <button
              onClick={() => setIsDeleteDialogOpen(true)}
              className="px-4 py-2 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--danger)] text-white hover:opacity-90 transition-opacity flex items-center gap-1.5 shadow-xs"
            >
              <Trash2 className="w-3.5 h-3.5" />
              Hapus Situs Ini
            </button>
          </div>
        </div>
      )}

      {/* ===================================================================== */}
      {/* MODALS                                                                */}
      {/* ===================================================================== */}

      {/* Add Domain Modal */}
      {isAddDomainModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs animate-in fade-in duration-150"
          onDismiss={() => setIsAddDomainModalOpen(false)}
        >
          <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] shadow-[var(--shadow-floating)] w-full max-w-md overflow-hidden dialog-panel">
            <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--line)] bg-[var(--raised)] dialog-header">
              <h3 className="text-sm font-semibold text-[var(--ink)]">{t('sites.detail.addDomain')}</h3>
              <button
                onClick={() => setIsAddDomainModalOpen(false)}
                className="text-[var(--ink-muted)] hover:text-[var(--ink)]"
              >
                &times;
              </button>
            </div>
            <form onSubmit={handleAddDomain} className="p-6 dialog-form">
              <div className="dialog-body space-y-4">
                <div>
                  <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                    Nama Domain Alias <span className="text-[var(--danger)]">*</span>
                  </label>
                  <input
                    type="text"
                    required
                    placeholder="alias.domain.com atau www.situs.com"
                    value={newDomainName}
                    onChange={(e) => setNewDomainName(e.target.value.toLowerCase())}
                    className="w-full px-3 py-1.5 text-xs font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                  />
                </div>

                <div>
                  <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                    Mode Pengalihan (Redirect)
                  </label>
                  <select
                    value={newDomainRedirectMode}
                    onChange={(e) => setNewDomainRedirectMode(e.target.value as RedirectMode)}
                    className="w-full px-3 py-1.5 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                  >
                    <option value="none">Tanpa Pengalihan (Melayani konten langsung)</option>
                    <option value="www_to_non_www">Alihkan www ke non-www</option>
                    <option value="non_www_to_www">Alihkan non-www ke www</option>
                    <option value="custom_url">Alihkan ke URL Kustom</option>
                  </select>
                </div>

                {newDomainRedirectMode === 'custom_url' && (
                  <div>
                    <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                      Target URL Pengalihan <span className="text-[var(--danger)]">*</span>
                    </label>
                    <input
                      type="url"
                      required
                      placeholder="https://tujuan-pengalihan.com"
                      value={newDomainRedirectTarget}
                      onChange={(e) => setNewDomainRedirectTarget(e.target.value)}
                      className="w-full px-3 py-1.5 text-xs font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                    />
                  </div>
                )}
              </div>
              <div className="flex items-center justify-end gap-2 pt-3 border-t border-[var(--line)] dialog-footer">
                <button
                  type="button"
                  onClick={() => setIsAddDomainModalOpen(false)}
                  className="px-3.5 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface)] transition-colors"
                >
                  Batal
                </button>
                <button
                  type="submit"
                  disabled={isSubmittingDomain}
                  className="px-4 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] disabled:opacity-50 transition-colors"
                >
                  {isSubmittingDomain ? 'Menerapkan...' : 'Tambah Domain'}
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* Issue Let's Encrypt Modal */}
      {isIssueModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs animate-in fade-in duration-150"
          onDismiss={() => setIsIssueModalOpen(false)}
        >
          <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] shadow-[var(--shadow-floating)] w-full max-w-md overflow-hidden dialog-panel">
            <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--line)] bg-[var(--raised)] dialog-header">
              <h3 className="text-sm font-semibold text-[var(--ink)]">{t('sites.detail.issueLetsEncrypt')}</h3>
              <button
                onClick={() => setIsIssueModalOpen(false)}
                className="text-[var(--ink-muted)] hover:text-[var(--ink)]"
              >
                &times;
              </button>
            </div>
            <form onSubmit={handleIssueLetsEncrypt} className="p-6 dialog-form">
              <div className="dialog-body space-y-4">
                <div>
                  <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                    Email Kontak Let's Encrypt (Opsional)
                  </label>
                  <input
                    type="email"
                    placeholder="admin@perusahaan.com"
                    value={issueEmail}
                    onChange={(e) => setIssueEmail(e.target.value)}
                    className="w-full px-3 py-1.5 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                  />
                </div>

                <label className="flex items-start gap-2.5 p-3 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer">
                  <input
                    type="checkbox"
                    checked={issueStaging}
                    onChange={(e) => setIssueStaging(e.target.checked)}
                    className="mt-0.5 rounded-[var(--radius-sm)] border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
                  />
                  <div>
                    <span className="text-xs font-semibold text-[var(--ink)] block">
                      Gunakan Let's Encrypt Staging
                    </span>
                    <span className="text-[11px] text-[var(--ink-muted)] block mt-0.5">
                      Uji coba tantangan HTTP-01 tanpa menghabiskan kuota rate limit produksi.
                    </span>
                  </div>
                </label>

                <label className="flex items-start gap-2.5 p-3 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] cursor-pointer">
                  <input
                    type="checkbox"
                    checked={issueFallbackSelfSigned}
                    onChange={(e) => setIssueFallbackSelfSigned(e.target.checked)}
                    className="mt-0.5 rounded-[var(--radius-sm)] border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
                  />
                  <div>
                    <span className="text-xs font-semibold text-[var(--ink)] block">
                      Fallback Self-Signed Otomatis
                    </span>
                    <span className="text-[11px] text-[var(--ink-muted)] block mt-0.5">
                      Bila validasi DNS gagal, buat sertifikat self-signed agar HTTPS tetap dapat dibuka.
                    </span>
                  </div>
                </label>
              </div>
              <div className="flex items-center justify-end gap-2 pt-3 border-t border-[var(--line)] dialog-footer">
                <button
                  type="button"
                  onClick={() => setIsIssueModalOpen(false)}
                  className="px-3.5 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface)] transition-colors"
                >
                  Batal
                </button>
                <button
                  type="submit"
                  disabled={isIssuingSsl}
                  className="px-4 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] disabled:opacity-50 transition-colors flex items-center gap-1.5"
                >
                  {isIssuingSsl && <RefreshCw className="w-3.5 h-3.5 animate-spin" />}
                  {isIssuingSsl ? 'Menerbitkan...' : 'Terbitkan Sertifikat'}
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* Upload Custom SSL Modal */}
      {isCustomModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs animate-in fade-in duration-150"
          onDismiss={() => setIsCustomModalOpen(false)}
        >
          <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] shadow-[var(--shadow-floating)] w-full max-w-lg overflow-hidden dialog-panel">
            <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--line)] bg-[var(--raised)] dialog-header">
              <h3 className="text-sm font-semibold text-[var(--ink)]">{t('sites.detail.uploadCustom')}</h3>
              <button
                onClick={() => setIsCustomModalOpen(false)}
                className="text-[var(--ink-muted)] hover:text-[var(--ink)]"
              >
                &times;
              </button>
            </div>
            <form onSubmit={handleUploadCustomSsl} className="p-6 dialog-form">
              <div className="dialog-body space-y-4">
                <div>
                  <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                    Certificate Chain (PEM) <span className="text-[var(--danger)]">*</span>
                  </label>
                  <textarea
                    required
                    rows={4}
                    placeholder="-----BEGIN CERTIFICATE-----&#10;...&#10;-----END CERTIFICATE-----"
                    value={customFullchain}
                    onChange={(e) => setCustomFullchain(e.target.value)}
                    className="w-full p-2.5 text-[11px] font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                  />
                </div>

                <div>
                  <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                    Private Key (PEM) <span className="text-[var(--danger)]">*</span>
                  </label>
                  <textarea
                    required
                    rows={4}
                    placeholder="-----BEGIN PRIVATE KEY-----&#10;...&#10;-----END PRIVATE KEY-----"
                    value={customPrivkey}
                    onChange={(e) => setCustomPrivkey(e.target.value)}
                    className="w-full p-2.5 text-[11px] font-mono rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                  />
                </div>

                <label className="flex items-center gap-2 text-xs text-[var(--ink)] cursor-pointer">
                  <input
                    type="checkbox"
                    checked={customAutoRenew}
                    onChange={(e) => setCustomAutoRenew(e.target.checked)}
                    className="rounded border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
                  />
                  <span>Tandai untuk pembaruan otomatis bila memungkinkan</span>
                </label>
              </div>
              <div className="flex items-center justify-end gap-2 pt-3 border-t border-[var(--line)] dialog-footer">
                <button
                  type="button"
                  onClick={() => setIsCustomModalOpen(false)}
                  className="px-3.5 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface)] transition-colors"
                >
                  Batal
                </button>
                <button
                  type="submit"
                  disabled={isUploadingCustom}
                  className="px-4 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] disabled:opacity-50 transition-colors flex items-center gap-1.5"
                >
                  {isUploadingCustom && <RefreshCw className="w-3.5 h-3.5 animate-spin" />}
                  {isUploadingCustom ? 'Memvalidasi...' : 'Pasang Sertifikat'}
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* Confirm Delete Site Dialog */}
      {isDeleteDialogOpen && (
        <ConfirmDangerDialog
          isOpen={true}
          title={`Hapus Situs ${site.primary_domain}`}
          description={`Tindakan ini akan menghapus vhost Nginx, sertifikat SSL, seluruh domain alias, dan konfigurasi situs ${site.primary_domain}. Tindakan ini tidak dapat dibatalkan.`}
          confirmMatchText={site.primary_domain}
          confirmButtonLabel="Hapus Situs Permanen"
          isLoading={isDeletingSite}
          onConfirm={handleConfirmDeleteSite}
          onClose={() => setIsDeleteDialogOpen(false)}
        />
      )}

      {/* Confirm Delete Domain Dialog */}
      {domainToDelete && (
        <ConfirmDangerDialog
          isOpen={true}
          title={`Hapus Domain Alias ${domainToDelete.name}`}
          description={`Domain ${domainToDelete.name} tidak akan lagi mengarah ke situs ini.`}
          confirmMatchText={domainToDelete.name}
          confirmButtonLabel="Hapus Domain Alias"
          onConfirm={handleDeleteDomain}
          onClose={() => setDomainToDelete(null)}
        />
      )}

      {/* Modal Ganti Versi PHP */}
      {isSwitchPhpModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-xs"
          onDismiss={() => setIsSwitchPhpModalOpen(false)}
        >
          <div className="w-full max-w-md rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-6 shadow-xl space-y-4 dialog-panel">
            <div className="flex items-center justify-between pb-3 border-b border-[var(--line)] dialog-header">
              <h3 className="font-bold text-sm text-[var(--ink)] flex items-center gap-2">
                <Code2 className="w-4 h-4 text-[var(--primary)]" />
                Ganti Versi PHP Situs
              </h3>
              <button
                onClick={() => setIsSwitchPhpModalOpen(false)}
                className="text-xs text-[var(--ink-muted)] hover:text-[var(--ink)] p-1"
              >
                &times;
              </button>
            </div>
            <form onSubmit={handleSwitchPhpVersion} className="dialog-form">
              <div className="dialog-body space-y-4">
                <div className="text-xs text-[var(--ink-muted)] leading-relaxed space-y-2">
                  <p>
                    Versi saat ini:{' '}
                    <strong className="text-[var(--ink)] font-mono">PHP {site.version || '8.3'}</strong>
                  </p>
                  <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--primary)]/10 border border-[var(--primary)]/20 text-[var(--ink)] text-[11px] leading-relaxed">
                    <strong>Zero-Downtime & Automatic Rollback:</strong> Master pool target disiapkan dan diverifikasi
                    kesehatannya terlebih dahulu sebelum socket Nginx dialihkan. Jika pemeriksaan gagal, konfigurasi
                    otomatis dibatalkan tanpa downtime.
                  </div>
                </div>

                <div>
                  <label className="block text-xs font-semibold text-[var(--ink)] mb-1.5">Pilih Versi PHP Baru</label>
                  {isLoadingPhpVersions ? (
                    <div className="flex items-center gap-2 text-xs text-[var(--ink-muted)] py-2">
                      <RefreshCw className="w-3.5 h-3.5 animate-spin" />
                      Memuat versi PHP yang terpasang...
                    </div>
                  ) : availablePhpVersions.length === 0 ? (
                    <div className="text-xs text-[var(--ink-muted)] py-2">
                      Tidak ada versi PHP terpasang lain yang tersedia.
                    </div>
                  ) : (
                    <select
                      value={selectedPhpVersion}
                      onChange={(e) => setSelectedPhpVersion(e.target.value)}
                      className="w-full px-3 py-2 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)] font-mono"
                    >
                      {availablePhpVersions.map((v) => (
                        <option key={v.version} value={v.version}>
                          PHP {v.version} {v.version === site.version ? '(Saat Ini)' : ''}
                        </option>
                      ))}
                    </select>
                  )}
                </div>
              </div>
              <div className="flex items-center justify-end gap-2 pt-3 border-t border-[var(--line)] dialog-footer">
                <button
                  type="button"
                  onClick={() => setIsSwitchPhpModalOpen(false)}
                  className="px-3.5 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--raised)] transition-colors"
                >
                  Batal
                </button>
                <button
                  type="submit"
                  disabled={
                    isSwitchingPhp ||
                    !selectedPhpVersion ||
                    selectedPhpVersion === site.version ||
                    availablePhpVersions.length === 0
                  }
                  className="px-4 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-[var(--primary-foreground)] hover:bg-[var(--primary-hover)] disabled:opacity-50 transition-colors flex items-center gap-1.5"
                >
                  {isSwitchingPhp && <RefreshCw className="w-3.5 h-3.5 animate-spin" />}
                  {isSwitchingPhp ? 'Mengalihkan Versi...' : 'Ganti Versi'}
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* SSH & SFTP Connection Modal */}
      {site && (
        <SshConnectModal
          isOpen={isSshModalOpen}
          onClose={() => setIsSshModalOpen(false)}
          siteId={site.id}
          siteDomain={site.primary_domain}
          path={site.docroot}
        />
      )}
    </div>
  );
};
