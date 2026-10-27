import React, { useState, useEffect, useCallback } from 'react';
import {
  Activity,
  RefreshCw,
  AlertTriangle,
  CheckCircle2,
  XCircle,
  Play,
  Square,
  RotateCw,
  Terminal,
  Loader2,
  Copy,
  Check,
  Package,
  Wrench,
  Cpu,
  Clock,
  HardDrive,
  Hash,
  Radio,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  getNodeSiteStatus,
  postNodeSiteAction,
  runNodeSiteJob,
} from '../../api/client';
import {
  NodeSiteStatusApiResponse,
  NodeSiteJobApiResponse,
} from '../../types/api';

export interface NodeStatusCardProps {
  siteId: number;
}

export const NodeStatusCard: React.FC<NodeStatusCardProps> = ({ siteId }) => {
  const { t } = useTranslation();
  const [status, setStatus] = useState<NodeSiteStatusApiResponse | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [jobLoading, setJobLoading] = useState<string | null>(null);
  const [customScript, setCustomScript] = useState<string>('');
  const [jobResult, setJobResult] = useState<NodeSiteJobApiResponse | null>(null);
  const [copiedStdout, setCopiedStdout] = useState(false);

  const loadStatus = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const data = await getNodeSiteStatus(siteId);
      setStatus(data);
    } catch (err: unknown) {
      const msg =
        err instanceof Error ? err.message : 'Gagal memuat status proses Node.js';
      setError(msg);
    } finally {
      setIsLoading(false);
    }
  }, [siteId]);

  useEffect(() => {
    loadStatus();
    const timer = setInterval(() => {
      // Background poll status every 10 seconds
      getNodeSiteStatus(siteId)
        .then((data) => setStatus(data))
        .catch(() => {});
    }, 10000);
    return () => clearInterval(timer);
  }, [siteId, loadStatus]);

  const handleAction = async (action: 'start' | 'stop' | 'restart' | 'redeploy') => {
    setActionLoading(action);
    setError(null);
    try {
      await postNodeSiteAction(siteId, { action });
      await loadStatus();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : `Gagal mengeksekusi aksi ${action}`;
      setError(msg);
    } finally {
      setActionLoading(null);
    }
  };

  const handleRunJob = async (jobType: string, scriptName?: string) => {
    setJobLoading(scriptName || jobType);
    setError(null);
    try {
      const res = await runNodeSiteJob(siteId, {
        job_type: jobType,
        script_name: scriptName,
      });
      setJobResult(res);
      await loadStatus();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal mengeksekusi perintah';
      setError(msg);
    } finally {
      setJobLoading(null);
    }
  };

  const formatBytes = (bytes: number | null): string => {
    if (!bytes || bytes <= 0) return '-';
    const mb = bytes / (1024 * 1024);
    if (mb < 1) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${mb.toFixed(1)} MB`;
  };

  const formatCpu = (usec: number | null): string => {
    if (!usec || usec <= 0) return '-';
    const ms = usec / 1000;
    if (ms < 1000) return `${ms.toFixed(0)} ms`;
    return `${(ms / 1000).toFixed(2)} s`;
  };

  const formatUptime = (secs: number | null): string => {
    if (!secs || secs <= 0) return '-';
    const d = Math.floor(secs / 86400);
    const h = Math.floor((secs % 86400) / 3600);
    const m = Math.floor((secs % 3600) / 60);
    const s = secs % 60;
    if (d > 0) return `${d}h ${h}j ${m}m`;
    if (h > 0) return `${h}j ${m}m ${s}d`;
    if (m > 0) return `${m}m ${s}d`;
    return `${s}d`;
  };

  const copyJobOutput = () => {
    if (!jobResult) return;
    const combined = `=== Output (${jobResult.command}) ===\nExit Code: ${jobResult.exit_code}\nDuration: ${jobResult.duration_ms}ms\n\nSTDOUT:\n${jobResult.stdout}\n\nSTDERR:\n${jobResult.stderr}`;
    navigator.clipboard.writeText(combined);
    setCopiedStdout(true);
    setTimeout(() => setCopiedStdout(false), 2000);
  };

  return (
    <div className="space-y-6">
      {/* 1. Status Overview Card */}
      <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-md)] p-5 space-y-5">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--line)] pb-4">
          <div className="flex items-center gap-2.5">
            <Activity className="w-4 h-4 text-[var(--primary)]" />
            <h4 className="text-sm font-semibold text-[var(--ink)]">
              {t('sites.node.cardTitle')}
            </h4>
            {status && (
              <span className="text-xs text-[var(--ink-muted)] font-mono">
                ({status.service_name})
              </span>
            )}
          </div>

          <div className="flex items-center gap-2">
            {status && (
              <>
                {/* Active / Inactive Pill */}
                <span
                  className={`inline-flex items-center gap-1.5 text-xs px-2.5 py-0.5 rounded-full font-medium ${
                    status.active
                      ? 'bg-emerald-100 text-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-300'
                      : 'bg-red-100 text-red-800 dark:bg-red-950/40 dark:text-red-300'
                  }`}
                >
                  {status.active ? (
                    <>
                      <CheckCircle2 className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-400" />
                      {t('sites.node.active')}
                    </>
                  ) : (
                    <>
                      <XCircle className="w-3.5 h-3.5 text-red-600 dark:text-red-400" />
                      {t('sites.node.inactive')}
                    </>
                  )}
                </span>

                {/* Health Check Badge */}
                <span
                  className={`inline-flex items-center gap-1.5 text-xs px-2.5 py-0.5 rounded-full font-medium ${
                    status.healthy
                      ? 'bg-blue-100 text-blue-800 dark:bg-blue-950/40 dark:text-blue-300'
                      : 'bg-amber-100 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300'
                  }`}
                  title={status.probe_message || undefined}
                >
                  <Radio className="w-3.5 h-3.5 text-blue-600 dark:text-blue-400" />
                  {status.healthy ? t('sites.node.healthy') : t('sites.node.unhealthy')}
                </span>
              </>
            )}

            <button
              onClick={loadStatus}
              disabled={isLoading}
              className="p-1 rounded text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] transition-colors"
              title="Perbarui status"
            >
              <RefreshCw
                className={`w-3.5 h-3.5 ${isLoading ? 'animate-spin' : ''}`}
              />
            </button>
          </div>
        </div>

        {error && (
          <div className="p-3 rounded-[var(--radius-md)] bg-[var(--danger)]/10 border border-[var(--danger)]/20 text-[var(--danger)] text-xs flex items-start gap-2">
            <AlertTriangle className="w-4 h-4 shrink-0 mt-0.5" />
            <span>{error}</span>
          </div>
        )}

        {status && (
          <>
            {/* Metrics Grid */}
            <div className="grid grid-cols-2 sm:grid-cols-5 gap-3">
              <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
                <div className="flex items-center gap-1.5 text-xs text-[var(--ink-muted)] mb-1">
                  <Hash className="w-3.5 h-3.5" />
                  Port Terikat
                </div>
                <div className="text-sm font-semibold text-[var(--ink)] font-mono">
                  {status.port}
                </div>
                <div className="text-[10px] text-[var(--ink-muted)] mt-0.5">
                  127.0.0.1:{status.port}
                </div>
              </div>

              <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
                <div className="flex items-center gap-1.5 text-xs text-[var(--ink-muted)] mb-1">
                  <Activity className="w-3.5 h-3.5" />
                  PID Proses
                </div>
                <div className="text-sm font-semibold text-[var(--ink)] font-mono">
                  {status.pid ?? '-'}
                </div>
                <div className="text-[10px] text-[var(--ink-muted)] mt-0.5">
                  Substate: {status.sub_state}
                </div>
              </div>

              <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
                <div className="flex items-center gap-1.5 text-xs text-[var(--ink-muted)] mb-1">
                  <HardDrive className="w-3.5 h-3.5" />
                  Memori RAM
                </div>
                <div className="text-sm font-semibold text-[var(--ink)]">
                  {formatBytes(status.memory_bytes)}
                </div>
                <div className="text-[10px] text-[var(--ink-muted)] mt-0.5">
                  Tenant cgroup slice
                </div>
              </div>

              <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
                <div className="flex items-center gap-1.5 text-xs text-[var(--ink-muted)] mb-1">
                  <Cpu className="w-3.5 h-3.5" />
                  Waktu CPU
                </div>
                <div className="text-sm font-semibold text-[var(--ink)]">
                  {formatCpu(status.cpu_usage_usec)}
                </div>
                <div className="text-[10px] text-[var(--ink-muted)] mt-0.5">
                  CPU throttled by slice
                </div>
              </div>

              <div className="p-3 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]">
                <div className="flex items-center gap-1.5 text-xs text-[var(--ink-muted)] mb-1">
                  <Clock className="w-3.5 h-3.5" />
                  Masa Aktif
                </div>
                <div className="text-sm font-semibold text-[var(--ink)]">
                  {formatUptime(status.uptime_secs)}
                </div>
                <div className="text-[10px] text-[var(--ink-muted)] mt-0.5">
                  HTTP probe: {status.probe_status_code ?? '-'}
                </div>
              </div>
            </div>

            {/* Process Action Controls */}
            <div className="pt-2 border-t border-[var(--line)] flex flex-wrap items-center gap-2">
              <span className="text-xs font-medium text-[var(--ink-muted)] mr-2">
                {t('sites.node.actions')}:
              </span>

              <button
                onClick={() => handleAction('start')}
                disabled={actionLoading !== null || status.active}
                className="text-xs px-3 py-1.5 rounded-[var(--radius-sm)] bg-emerald-600 text-white hover:bg-emerald-700 disabled:opacity-50 transition-colors flex items-center gap-1.5 shadow-xs"
              >
                {actionLoading === 'start' ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                ) : (
                  <Play className="w-3.5 h-3.5" />
                )}
                {t('sites.node.start')}
              </button>

              <button
                onClick={() => handleAction('stop')}
                disabled={actionLoading !== null || !status.active}
                className="text-xs px-3 py-1.5 rounded-[var(--radius-sm)] bg-red-600 text-white hover:bg-red-700 disabled:opacity-50 transition-colors flex items-center gap-1.5 shadow-xs"
              >
                {actionLoading === 'stop' ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                ) : (
                  <Square className="w-3.5 h-3.5" />
                )}
                {t('sites.node.stop')}
              </button>

              <button
                onClick={() => handleAction('restart')}
                disabled={actionLoading !== null}
                className="text-xs px-3 py-1.5 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface-input)] disabled:opacity-50 transition-colors flex items-center gap-1.5"
              >
                {actionLoading === 'restart' ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                ) : (
                  <RotateCw className="w-3.5 h-3.5" />
                )}
                {t('sites.node.restart')}
              </button>

              <button
                onClick={() => handleAction('redeploy')}
                disabled={actionLoading !== null}
                className="text-xs px-3 py-1.5 rounded-[var(--radius-sm)] bg-[var(--primary)] text-white hover:bg-[var(--primary)]/90 disabled:opacity-50 transition-colors flex items-center gap-1.5 shadow-xs"
              >
                {actionLoading === 'redeploy' ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                ) : (
                  <Wrench className="w-3.5 h-3.5" />
                )}
                {t('sites.node.redeploy')}
              </button>
            </div>
          </>
        )}
      </div>

      {/* 2. Tenant Jobs Command Runner */}
      <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-md)] p-5 space-y-4">
        <div className="flex items-center gap-2 border-b border-[var(--line)] pb-3">
          <Terminal className="w-4 h-4 text-[var(--primary)]" />
          <div>
            <h4 className="text-sm font-semibold text-[var(--ink)]">
              {t('sites.node.jobsTitle')}
            </h4>
            <p className="text-xs text-[var(--ink-muted)]">
              {t('sites.node.jobsDesc')}
            </p>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-3">
          <button
            onClick={() => handleRunJob('npm_install')}
            disabled={jobLoading !== null}
            className="text-xs px-3 py-1.5 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] hover:border-[var(--primary)] disabled:opacity-50 transition-colors flex items-center gap-1.5"
          >
            {jobLoading === 'npm_install' ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin text-[var(--primary)]" />
            ) : (
              <Package className="w-3.5 h-3.5 text-[var(--primary)]" />
            )}
            {t('sites.node.runNpmInstall')}
          </button>

          <button
            onClick={() => handleRunJob('npm_build')}
            disabled={jobLoading !== null}
            className="text-xs px-3 py-1.5 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] hover:border-[var(--primary)] disabled:opacity-50 transition-colors flex items-center gap-1.5"
          >
            {jobLoading === 'npm_build' ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin text-[var(--primary)]" />
            ) : (
              <Wrench className="w-3.5 h-3.5 text-[var(--primary)]" />
            )}
            {t('sites.node.runNpmBuild')}
          </button>

          {/* Custom script runner */}
          <div className="flex items-center gap-1.5 flex-1 min-w-[240px]">
            <input
              type="text"
              placeholder={t('sites.node.scriptPlaceholder')}
              value={customScript}
              onChange={(e) => setCustomScript(e.target.value)}
              className="text-xs px-3 py-1.5 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] placeholder:text-[var(--ink-muted)] flex-1 focus:outline-none focus:border-[var(--primary)]"
            />
            <button
              onClick={() => handleRunJob('custom', customScript.trim())}
              disabled={jobLoading !== null || !customScript.trim()}
              className="text-xs px-3 py-1.5 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface-input)] disabled:opacity-50 transition-colors shrink-0 flex items-center gap-1.5"
            >
              {jobLoading === customScript.trim() ? (
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <Play className="w-3.5 h-3.5" />
              )}
              {t('sites.node.executeJob')}
            </button>
          </div>
        </div>

        {/* Job Output Terminal Box */}
        {jobResult && (
          <div className="mt-4 border border-[var(--line)] rounded-[var(--radius-sm)] overflow-hidden bg-zinc-950 text-zinc-100 text-xs font-mono">
            <div className="flex items-center justify-between px-3 py-2 bg-zinc-900 border-b border-zinc-800">
              <div className="flex items-center gap-2">
                <span
                  className={`w-2 h-2 rounded-full ${
                    jobResult.success ? 'bg-emerald-400' : 'bg-red-400'
                  }`}
                />
                <span className="font-semibold">{jobResult.command}</span>
                <span className="text-zinc-400 text-[11px]">
                  (Exit: {jobResult.exit_code}, {jobResult.duration_ms}ms)
                </span>
              </div>
              <button
                onClick={copyJobOutput}
                className="text-[11px] text-zinc-400 hover:text-zinc-200 flex items-center gap-1"
              >
                {copiedStdout ? (
                  <Check className="w-3 h-3 text-emerald-400" />
                ) : (
                  <Copy className="w-3 h-3" />
                )}
                Salin
              </button>
            </div>
            <div className="p-3 max-h-60 overflow-y-auto space-y-2 whitespace-pre-wrap">
              {jobResult.stdout && <div>{jobResult.stdout}</div>}
              {jobResult.stderr && (
                <div className="text-amber-400">{jobResult.stderr}</div>
              )}
              {!jobResult.stdout && !jobResult.stderr && (
                <div className="text-zinc-500 italic">
                  (Tidak ada keluaran teks)
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
