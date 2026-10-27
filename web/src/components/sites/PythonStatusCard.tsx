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
  Radio,
} from 'lucide-react';
import {
  getPythonSiteStatus,
  postPythonSiteAction,
  runPythonSiteJob,
} from '../../api/client';
import {
  PythonSiteStatusApiResponse,
  PythonSiteJobApiResponse,
} from '../../types/api';

export interface PythonStatusCardProps {
  siteId: number;
}

export const PythonStatusCard: React.FC<PythonStatusCardProps> = ({ siteId }) => {
  const [status, setStatus] = useState<PythonSiteStatusApiResponse | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [jobLoading, setJobLoading] = useState<string | null>(null);
  const [customScript, setCustomScript] = useState<string>('');
  const [jobResult, setJobResult] = useState<PythonSiteJobApiResponse | null>(null);
  const [copiedStdout, setCopiedStdout] = useState(false);

  const loadStatus = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const data = await getPythonSiteStatus(siteId);
      setStatus(data);
    } catch (err: unknown) {
      const msg =
        err instanceof Error ? err.message : 'Gagal memuat status proses Python';
      setError(msg);
    } finally {
      setIsLoading(false);
    }
  }, [siteId]);

  useEffect(() => {
    loadStatus();
    const timer = setInterval(() => {
      // Background poll status every 10 seconds
      getPythonSiteStatus(siteId)
        .then((data) => setStatus(data))
        .catch(() => {});
    }, 10000);
    return () => clearInterval(timer);
  }, [siteId, loadStatus]);

  const handleAction = async (action: 'start' | 'stop' | 'restart' | 'redeploy') => {
    setActionLoading(action);
    setError(null);
    try {
      await postPythonSiteAction(siteId, { action });
      await loadStatus();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : `Gagal mengeksekusi aksi ${action}`;
      setError(msg);
    } finally {
      setActionLoading(null);
    }
  };

  const handleRunJob = async (jobType: string, reqFile?: string, scriptName?: string) => {
    setJobLoading(scriptName || reqFile || jobType);
    setError(null);
    try {
      const res = await runPythonSiteJob(siteId, {
        job_type: jobType,
        req_file: reqFile,
        script_name: scriptName,
      });
      setJobResult(res);
      await loadStatus();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal mengeksekusi perintah Python/uv';
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
    if (secs < 60) return `${secs} detik`;
    if (secs < 3600) return `${Math.floor(secs / 60)} menit ${secs % 60} dtk`;
    const hours = Math.floor(secs / 3600);
    const mins = Math.floor((secs % 3600) / 60);
    return `${hours} jam ${mins} menit`;
  };

  const copyToClipboard = (text: string) => {
    navigator.clipboard.writeText(text);
    setCopiedStdout(true);
    setTimeout(() => setCopiedStdout(false), 2000);
  };

  return (
    <div className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] p-5 shadow-xs space-y-6">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 border-b border-[var(--line)] pb-4">
        <div>
          <div className="flex items-center gap-2">
            <Activity className="w-5 h-5 text-[var(--primary)]" />
            <h2 className="text-base font-semibold text-[var(--ink)]">
              Status Layanan Python (uv + venv)
            </h2>
          </div>
          <p className="text-xs text-[var(--ink-muted)] mt-1">
            Status systemd unit WSGI/ASGI, pemantauan soket Unix, dan eksekusi tenant job aman
          </p>
        </div>

        <button
          onClick={loadStatus}
          disabled={isLoading}
          className="self-start sm:self-auto inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] hover:bg-[var(--line)] transition-colors disabled:opacity-50"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? 'animate-spin' : ''}`} />
          Segarkan
        </button>
      </div>

      {/* Global Error Banner */}
      {error && (
        <div className="p-3 bg-[var(--danger)]/10 border border-[var(--danger)]/20 rounded-[var(--radius-md)] text-xs text-[var(--danger)] flex items-start gap-2">
          <AlertTriangle className="w-4 h-4 shrink-0 mt-0.5" />
          <span>{error}</span>
        </div>
      )}

      {/* Service Status Overview Grid */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3.5">
        {/* Process Active State */}
        <div className="p-3.5 bg-[var(--raised)] rounded-[var(--radius-md)] border border-[var(--line)] flex flex-col justify-between">
          <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-2">
            <span>Status Layanan</span>
            <Activity className="w-4 h-4 text-[var(--primary)]" />
          </div>
          <div className="flex items-center gap-2">
            {isLoading ? (
              <Loader2 className="w-4 h-4 animate-spin text-[var(--ink-muted)]" />
            ) : status?.active ? (
              <>
                <span className="w-2.5 h-2.5 rounded-full bg-[var(--success)] animate-pulse" />
                <span className="text-sm font-semibold text-[var(--success)]">
                  Aktif ({status.sub_state || 'running'})
                </span>
              </>
            ) : (
              <>
                <span className="w-2.5 h-2.5 rounded-full bg-[var(--danger)]" />
                <span className="text-sm font-semibold text-[var(--danger)]">
                  Nonaktif ({status?.sub_state || 'dead'})
                </span>
              </>
            )}
          </div>
          <div className="text-[11px] text-[var(--ink-muted)] mt-2 font-mono truncate">
            {status?.service_name || `panel-py-site-${siteId}.service`}
          </div>
        </div>

        {/* Unix Socket Health */}
        <div className="p-3.5 bg-[var(--raised)] rounded-[var(--radius-md)] border border-[var(--line)] flex flex-col justify-between">
          <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-2">
            <span>Soket Unix</span>
            <Radio className="w-4 h-4 text-[var(--primary)]" />
          </div>
          <div className="flex items-center gap-2">
            {isLoading ? (
              <Loader2 className="w-4 h-4 animate-spin text-[var(--ink-muted)]" />
            ) : status?.socket_exists ? (
              <>
                <CheckCircle2 className="w-4 h-4 text-[var(--success)]" />
                <span className="text-sm font-semibold text-[var(--ink)]">
                  Soket Terbuka
                </span>
              </>
            ) : (
              <>
                <XCircle className="w-4 h-4 text-[var(--danger)]" />
                <span className="text-sm font-semibold text-[var(--danger)]">
                  Soket Tidak Ditemukan
                </span>
              </>
            )}
          </div>
          <div className="text-[11px] text-[var(--ink-muted)] mt-2 font-mono truncate" title={status?.socket_path || ''}>
            {status?.socket_path || `/run/panel/python/site-${siteId}.sock`}
          </div>
        </div>

        {/* PID & Uptime */}
        <div className="p-3.5 bg-[var(--raised)] rounded-[var(--radius-md)] border border-[var(--line)] flex flex-col justify-between">
          <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-2">
            <span>PID / Waktu Nyala</span>
            <Clock className="w-4 h-4 text-[var(--primary)]" />
          </div>
          <div className="flex items-baseline gap-2">
            <span className="text-sm font-mono font-semibold text-[var(--ink)]">
              PID: {status?.pid || '-'}
            </span>
          </div>
          <div className="text-[11px] text-[var(--ink-muted)] mt-2 flex items-center gap-1">
            <span>Nyala:</span>
            <span className="font-semibold text-[var(--ink)]">
              {formatUptime(status?.uptime_secs ?? null)}
            </span>
          </div>
        </div>

        {/* Memory & CPU */}
        <div className="p-3.5 bg-[var(--raised)] rounded-[var(--radius-md)] border border-[var(--line)] flex flex-col justify-between">
          <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-2">
            <span>Konsumsi Sumber Daya</span>
            <Cpu className="w-4 h-4 text-[var(--primary)]" />
          </div>
          <div className="flex items-baseline justify-between">
            <div>
              <span className="text-[11px] text-[var(--ink-muted)] block">RAM:</span>
              <span className="text-sm font-mono font-semibold text-[var(--ink)]">
                {formatBytes(status?.memory_bytes ?? null)}
              </span>
            </div>
            <div>
              <span className="text-[11px] text-[var(--ink-muted)] block">CPU:</span>
              <span className="text-sm font-mono font-semibold text-[var(--ink)]">
                {formatCpu(status?.cpu_usage_usec ?? null)}
              </span>
            </div>
          </div>
          <div className="text-[11px] text-[var(--ink-muted)] mt-2">
            Batas cgroup tenant slice
          </div>
        </div>
      </div>

      {/* Lifecycle Action Buttons */}
      <div className="flex flex-wrap items-center gap-2 pt-2 border-t border-[var(--line)]">
        <span className="text-xs font-semibold text-[var(--ink-muted)] mr-2">
          Kendali Proses:
        </span>

        {/* Start */}
        <button
          onClick={() => handleAction('start')}
          disabled={actionLoading !== null || status?.active}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] bg-[var(--success)] text-white hover:opacity-90 disabled:opacity-40 transition-opacity"
        >
          {actionLoading === 'start' ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <Play className="w-3.5 h-3.5 fill-current" />
          )}
          Mulai
        </button>

        {/* Restart */}
        <button
          onClick={() => handleAction('restart')}
          disabled={actionLoading !== null}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] hover:bg-[var(--line)] disabled:opacity-40 transition-colors"
        >
          {actionLoading === 'restart' ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <RotateCw className="w-3.5 h-3.5" />
          )}
          Muat Ulang
        </button>

        {/* Redeploy */}
        <button
          onClick={() => handleAction('redeploy')}
          disabled={actionLoading !== null}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] hover:bg-[var(--line)] disabled:opacity-40 transition-colors"
          title="Terapkan ulang file unit systemd dan restart"
        >
          {actionLoading === 'redeploy' ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <RefreshCw className="w-3.5 h-3.5" />
          )}
          Deploy Ulang
        </button>

        {/* Stop */}
        <button
          onClick={() => handleAction('stop')}
          disabled={actionLoading !== null || !status?.active}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] bg-[var(--danger)] text-white hover:opacity-90 disabled:opacity-40 transition-opacity ml-auto"
        >
          {actionLoading === 'stop' ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <Square className="w-3.5 h-3.5 fill-current" />
          )}
          Hentikan
        </button>
      </div>

      {/* Tenant Job Execution (uv / pip) */}
      <div className="pt-4 border-t border-[var(--line)] space-y-3">
        <div>
          <div className="flex items-center gap-2">
            <Package className="w-4 h-4 text-[var(--primary)]" />
            <h3 className="text-xs font-semibold text-[var(--ink)]">
              Eksekusi Perintah Tenant (uv &amp; Python)
            </h3>
          </div>
          <p className="text-[11px] text-[var(--ink-muted)] mt-0.5">
            Perintah dijalankan aman di bawah UID tenant pada cgroup slice tenant tanpa hak root
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          {/* uv venv */}
          <button
            onClick={() => handleRunJob('uv_venv')}
            disabled={jobLoading !== null}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] hover:border-[var(--primary)] disabled:opacity-50 transition-colors"
          >
            {jobLoading === 'uv_venv' ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Wrench className="w-3.5 h-3.5" />
            )}
            uv venv (Buat Virtualenv)
          </button>

          {/* uv pip install -r requirements.txt */}
          <button
            onClick={() => handleRunJob('uv_pip_install', 'requirements.txt')}
            disabled={jobLoading !== null}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] hover:border-[var(--primary)] disabled:opacity-50 transition-colors"
          >
            {jobLoading === 'requirements.txt' ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Package className="w-3.5 h-3.5" />
            )}
            uv pip install -r requirements.txt
          </button>

          {/* uv pip install pyproject.toml */}
          <button
            onClick={() => handleRunJob('uv_pip_pyproject')}
            disabled={jobLoading !== null}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] hover:border-[var(--primary)] disabled:opacity-50 transition-colors"
          >
            {jobLoading === 'uv_pip_pyproject' ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Terminal className="w-3.5 h-3.5" />
            )}
            uv pip install -e . (pyproject.toml)
          </button>
        </div>

        {/* Custom Script Runner */}
        <div className="flex items-center gap-2 pt-1">
          <input
            type="text"
            placeholder="Jalankan script Python atau manage.py (contoh: manage.py migrate)"
            value={customScript}
            onChange={(e) => setCustomScript(e.target.value)}
            disabled={jobLoading !== null}
            className="flex-1 text-xs px-3 py-1.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] placeholder:text-[var(--ink-muted)] focus:outline-none focus:border-[var(--primary)]"
          />
          <button
            onClick={() => handleRunJob('custom', undefined, customScript.trim())}
            disabled={jobLoading !== null || !customScript.trim()}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 disabled:opacity-50 transition-opacity"
          >
            {jobLoading === customScript.trim() ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Terminal className="w-3.5 h-3.5" />
            )}
            Jalankan Script
          </button>
        </div>

        {/* Execution Output Box */}
        {jobResult && (
          <div className="mt-3 p-3 bg-neutral-950 text-neutral-200 rounded-[var(--radius-md)] border border-neutral-800 font-mono text-xs space-y-2">
            <div className="flex items-center justify-between border-b border-neutral-800 pb-2 text-[11px] text-neutral-400">
              <div className="flex items-center gap-2">
                <span>Perintah:</span>
                <span className="text-emerald-400 font-semibold">{jobResult.command}</span>
              </div>
              <div className="flex items-center gap-3">
                <span>Durasi: {jobResult.duration_ms} ms</span>
                <span
                  className={
                    jobResult.success ? 'text-emerald-400' : 'text-rose-400'
                  }
                >
                  Exit Code: {jobResult.exit_code}
                </span>
                <button
                  onClick={() =>
                    copyToClipboard(
                      `${jobResult.stdout}\n${jobResult.stderr}`.trim()
                    )
                  }
                  className="p-1 hover:text-white transition-colors"
                  title="Salin Output"
                >
                  {copiedStdout ? (
                    <Check className="w-3.5 h-3.5 text-emerald-400" />
                  ) : (
                    <Copy className="w-3.5 h-3.5" />
                  )}
                </button>
              </div>
            </div>

            {jobResult.stdout && (
              <pre className="whitespace-pre-wrap max-h-48 overflow-y-auto text-neutral-300">
                {jobResult.stdout}
              </pre>
            )}

            {jobResult.stderr && (
              <pre className="whitespace-pre-wrap max-h-48 overflow-y-auto text-rose-300">
                {jobResult.stderr}
              </pre>
            )}
          </div>
        )}
      </div>
    </div>
  );
};
