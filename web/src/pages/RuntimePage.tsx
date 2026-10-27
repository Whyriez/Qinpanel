import React, { useEffect, useState, useCallback, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Cpu,
  RefreshCw,
  ShieldCheck,
  CheckCircle2,
  AlertCircle,
  Download,
  Trash2,
  Loader2,
  Terminal,
  ExternalLink,
  Puzzle,
  Boxes,
  Package,
  Layers,
  Search,
  AlertTriangle,
  Activity,
  HardDrive,
  Database,
  Settings,
} from 'lucide-react';
import {
  PhpRuntimeListApiResponse,
  PhpVersionStatusItem,
  NodeRuntimeListApiResponse,
  NodeVersionItem,
  NodeRuntimeInstallRequest,
  ProcessListResponse,
  JobItem,
} from '../types/api';
import {
  fetchPhpVersions,
  setupPhpRepo,
  installPhpVersion,
  removePhpVersion,
  fetchNodeVersions,
  installNodeVersion,
  removeNodeVersion,
  enableNodeCorepack,
  fetchProcesses,
} from '../api/client';
import { ConfirmDangerDialog } from '../components/ui/ConfirmDangerDialog';
import { PhpExtensionsModal } from '../components/runtime/PhpExtensionsModal';
import { NodeInstallSettings, defaultNodeInstallOptions, nodeInstallPayload } from '../components/runtime/NodeInstallSettings';
import { runtimeJobVersion, useRuntimeJobs } from '../components/runtime/useRuntimeJobs';
import { RuntimeJobProgress } from '../components/runtime/RuntimeJobProgress';
import { UnifiedProcessCard } from '../components/sites/UnifiedProcessCard';
import { DatabaseServersTab } from '../components/runtime/DatabaseServersTab';
import { PythonRuntimesTab } from '../components/runtime/PythonRuntimesTab';
import { CatalogScheduleCard } from '../components/runtime/CatalogScheduleCard';
import { toast } from 'sonner';
import { navigate, useBrowserRoute } from '../lib/router';

type RuntimeTab = 'php' | 'node' | 'python' | 'settings' | 'processes' | 'databases';
const mainRuntimeNames = new Set(['php.install', 'php.remove', 'php.repo.setup', 'node.install']);

export const RuntimePage: React.FC = () => {
  const { t } = useTranslation();
  const route = useBrowserRoute();
  const activeTab = (['php', 'node', 'python', 'settings', 'processes', 'databases'] as const).find(tab => tab === route.search.get('tab')) ?? 'php';
  const setActiveTab = (tab: RuntimeTab) => navigate(`/runtime?tab=${tab}`);
  const [pythonRevision, setPythonRevision] = useState(0);
  const [catalogRevision, setCatalogRevision] = useState(0);
  const [databaseRevision, setDatabaseRevision] = useState(0);

  // =========================================================
  // PHP State
  // =========================================================
  const [phpData, setPhpData] = useState<PhpRuntimeListApiResponse | null>(null);
  const [phpLoading, setPhpLoading] = useState(true);
  const [phpError, setPhpError] = useState<string | null>(null);
  const [settingUpRepo, setSettingUpRepo] = useState(false);
  const [processingPhpVersion, setProcessingPhpVersion] = useState<string | null>(null);
  const [phpVersionToRemove, setPhpVersionToRemove] = useState<string | null>(null);
  const [versionForExtensions, setVersionForExtensions] = useState<string | null>(null);

  // =========================================================
  // Node.js State
  // =========================================================
  const [nodeData, setNodeData] = useState<NodeRuntimeListApiResponse | null>(null);
  const [nodeLoading, setNodeLoading] = useState(true);
  const [nodeError, setNodeError] = useState<string | null>(null);
  const [nodeSearch, setNodeSearch] = useState('');
  const [processingNodeVersion, setProcessingNodeVersion] = useState<string | null>(null);
  const [nodeVersionToRemove, setNodeVersionToRemove] = useState<NodeVersionItem | null>(null);
  const [nodeInstallOptions, setNodeInstallOptions] = useState(defaultNodeInstallOptions);
  const nodeSubmitting = useRef(false);

  // =========================================================
  // Process Monitoring State (Phase 7 - P7-T5)
  // =========================================================
  const [processData, setProcessData] = useState<ProcessListResponse | null>(null);
  const [processLoading, setProcessLoading] = useState(true);
  const [processError, setProcessError] = useState<string | null>(null);
  const [processSearch, setProcessSearch] = useState('');
  const [processRuntimeFilter, setProcessRuntimeFilter] = useState<'all' | 'nodejs' | 'python'>('all');
  const [processStatusFilter, setProcessStatusFilter] = useState<'all' | 'active' | 'inactive'>('all');

  // =========================================================
  // Data Fetchers
  // =========================================================
  const loadPhpData = useCallback(async (quiet = false) => {
    if (!quiet) setPhpLoading(true);
    setPhpError(null);
    try {
      const res = await fetchPhpVersions();
      setPhpData(res);
      return res;
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memuat status runtime PHP.';
      setPhpError(msg);
      if (!quiet) {
        toast.error('Gagal memuat status runtime PHP', { description: msg });
      }
      return null;
    } finally {
      if (!quiet) setPhpLoading(false);
    }
  }, []);

  const loadNodeData = useCallback(async (quiet = false) => {
    if (!quiet) setNodeLoading(true);
    setNodeError(null);
    try {
      const res = await fetchNodeVersions();
      setNodeData(res);
      return res;
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memuat daftar runtime Node.js.';
      setNodeError(msg);
      if (!quiet) {
        toast.error('Gagal memuat runtime Node.js', { description: msg });
      }
      return null;
    } finally {
      if (!quiet) setNodeLoading(false);
    }
  }, []);

  const loadProcessData = useCallback(async (quiet = false) => {
    if (!quiet) setProcessLoading(true);
    setProcessError(null);
    try {
      const res = await fetchProcesses();
      setProcessData(res);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memuat daftar proses aplikasi.';
      setProcessError(msg);
      if (!quiet) {
        toast.error('Gagal memuat proses aplikasi', { description: msg });
      }
    } finally {
      if (!quiet) setProcessLoading(false);
    }
  }, []);

  const onRuntimeFinished = useCallback(async (job: JobItem) => {
    const version = runtimeJobVersion(job);
    if (job.name.startsWith('php.')) {
      const inventory = await loadPhpData(true);
      if (!inventory) return false;
      if (job.status !== 'succeeded') return true;
      if (job.name === 'php.repo.setup') return inventory.ppa_configured;
      const installed = inventory.versions.some(item => item.version === version && item.installed);
      return job.name === 'php.remove' ? !installed : installed;
    }
    const inventory = await loadNodeData(true);
    return Boolean(inventory && (job.status !== 'succeeded' ||
      inventory.versions.some(item => item.version === version && item.is_installed)));
  }, [loadPhpData, loadNodeData]);
  const runtimeJobs = useRuntimeJobs(onRuntimeFinished, mainRuntimeNames);
  const jobsUnavailable = runtimeJobs.loading || Boolean(runtimeJobs.error);
  const phpRepoJob = runtimeJobs.jobs.find(job => job.name === 'php.repo.setup');
  const nodeBusyVersions = useMemo(() => [
    ...runtimeJobs.jobs.filter(job => job.name === 'node.install').map(runtimeJobVersion).filter((v): v is string => Boolean(v)),
    ...(processingNodeVersion ? [processingNodeVersion] : []),
  ], [runtimeJobs.jobs, processingNodeVersion]);

  useEffect(() => {
    if (activeTab === 'php') {
      loadPhpData();
    } else if (activeTab === 'node') {
      loadNodeData();
    } else if (activeTab === 'processes') {
      loadProcessData();
    }
  }, [activeTab, loadPhpData, loadNodeData, loadProcessData]);

  // =========================================================
  // PHP Handlers
  // =========================================================
  const handleSetupRepo = async () => {
    setSettingUpRepo(true);
    try {
      const job = await setupPhpRepo();
      runtimeJobs.track(job);
      toast.success('Tugas konfigurasi repositori PHP telah dibuat', {
        description: `ID Tugas: ${job.id}. Pembaruan paket sedang berjalan di latar belakang.`,
      });
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memulai konfigurasi repositori.';
      toast.error('Gagal mengatur repositori PHP', { description: msg });
    } finally {
      setSettingUpRepo(false);
    }
  };

  const handleInstallPhp = async (version: string) => {
    setProcessingPhpVersion(version);
    try {
      const job = await installPhpVersion(version);
      runtimeJobs.track(job);
      toast.success(`Tugas pemasangan PHP ${version} telah dibuat`, {
        description: `ID Tugas: ${job.id}. Paket sedang diunduh dan service OS akan di-mask.`,
      });
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : `Gagal memasang PHP ${version}.`;
      toast.error(`Gagal memasang PHP ${version}`, { description: msg });
    } finally {
      setProcessingPhpVersion(null);
    }
  };

  const handleConfirmRemovePhp = async () => {
    if (!phpVersionToRemove) return;
    const version = phpVersionToRemove;
    setPhpVersionToRemove(null);
    setProcessingPhpVersion(version);
    try {
      const job = await removePhpVersion(version);
      runtimeJobs.track(job);
      toast.success(`Tugas penghapusan PHP ${version} telah dibuat`, {
        description: `ID Tugas: ${job.id}. Paket sedang dihapus dari sistem.`,
      });
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : `Gagal menghapus PHP ${version}.`;
      toast.error(`Gagal menghapus PHP ${version}`, { description: msg });
    } finally {
      setProcessingPhpVersion(null);
    }
  };

  // =========================================================
  // Node.js Handlers
  // =========================================================
  const handleInstallNode = async (payload: NodeRuntimeInstallRequest) => {
    if (nodeSubmitting.current || jobsUnavailable || nodeLoading || nodeError || nodeData?.is_offline_fallback ||
      nodeBusyVersions.includes(payload.version) || !nodeData?.versions.some(item => item.version === payload.version && !item.is_installed)) return;
    nodeSubmitting.current = true;
    setProcessingNodeVersion(payload.version);
    try {
      const job = await installNodeVersion(payload);
      runtimeJobs.track(job);
      toast.success(`Tugas pemasangan Node.js v${payload.version} telah dibuat`, {
        description: `ID Tugas: ${job.id}. Pemasangan berjalan di latar belakang. Pantau melalui halaman Tugas.`,
      });
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : `Gagal memasang Node.js v${payload.version}.`;
      toast.error(`Pemasangan Node.js gagal`, { description: msg });
      throw err;
    } finally {
      nodeSubmitting.current = false;
      setProcessingNodeVersion(null);
    }
  };

  const handleConfirmRemoveNode = async () => {
    if (!nodeVersionToRemove) return;
    const item = nodeVersionToRemove;
    setNodeVersionToRemove(null);
    setProcessingNodeVersion(item.version);
    try {
      const res = await removeNodeVersion(item.version);
      toast.success(`Node.js v${item.version} berhasil dihapus`, {
        description: res.message,
      });
      await loadNodeData(true);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : `Gagal menghapus Node.js v${item.version}.`;
      toast.error(`Gagal menghapus Node.js`, { description: msg });
    } finally {
      setProcessingNodeVersion(null);
    }
  };

  const handleEnableCorepack = async (version: string) => {
    setProcessingNodeVersion(version);
    try {
      const res = await enableNodeCorepack(version);
      toast.success(`Corepack Node.js v${version} berhasil diaktifkan`, {
        description: res.message || 'Shims pnpm & yarn siap digunakan',
      });
      await loadNodeData(true);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal mengaktifkan Corepack.';
      toast.error('Gagal mengaktifkan Corepack', { description: msg });
    } finally {
      setProcessingNodeVersion(null);
    }
  };

  const filteredNodeVersions = useMemo(() => {
    if (!nodeData?.versions) return [];
    if (!nodeSearch.trim()) return nodeData.versions;
    const q = nodeSearch.toLowerCase();
    return nodeData.versions.filter(
      (v) =>
        v.version.toLowerCase().includes(q) ||
        (v.lts_codename && v.lts_codename.toLowerCase().includes(q)) ||
        (v.used_by_domains && v.used_by_domains.some((d) => d.toLowerCase().includes(q)))
    );
  }, [nodeData, nodeSearch]);

  const filteredProcesses = useMemo(() => {
    if (!processData) return [];
    return processData.processes.filter((p) => {
      if (processRuntimeFilter !== 'all' && p.runtime !== processRuntimeFilter) {
        return false;
      }
      if (processStatusFilter === 'active' && !p.active) return false;
      if (processStatusFilter === 'inactive' && p.active) return false;
      if (processSearch) {
        const q = processSearch.toLowerCase();
        return (
          p.primary_domain.toLowerCase().includes(q) ||
          p.service_name.toLowerCase().includes(q) ||
          p.runtime.toLowerCase().includes(q) ||
          (p.pid && p.pid.toString().includes(q))
        );
      }
      return true;
    });
  }, [processData, processRuntimeFilter, processStatusFilter, processSearch]);

  const installedNodeCount = nodeData?.installed_count ?? 0;
  const installedPhpCount = phpData?.versions?.filter((v) => v.installed).length ?? 0;
  const activeProcessCount = processData?.total_active ?? 0;
  const runtimeTabs: { id: RuntimeTab; label: string; icon: typeof Cpu; count?: number }[] = [
    { id: 'php', label: t('runtime.tabPhp'), icon: Cpu, count: installedPhpCount },
    { id: 'node', label: t('runtime.tabNode'), icon: Boxes, count: installedNodeCount },
    { id: 'python', label: 'Python', icon: Terminal },
    { id: 'settings', label: 'Pengaturan', icon: Settings },
    { id: 'processes', label: 'Monitoring Proses', icon: Activity, count: activeProcessCount },
    { id: 'databases', label: 'Server Basis Data', icon: Database },
  ];

  return (
    <div className="space-y-6 max-w-6xl mx-auto">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-xl font-bold tracking-tight text-[var(--ink)] flex items-center gap-2.5">
            {activeTab === 'php' ? (
              <>
                <Cpu className="w-5 h-5 text-[var(--primary)]" />
                {t('runtime.title')}
              </>
            ) : activeTab === 'node' ? (
              <>
                <Boxes className="w-5 h-5 text-emerald-500" />
                {t('runtime.nodeTitle')}
              </>
            ) : activeTab === 'python' ? (
              <><Terminal className="w-5 h-5 text-[var(--primary)]" />Manajer runtime Python</>
            ) : activeTab === 'settings' ? (
              <><Settings className="w-5 h-5 text-[var(--primary)]" />Pengaturan runtime</>
            ) : activeTab === 'databases' ? (
              <><Database className="w-5 h-5 text-[var(--primary)]" />Server basis data</>
            ) : (
              <>
                <Activity className="w-5 h-5 text-blue-500" />
                Monitoring Proses Aplikasi
              </>
            )}
          </h1>
          <p className="text-xs text-[var(--ink-muted)] mt-1">
            {activeTab === 'php'
              ? t('runtime.subtitle')
              : activeTab === 'node'
                ? t('runtime.nodeSubtitle')
                : activeTab === 'python' ? 'Kelola versi Python dan pemasangannya untuk aplikasi situs.' : activeTab === 'settings' ? 'Atur jadwal pembaruan katalog PHP, Node.js, dan Python. Pemasangan versi tetap manual.' : activeTab === 'databases' ? 'Kelola server database dan akses phpMyAdmin atau Adminer untuk tenant.' : 'Pantau status langsung, konsumsi memori, CPU, dan aliran log stdout/stderr dari aplikasi yang berjalan.'}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={() => {
              if (activeTab === 'php') loadPhpData();
              else if (activeTab === 'node') loadNodeData();
              else if (activeTab === 'python') setPythonRevision(value => value + 1);
              else if (activeTab === 'settings') setCatalogRevision(value => value + 1);
              else if (activeTab === 'databases') setDatabaseRevision(value => value + 1);
              else loadProcessData();
            }}
            disabled={
              activeTab === 'php'
                ? phpLoading
                : activeTab === 'node'
                  ? nodeLoading
                  : activeTab === 'python' || activeTab === 'settings' || activeTab === 'databases' ? false : processLoading
            }
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] hover:bg-[var(--line)] transition-colors disabled:opacity-50"
          >
            <RefreshCw
              className={`w-3.5 h-3.5 ${
                (activeTab === 'php'
                  ? phpLoading
                  : activeTab === 'node'
                    ? nodeLoading
                    : activeTab === 'python' || activeTab === 'settings' || activeTab === 'databases' ? false : processLoading)
                  ? 'animate-spin'
                  : ''
              }`}
            />
            {t('common.refresh')}
          </button>
        </div>
      </div>

      {/* Runtime navigation */}
      {runtimeJobs.error && (
        <p role="alert" className="text-xs text-[var(--danger)] whitespace-pre-wrap break-words">
          Status tugas runtime gagal dimuat: {runtimeJobs.error}. Pemasangan ditahan sampai status dapat diperiksa kembali.
        </p>
      )}
      <nav aria-label="Navigasi runtime" className="flex flex-wrap items-center gap-1 p-1 rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] w-full sm:w-fit">
        {[runtimeTabs.slice(0, 4), runtimeTabs.slice(4)].map((group, index) => (
          <div key={index} className="flex items-center gap-1">
            {group.map(({ id, label, icon: Icon, count }) => (
              <button
                key={id}
                onClick={() => setActiveTab(id)}
                aria-pressed={activeTab === id}
                className={`flex items-center gap-1.5 px-2 sm:px-3.5 py-2 text-xs font-medium whitespace-nowrap rounded-[var(--radius-md)] transition-colors ${activeTab === id ? 'bg-[var(--primary)] text-white' : 'text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--line)]/50'}`}
              >
                <Icon className="hidden sm:block w-3.5 h-3.5" />
                <span>{label}</span>
                {count !== undefined && (
                  <span className={`hidden sm:inline text-[10px] px-1.5 rounded-full font-semibold ${activeTab === id ? 'bg-white/20 text-white' : 'bg-[var(--line)] text-[var(--ink-muted)]'}`}>
                    {count}
                  </span>
                )}
              </button>
            ))}
          </div>
        ))}
      </nav>

      {activeTab === 'python' && <PythonRuntimesTab key={`python-${pythonRevision}`} onCatalogRefresh={() => setCatalogRevision(value => value + 1)} />}
      {activeTab === 'settings' && <CatalogScheduleCard key={`catalog-${catalogRevision}`} onRefresh={() => { void loadPhpData(true); void loadNodeData(true); setPythonRevision(value => value + 1); }} />}
      {/* ========================================================= */}
      {/* TAB CONTENT: PHP RUNTIMES                                  */}
      {/* ========================================================= */}
      {activeTab === 'php' && (
        <>
          {/* Security Architecture Notice */}
          <div className="p-4 rounded-[var(--radius-md)] border border-emerald-500/20 bg-emerald-500/5 flex items-start gap-3">
            <ShieldCheck className="w-5 h-5 text-emerald-500 shrink-0 mt-0.5" />
            <div className="text-xs space-y-1">
              <p className="font-semibold text-[var(--ink)]">
                Isolasi FPM Multi-Tenant &amp; Kebijakan Masking OS
              </p>
              <p className="text-[var(--ink-muted)] leading-relaxed">
                {t('runtime.maskingNote')}
              </p>
            </div>
          </div>

          {/* PHP repository status card */}
          <div className="p-5 rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-[var(--shadow-card)] space-y-4">
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 pb-4 border-b border-[var(--line)]">
              <div className="space-y-1">
                <div className="flex items-center gap-2">
                  <span className="text-xs font-medium text-[var(--ink-muted)]">
                    {t('runtime.ppaStatus')}:
                  </span>
                  {phpData?.ppa_configured ? (
                    <span className="inline-flex items-center gap-1 text-[11px] font-semibold text-emerald-600 dark:text-emerald-400 bg-emerald-500/10 px-2 py-0.5 rounded-full">
                      <CheckCircle2 className="w-3 h-3" />
                      {t('runtime.ppaConfigured')}
                    </span>
                  ) : (
                    <span className="inline-flex items-center gap-1 text-[11px] font-semibold text-amber-600 dark:text-amber-400 bg-amber-500/10 px-2 py-0.5 rounded-full">
                      <AlertCircle className="w-3 h-3" />
                      {t('runtime.ppaNotConfigured')}
                    </span>
                  )}
                </div>
                <p className="text-xs text-[var(--ink-muted)]">
                  {t('runtime.setupRepoDesc')}
                </p>
              </div>

              <button
                onClick={handleSetupRepo}
                disabled={settingUpRepo || phpLoading || jobsUnavailable || Boolean(phpRepoJob)}
                className="inline-flex items-center gap-1.5 px-3.5 py-2 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 transition-opacity disabled:opacity-50 shrink-0"
              >
                {settingUpRepo || phpRepoJob ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                ) : (
                  <ExternalLink className="w-3.5 h-3.5" />
                )}
                {t('runtime.setupRepo')}
              </button>
            </div>
            {phpRepoJob && <RuntimeJobProgress job={phpRepoJob} runtime="repositori PHP" />}
          </div>

          {/* Error Banner */}
          {phpError && (
            <div className="p-4 rounded-[var(--radius-md)] border border-rose-500/20 bg-rose-500/5 text-xs text-rose-600 dark:text-rose-400 flex items-center gap-2">
              <AlertCircle className="w-4 h-4 shrink-0" />
              <span>{phpError}</span>
            </div>
          )}

          {/* PHP Versions List */}
          <div className="rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-[var(--shadow-card)] overflow-hidden">
            <div className="px-5 py-4 border-b border-[var(--line)] flex items-center justify-between">
              <h2 className="text-sm font-semibold text-[var(--ink)]">
                Daftar Versi PHP yang Didukung
              </h2>
              <span className="text-xs text-[var(--ink-muted)]">
                {installedPhpCount} dari {phpData?.versions?.length || 0} terpasang
              </span>
            </div>

            <div className="divide-y divide-[var(--line)]">
              {phpLoading && !phpData ? (
                <div className="p-12 text-center text-xs text-[var(--ink-muted)] flex flex-col items-center gap-2">
                  <Loader2 className="w-5 h-5 animate-spin text-[var(--primary)]" />
                  <span>Memeriksa binary dan status service PHP di sistem...</span>
                </div>
              ) : (
                phpData?.versions.map((v: PhpVersionStatusItem) => {
                  const activeJob = runtimeJobs.jobs.find(job => job.name.startsWith('php.') && runtimeJobVersion(job) === v.version);
                  const isProcessing = processingPhpVersion === v.version || Boolean(activeJob);

                  return (
                    <div
                      key={v.version}
                      className="p-5 flex flex-col sm:flex-row sm:items-center justify-between gap-4 hover:bg-[var(--bg)]/50 transition-colors"
                    >
                      <div className="space-y-2">
                        <div className="flex items-center gap-3">
                          <div className="w-8 h-8 rounded-[var(--radius-md)] bg-[var(--primary)]/10 text-[var(--primary)] font-bold text-xs flex items-center justify-center">
                            {v.version}
                          </div>
                          <div>
                            <div className="flex items-center gap-2">
                              <span className="text-sm font-semibold text-[var(--ink)]">
                                PHP {v.version}
                                {v.latest_release && <span className="ml-2 text-xs text-[var(--ink-muted)]">Rilis {v.latest_release}</span>}
                              </span>
                              {activeJob ? (
                                <span role="status" className="inline-flex items-center gap-1 text-xs text-[var(--primary)]">
                                  <Loader2 className="w-3 h-3 animate-spin" />
                                  {activeJob.status === 'succeeded' ? 'Memperbarui daftar' : activeJob.status === 'queued' ? 'Dalam antrean' : 'Sedang diproses'} · {activeJob.progress}%
                                </span>
                              ) : v.installed ? (
                                <span className="inline-flex items-center gap-1 text-[10px] font-semibold text-emerald-600 dark:text-emerald-400 bg-emerald-500/10 px-2 py-0.5 rounded-full">
                                  <CheckCircle2 className="w-2.5 h-2.5" />
                                  {t('runtime.installed')}
                                </span>
                              ) : (
                                <span className="inline-flex items-center text-[10px] font-medium text-[var(--ink-muted)] bg-[var(--line)]/50 px-2 py-0.5 rounded-full">
                                  {t('runtime.available')}
                                </span>
                              )}
                              {v.installed && v.service_masked && (
                                <span className="inline-flex items-center gap-1 text-[10px] font-semibold text-violet-600 dark:text-violet-400 bg-violet-500/10 px-2 py-0.5 rounded-full">
                                  <ShieldCheck className="w-2.5 h-2.5" />
                                  {t('runtime.masked')}
                                </span>
                              )}
                            </div>
                            <div className="text-[11px] text-[var(--ink-muted)] flex items-center gap-3 mt-1">
                              <span>
                                Service:{' '}
                                <code className="px-1 py-0.5 rounded bg-[var(--line)] font-mono text-[10px]">
                                  {v.service_name}
                                </code>
                              </span>
                            </div>
                          </div>
                        </div>

                        {v.installed && (
                          <div className="pl-11 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-[var(--ink-muted)] font-mono">
                            <span className="flex items-center gap-1">
                              <Terminal className="w-3 h-3 text-[var(--primary)]" />
                              CLI: {v.cli_binary_path}
                            </span>
                            <span className="flex items-center gap-1">
                              <Cpu className="w-3 h-3 text-emerald-500" />
                              FPM: {v.fpm_binary_path}
                            </span>
                          </div>
                        )}
                        {activeJob && (
                          <div className="pl-11">
                            <RuntimeJobProgress job={activeJob} runtime={`PHP ${v.version}`} />
                          </div>
                        )}
                      </div>

                      <div className="flex items-center gap-2 sm:self-center pl-11 sm:pl-0">
                        {v.installed ? (
                          <>
                            <button
                              onClick={() => setVersionForExtensions(v.version)}
                              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--card)] hover:bg-[var(--line)]/40 text-[var(--ink)] transition-colors"
                            >
                              <Puzzle className="w-3.5 h-3.5 text-[var(--primary)]" />
                              {t('runtime.extensions')}
                            </button>

                            <button
                              onClick={() => setPhpVersionToRemove(v.version)}
                              disabled={isProcessing}
                              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-rose-500/20 text-rose-600 dark:text-rose-400 hover:bg-rose-500/10 transition-colors disabled:opacity-50"
                            >
                              {isProcessing ? (
                                <Loader2 className="w-3.5 h-3.5 animate-spin" />
                              ) : (
                                <Trash2 className="w-3.5 h-3.5" />
                              )}
                              {t('runtime.remove')}
                            </button>
                          </>
                        ) : (
                          <button
                            onClick={() => handleInstallPhp(v.version)}
                            disabled={isProcessing || jobsUnavailable || Boolean(phpRepoJob)}
                            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 transition-opacity disabled:opacity-50"
                          >
                            {isProcessing ? (
                              <Loader2 className="w-3.5 h-3.5 animate-spin" />
                            ) : (
                              <Download className="w-3.5 h-3.5" />
                            )}
                            {activeJob ? activeJob.status === 'succeeded' ? 'Memperbarui daftar' : 'Sedang diproses' : t('runtime.install')}
                          </button>
                        )}
                      </div>
                    </div>
                  );
                })
              )}
            </div>
          </div>
        </>
      )}

      {/* ========================================================= */}
      {/* TAB CONTENT: NODE.JS RUNTIMES (Phase 7 - P7-T1)            */}
      {/* ========================================================= */}
      {activeTab === 'node' && (
        <>
          {/* Architecture & Security Notice */}
          <div className="p-4 rounded-[var(--radius-md)] border border-emerald-500/20 bg-emerald-500/5 flex items-start gap-3">
            <ShieldCheck className="w-5 h-5 text-emerald-600 dark:text-emerald-400 shrink-0 mt-0.5" />
            <div className="text-xs space-y-1">
              <p className="font-semibold text-[var(--ink)]">
                Isolasi Direktori Node.js &amp; Proteksi Integritas SHA256
              </p>
              <p className="text-[var(--ink-muted)] leading-relaxed">
                {t('runtime.nodeNotice')}
              </p>
            </div>
          </div>

          {/* Mirror & Catalog Summary */}
          <div className="p-4 rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-[var(--shadow-card)] flex flex-col sm:flex-row sm:items-center justify-between gap-4">
            <div className="space-y-1 text-xs">
              <div className="flex items-center gap-2">
                <span className="font-medium text-[var(--ink)]">Katalog Rilis Node.js:</span>
                <span className="font-mono text-[11px] text-[var(--ink-muted)]">
                  {nodeData?.mirror_url || 'https://nodejs.org/dist'}
                </span>
                {nodeData?.is_offline_fallback && (
                  <span className="inline-flex items-center gap-1 text-[10px] font-semibold text-amber-700 dark:text-amber-400 bg-amber-500/10 px-2 py-0.5 rounded-full">
                    <AlertTriangle className="w-2.5 h-2.5" />
                    Katalog perlu diperbarui
                  </span>
                )}
              </div>
              <p className="text-[11px] text-[var(--ink-muted)]">
                Lima patch terbaru per lini LTS yang masih didukung. Versi yang telah berakhir dukungannya tidak ditawarkan untuk pemasangan; runtime terpasang tetap ditampilkan.
              </p>
            </div>

            {/* Search filter */}
            <div className="relative w-full sm:w-64">
              <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-[var(--ink-muted)]" />
              <input
                type="text"
                placeholder="Cari versi atau nama LTS..."
                value={nodeSearch}
                onChange={(e) => setNodeSearch(e.target.value)}
                className="w-full pl-8 pr-3 py-1.5 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] placeholder:text-[var(--ink-muted)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)]"
              />
            </div>
          </div>

          {/* Error Banner */}
          {nodeError && (
            <div className="p-4 rounded-[var(--radius-md)] border border-rose-500/20 bg-rose-500/5 text-xs text-rose-600 dark:text-rose-400 flex items-center gap-2">
              <AlertCircle className="w-4 h-4 shrink-0" />
              <span>{nodeError}</span>
            </div>
          )}

          {/* Node.js Versions Table */}
          <div className="rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-[var(--shadow-card)] overflow-hidden">
            <div className="px-5 py-4 border-b border-[var(--line)] flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Boxes className="w-4 h-4 text-emerald-600 dark:text-emerald-400" />
                <h2 className="text-sm font-semibold text-[var(--ink)]">
                  Daftar Versi Node.js
                </h2>
              </div>
              <span className="text-xs text-[var(--ink-muted)]">
                {installedNodeCount} terpasang di sistem
              </span>
            </div>

            <div className="divide-y divide-[var(--line)]">
              {nodeLoading && !nodeData ? (
                <div className="p-12 text-center text-xs text-[var(--ink-muted)] flex flex-col items-center gap-2">
                  <Loader2 className="w-5 h-5 animate-spin text-emerald-600" />
                  <span>Memeriksa runtime Node.js dan catalog resmi...</span>
                </div>
              ) : filteredNodeVersions.length === 0 ? (
                <div className="p-10 text-center text-xs text-[var(--ink-muted)]">
                  Tidak ada versi Node.js yang sesuai dengan pencarian.
                </div>
              ) : (
                filteredNodeVersions.map((v) => {
                  const activeJob = runtimeJobs.jobs.find(job => job.name === 'node.install' && runtimeJobVersion(job) === v.version);
                  const isProcessing = processingNodeVersion === v.version || Boolean(activeJob);

                  return (
                    <div
                      key={v.version}
                      className="p-5 flex flex-col sm:flex-row sm:items-center justify-between gap-4 hover:bg-[var(--bg)]/50 transition-colors"
                    >
                      <div className="space-y-2">
                        <div className="flex items-center gap-3">
                          <div className="w-9 h-9 rounded-[var(--radius-md)] bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 font-bold text-xs flex items-center justify-center font-mono">
                            v{v.version.split('.')[0]}
                          </div>

                          <div>
                            <div className="flex items-center flex-wrap gap-2">
                              <span className="text-sm font-semibold text-[var(--ink)] font-mono">
                                Node.js v{v.version}
                                {v.support_status === 'eol' && <span className="ml-2 text-xs text-amber-700 dark:text-amber-400">Dukungan berakhir</span>}
                                {v.support_status === 'unverified' && <span className="ml-2 text-xs text-[var(--ink-muted)]">Dukungan belum terverifikasi</span>}
                              </span>

                              {v.is_lts && (
                                <span className="inline-flex items-center gap-1 text-[10px] font-semibold text-emerald-700 dark:text-emerald-400 bg-emerald-500/10 px-2 py-0.5 rounded-full">
                                  <ShieldCheck className="w-2.5 h-2.5" />
                                  LTS {v.lts_codename ? `(${v.lts_codename})` : ''}
                                </span>
                              )}

                              {activeJob ? (
                                <span role="status" className="inline-flex items-center gap-1 text-xs text-[var(--primary)]">
                                  <Loader2 className="w-3 h-3 animate-spin" />
                                  {activeJob.status === 'succeeded' ? 'Memperbarui daftar' : activeJob.status === 'queued' ? 'Dalam antrean' : 'Sedang dipasang'} · {activeJob.progress}%
                                </span>
                              ) : v.is_installed ? (
                                <span className="inline-flex items-center gap-1 text-[10px] font-semibold text-emerald-700 dark:text-emerald-400 bg-emerald-500/15 px-2 py-0.5 rounded-full">
                                  <CheckCircle2 className="w-2.5 h-2.5" />
                                  {t('runtime.installed')}
                                </span>
                              ) : (
                                <span className="inline-flex items-center text-[10px] font-medium text-[var(--ink-muted)] bg-[var(--line)]/50 px-2 py-0.5 rounded-full">
                                  {t('runtime.available')}
                                </span>
                              )}

                              {v.is_installed && v.corepack_enabled && (
                                <span className="inline-flex items-center gap-1 text-[10px] font-semibold text-cyan-700 dark:text-cyan-400 bg-cyan-500/10 px-2 py-0.5 rounded-full">
                                  <Package className="w-2.5 h-2.5" />
                                  Corepack (pnpm &amp; yarn)
                                </span>
                              )}

                              {v.used_by_sites_count > 0 && (
                                <span className="inline-flex items-center gap-1 text-[10px] font-semibold text-purple-700 dark:text-purple-400 bg-purple-500/10 px-2 py-0.5 rounded-full">
                                  <Layers className="w-2.5 h-2.5" />
                                  Digunakan {v.used_by_sites_count} Situs
                                </span>
                              )}
                            </div>

                            <div className="text-[11px] text-[var(--ink-muted)] flex flex-wrap items-center gap-x-4 gap-y-1 mt-1 font-mono">
                              {v.installed_path && (
                                <span>Path: {v.installed_path}</span>
                              )}
                              {v.npm_version && (
                                <span>npm v{v.npm_version}</span>
                              )}
                              {v.release_date && (
                                <span>Rilis: {v.release_date}</span>
                              )}
                            </div>
                          </div>
                        </div>

                        {/* Sites in use list */}
                        {activeJob && (
                          <div className="pl-12">
                            <RuntimeJobProgress job={activeJob} runtime={`Node.js v${v.version}`} />
                          </div>
                        )}
                        {v.used_by_domains && v.used_by_domains.length > 0 && (
                          <div className="pl-12 flex flex-wrap items-center gap-1.5 text-[11px] text-[var(--ink-muted)]">
                            <span className="font-medium text-[var(--ink)]">Situs aktif:</span>
                            {v.used_by_domains.map((dom) => (
                              <span
                                key={dom}
                                className="px-1.5 py-0.5 rounded bg-[var(--line)] font-mono text-[10px] text-[var(--ink)]"
                              >
                                {dom}
                              </span>
                            ))}
                          </div>
                        )}
                      </div>

                      {/* Action buttons */}
                      <div className="flex items-center gap-2 sm:self-center pl-12 sm:pl-0 shrink-0">
                        {v.is_installed ? (
                          <>
                            <button
                              onClick={() => handleEnableCorepack(v.version)}
                              disabled={isProcessing}
                              title="Aktifkan atau perbarui wrapper Corepack (pnpm & yarn)"
                              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--card)] hover:bg-[var(--line)]/40 text-[var(--ink)] transition-colors disabled:opacity-50"
                            >
                              {isProcessing ? (
                                <Loader2 className="w-3.5 h-3.5 animate-spin" />
                              ) : (
                                <Package className="w-3.5 h-3.5 text-cyan-600 dark:text-cyan-400" />
                              )}
                              <span>Corepack</span>
                            </button>

                            <button
                              onClick={() => setNodeVersionToRemove(v)}
                              disabled={isProcessing || !v.can_delete}
                              title={
                                v.can_delete
                                  ? `Hapus Node.js v${v.version} dari sistem`
                                  : `Tidak dapat dihapus karena sedang digunakan oleh ${v.used_by_sites_count} situs`
                              }
                              className={`inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border transition-colors ${
                                v.can_delete
                                  ? 'border-rose-500/20 text-rose-600 dark:text-rose-400 hover:bg-rose-500/10'
                                  : 'border-[var(--line)] text-[var(--ink-muted)] opacity-50 cursor-not-allowed bg-[var(--line)]/20'
                              } disabled:opacity-40`}
                            >
                              {isProcessing ? (
                                <Loader2 className="w-3.5 h-3.5 animate-spin" />
                              ) : (
                                <Trash2 className="w-3.5 h-3.5" />
                              )}
                              <span>Hapus</span>
                            </button>
                          </>
                        ) : (
                          <button
                            onClick={() => { void handleInstallNode(nodeInstallPayload(v.version, nodeInstallOptions)).catch(() => {}); }}
                            disabled={isProcessing || jobsUnavailable || nodeLoading || Boolean(nodeError) || Boolean(processingNodeVersion) || nodeData?.is_offline_fallback}
                            aria-label={`Pasang Node.js ${v.version}`}
                            className="inline-flex items-center gap-1.5 px-3.5 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-emerald-600 text-white hover:bg-emerald-700 transition-colors disabled:opacity-50 shadow-sm"
                          >
                            {isProcessing ? (
                              <Loader2 className="w-3.5 h-3.5 animate-spin" />
                            ) : (
                              <Download className="w-3.5 h-3.5" />
                            )}
                            <span>{activeJob ? activeJob.status === 'succeeded' ? 'Memperbarui daftar' : 'Sedang dipasang' : 'Pasang'}</span>
                          </button>
                        )}
                      </div>
                    </div>
                  );
                })
              )}
            </div>
          </div>
          <NodeInstallSettings value={nodeInstallOptions} onChange={setNodeInstallOptions} disabled={jobsUnavailable || Boolean(processingNodeVersion)} />
        </>
      )}

      {/* ========================================================= */}
      {/* TAB CONTENT: PROCESS MONITORING (Phase 7 - P7-T5)         */}
      {/* ========================================================= */}
      {activeTab === 'processes' && (
        <div className="space-y-6">
          {/* Error Banner */}
          {processError && (
            <div className="p-4 rounded-[var(--radius-md)] border border-rose-500/20 bg-rose-500/5 text-xs text-rose-600 dark:text-rose-400 flex items-center gap-2">
              <AlertCircle className="w-4 h-4 shrink-0" />
              <span>{processError}</span>
            </div>
          )}

          {/* Summary Stat Cards */}
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
            <div className="p-4 rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-[var(--shadow-card)]">
              <div className="flex items-center gap-2 text-xs text-[var(--ink-muted)] mb-1">
                <Activity className="w-4 h-4 text-blue-500" />
                <span>Total Proses Terkelola</span>
              </div>
              <div className="text-2xl font-bold font-mono text-[var(--ink)]">
                {processData?.processes.length ?? 0}
              </div>
              <span className="text-[11px] text-[var(--ink-muted)] block mt-1">
                Node.js dan Python yang terkonfigurasi
              </span>
            </div>

            <div className="p-4 rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-[var(--shadow-card)]">
              <div className="flex items-center gap-2 text-xs text-[var(--ink-muted)] mb-1">
                <CheckCircle2 className="w-4 h-4 text-emerald-500" />
                <span>Proses Berjalan (Aktif)</span>
              </div>
              <div className="text-2xl font-bold font-mono text-emerald-600 dark:text-emerald-400">
                {processData?.total_active ?? 0}
              </div>
              <span className="text-[11px] text-[var(--ink-muted)] block mt-1">
                Sedang melayani lalu lintas aplikasi
              </span>
            </div>

            <div className="p-4 rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-[var(--shadow-card)]">
              <div className="flex items-center gap-2 text-xs text-[var(--ink-muted)] mb-1">
                <HardDrive className="w-4 h-4 text-violet-500" />
                <span>Total Memori (RAM) Terpakai</span>
              </div>
              <div className="text-2xl font-bold font-mono text-[var(--ink)]">
                {processData?.total_memory_bytes
                  ? (processData.total_memory_bytes / (1024 * 1024)).toFixed(1) + ' MB'
                  : '0 MB'}
              </div>
              <span className="text-[11px] text-[var(--ink-muted)] block mt-1">
                Berdasarkan metrik cgroup terkini
              </span>
            </div>
          </div>

          {/* Filter Bar */}
          <div className="p-4 rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-[var(--shadow-card)] flex flex-col sm:flex-row sm:items-center justify-between gap-3">
            <div className="relative flex-1 max-w-sm">
              <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-[var(--ink-muted)]" />
              <input
                type="text"
                placeholder="Cari domain, PID, atau service..."
                value={processSearch}
                onChange={(e) => setProcessSearch(e.target.value)}
                className="w-full pl-9 pr-3 py-1.5 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] placeholder:text-[var(--ink-muted)] focus:outline-none focus:border-[var(--primary)]"
              />
            </div>

            <div className="flex items-center gap-2 flex-wrap">
              <select
                value={processRuntimeFilter}
                onChange={(e) =>
                  setProcessRuntimeFilter(e.target.value as 'all' | 'nodejs' | 'python')
                }
                className="py-1.5 px-2.5 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
              >
                <option value="all">Semua Runtime</option>
                <option value="nodejs">Node.js</option>
                <option value="python">Python</option>
              </select>

              <select
                value={processStatusFilter}
                onChange={(e) =>
                  setProcessStatusFilter(e.target.value as 'all' | 'active' | 'inactive')
                }
                className="py-1.5 px-2.5 text-xs rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
              >
                <option value="all">Semua Status</option>
                <option value="active">Hanya Aktif</option>
                <option value="inactive">Hanya Nonaktif</option>
              </select>
            </div>
          </div>

          {/* Process Cards List */}
          {processLoading && !processData ? (
            <div className="p-12 text-center text-xs text-[var(--ink-muted)] flex flex-col items-center gap-2 bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)]">
              <Loader2 className="w-5 h-5 animate-spin text-[var(--primary)]" />
              <span>Memuat status proses aplikasi...</span>
            </div>
          ) : filteredProcesses.length === 0 ? (
            <div className="p-12 text-center text-xs text-[var(--ink-muted)] bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)]">
              <p className="font-semibold text-sm text-[var(--ink)] mb-1">
                Tidak ada proses yang ditemukan
              </p>
              <p>
                {processSearch ||
                processRuntimeFilter !== 'all' ||
                processStatusFilter !== 'all'
                  ? 'Coba sesuaikan kata kunci pencarian atau filter status Anda.'
                  : 'Belum ada situs Node.js atau Python yang dibuat pada panel ini.'}
              </p>
            </div>
          ) : (
            <div className="space-y-4">
              {filteredProcesses.map((proc) => (
                <UnifiedProcessCard
                  key={`${proc.runtime}-${proc.site_id}`}
                  siteId={proc.site_id}
                  runtime={proc.runtime as 'nodejs' | 'python'}
                  domain={proc.primary_domain}
                  initialProcess={proc}
                  onRefresh={loadProcessData}
                />
              ))}
            </div>
          )}
        </div>
      )}

      {/* ========================================================= */}
      {/* TAB CONTENT: DATABASE SERVERS (Phase 8 - P8-T5)           */}
      {/* ========================================================= */}
      {activeTab === 'databases' && (
        <DatabaseServersTab key={databaseRevision} />
      )}

      {/* ========================================================= */}
      {/* MODALS & DIALOGS                                          */}
      {/* ========================================================= */}

      {/* Confirmation Modal for PHP Removal */}
      <ConfirmDangerDialog
        isOpen={Boolean(phpVersionToRemove)}
        onClose={() => setPhpVersionToRemove(null)}
        onConfirm={handleConfirmRemovePhp}
        title={`Hapus Runtime PHP ${phpVersionToRemove}?`}
        description={`Penghapusan akan mencopot paket PHP ${phpVersionToRemove} dari sistem. Tindakan ini hanya dapat dilakukan jika tidak ada situs aktif yang menggunakannya.`}
        confirmMatchText={`hapus php ${phpVersionToRemove}`}
        confirmButtonLabel="Hapus Runtime PHP"
      />

      {/* Confirmation Modal for Node.js Removal */}
      <ConfirmDangerDialog
        isOpen={Boolean(nodeVersionToRemove)}
        onClose={() => setNodeVersionToRemove(null)}
        onConfirm={handleConfirmRemoveNode}
        title={`Hapus Runtime Node.js v${nodeVersionToRemove?.version}?`}
        description={`Penghapusan akan menghapus seluruh file binary dan pustaka Node.js v${nodeVersionToRemove?.version} dari direktori sistem /opt/panel/runtimes/node/${nodeVersionToRemove?.version}. Pastikan tidak ada aplikasi yang memerlukannya.`}
        confirmMatchText={`hapus node ${nodeVersionToRemove?.version}`}
        confirmButtonLabel="Hapus Runtime Node.js"
      />

      {/* PHP Extensions Management Modal */}
      {versionForExtensions && (
        <PhpExtensionsModal
          version={versionForExtensions}
          isOpen={Boolean(versionForExtensions)}
          onClose={() => setVersionForExtensions(null)}
        />
      )}

    </div>
  );
};

export default RuntimePage;
