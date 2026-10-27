import { panelWebSocketUrl } from '../../lib/panelUrl';
import React, { useState, useEffect, useCallback, useRef } from 'react';
import {
  Activity,
  RefreshCw,
  Play,
  Square,
  RotateCw,
  Rocket,
  CheckCircle2,
  XCircle,
  Radio,
  Cpu,
  Clock,
  HardDrive,
  Hash,
  Terminal,
  Pause,
  Search,
  ChevronDown,
  ChevronUp,
  AlertTriangle,
} from 'lucide-react';
import {
  getNodeSiteStatus,
  postNodeSiteAction,
  getPythonSiteStatus,
  postPythonSiteAction,
  fetchSiteLogs,
} from '../../api/client';
import { ProcessItemDto } from '../../types/api';

export interface UnifiedProcessCardProps {
  siteId: number;
  runtime: 'nodejs' | 'python';
  domain?: string;
  initialProcess?: ProcessItemDto;
  onRefresh?: () => void;
}

export const UnifiedProcessCard: React.FC<UnifiedProcessCardProps> = ({
  siteId,
  runtime,
  domain,
  initialProcess,
  onRefresh,
}) => {
  const [active, setActive] = useState<boolean>(initialProcess?.active ?? false);
  const [subState, setSubState] = useState<string>(initialProcess?.sub_state ?? 'inactive');
  const [pid, setPid] = useState<number | null>(initialProcess?.pid ?? null);
  const [memoryBytes, setMemoryBytes] = useState<number | null>(
    initialProcess?.memory_bytes ?? null
  );
  const [cpuUsageUsec, setCpuUsageUsec] = useState<number | null>(
    initialProcess?.cpu_usage_usec ?? null
  );
  const [uptimeSecs, setUptimeSecs] = useState<number | null>(
    initialProcess?.uptime_secs ?? null
  );
  const [healthy, setHealthy] = useState<boolean>(initialProcess?.healthy ?? false);
  const [serviceName, setServiceName] = useState<string>(
    initialProcess?.service_name ?? `site-site_${siteId}.service`
  );
  const [portOrSocket, setPortOrSocket] = useState<string | null>(
    initialProcess?.port_or_socket ?? null
  );
  const [probeMessage, setProbeMessage] = useState<string | null>(
    initialProcess?.probe_message ?? null
  );

  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [actionLoading, setActionLoading] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Live log viewer state
  const [isLogOpen, setIsLogOpen] = useState<boolean>(false);
  const [logLines, setLogLines] = useState<string[]>([]);
  const [isLiveTailActive, setIsLiveTailActive] = useState<boolean>(true);
  const [isWsConnected, setIsWsConnected] = useState<boolean>(false);
  const [logQueryFilter, setLogQueryFilter] = useState<string>('');
  const [logLinesCount, setLogLinesCount] = useState<number>(100);
  const wsRef = useRef<WebSocket | null>(null);
  const logTerminalRef = useRef<HTMLDivElement | null>(null);

  // Live uptime increment timer
  useEffect(() => {
    if (!active || uptimeSecs === null) return;
    const interval = setInterval(() => {
      setUptimeSecs((prev) => (prev !== null ? prev + 1 : 1));
    }, 1000);
    return () => clearInterval(interval);
  }, [active, uptimeSecs]);

  // Load process status from backend
  const loadStatus = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      if (runtime === 'nodejs') {
        const data = await getNodeSiteStatus(siteId);
        setActive(data.active);
        setSubState(data.sub_state);
        setPid(data.pid);
        setMemoryBytes(data.memory_bytes);
        setCpuUsageUsec(data.cpu_usage_usec);
        setUptimeSecs(data.uptime_secs);
        setHealthy(data.healthy);
        setServiceName(data.service_name);
        setPortOrSocket(data.port ? `Port ${data.port}` : null);
        setProbeMessage(data.probe_message);
      } else {
        const data = await getPythonSiteStatus(siteId);
        setActive(data.active);
        setSubState(data.sub_state);
        setPid(data.pid);
        setMemoryBytes(data.memory_bytes);
        setCpuUsageUsec(data.cpu_usage_usec);
        setUptimeSecs(data.uptime_secs);
        setHealthy(data.healthy);
        setServiceName(data.service_name);
        setPortOrSocket(data.socket_path ?? null);
        setProbeMessage(data.probe_message);
      }
      onRefresh?.();
    } catch (err: unknown) {
      const msg =
        err instanceof Error
          ? err.message
          : 'Gagal memuat status proses aplikasi';
      setError(msg);
    } finally {
      setIsLoading(false);
    }
  }, [siteId, runtime, onRefresh]);

  useEffect(() => {
    if (!initialProcess) {
      loadStatus();
    }
    const timer = setInterval(() => {
      loadStatus();
    }, 15000);
    return () => clearInterval(timer);
  }, [loadStatus, initialProcess]);

  // Process lifecycle action handler
  const handleAction = async (action: 'start' | 'stop' | 'restart' | 'redeploy') => {
    setActionLoading(action);
    setError(null);
    try {
      if (runtime === 'nodejs') {
        await postNodeSiteAction(siteId, { action });
      } else {
        await postPythonSiteAction(siteId, { action });
      }
      await loadStatus();
    } catch (err: unknown) {
      const msg =
        err instanceof Error ? err.message : `Gagal mengeksekusi aksi ${action}`;
      setError(msg);
    } finally {
      setActionLoading(null);
    }
  };

  // Live Log streaming (WebSocket + REST fallback)
  const fetchRestLogs = useCallback(async () => {
    try {
      const res = await fetchSiteLogs(
        siteId,
        'app',
        logLinesCount,
        logQueryFilter || undefined
      );
      setLogLines(res.lines);
    } catch {
      // Ignore background fetch failure
    }
  }, [siteId, logLinesCount, logQueryFilter]);

  useEffect(() => {
    if (!isLogOpen) {
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
      return;
    }

    if (!isLiveTailActive) {
      fetchRestLogs();
      return;
    }

    let isMounted = true;
    const wsUrl = panelWebSocketUrl(`/ws/logs/${siteId}?type=app&lines=${logLinesCount}`);

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
          if (
            !logQueryFilter ||
            msg.line.toLowerCase().includes(logQueryFilter.toLowerCase())
          ) {
            setLogLines((prev) => [...prev.slice(-999), msg.line]);
          }
        }
      } catch {
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
  }, [isLogOpen, isLiveTailActive, siteId, logLinesCount, logQueryFilter, fetchRestLogs]);

  // Auto-scroll logs
  useEffect(() => {
    if (logTerminalRef.current) {
      logTerminalRef.current.scrollTop = logTerminalRef.current.scrollHeight;
    }
  }, [logLines]);

  // Format helper functions
  const formatBytes = (bytes: number | null): string => {
    if (!bytes || bytes <= 0) return '-';
    const mb = bytes / (1024 * 1024);
    if (mb < 1) return `${(bytes / 1024).toFixed(1)} KB`;
    if (mb >= 1024) return `${(mb / 1024).toFixed(2)} GB`;
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
    if (d > 0) return `${d}h ${h}j ${m}m ${s}d`;
    if (h > 0) return `${h}j ${m}m ${s}d`;
    if (m > 0) return `${m}m ${s}d`;
    return `${s}d`;
  };

  const getSubStateColor = () => {
    if (subState === 'running') return 'text-emerald-500';
    if (subState === 'failed') return 'text-rose-500';
    if (subState === 'reloading' || subState === 'activating')
      return 'text-amber-500';
    return 'text-[var(--ink-muted)]';
  };

  return (
    <div
      data-testid="unified-process-card"
      className="bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-md)] p-5 space-y-5 shadow-xs"
    >
      {/* 1. Header & Live Status Indicators */}
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--line)] pb-4">
        <div className="flex items-center gap-3">
          <div className="p-2 rounded-[var(--radius-md)] bg-[var(--raised)] border border-[var(--line)]">
            <Activity className="w-4 h-4 text-[var(--primary)]" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <h4 className="text-sm font-semibold text-[var(--ink)]">
                {domain || (runtime === 'nodejs' ? 'Aplikasi Node.js' : 'Aplikasi Python')}
              </h4>
              <span className="text-[10px] font-mono px-2 py-0.5 rounded-full font-semibold uppercase bg-[var(--raised)] border border-[var(--line)] text-[var(--ink-muted)]">
                {runtime}
              </span>
            </div>
            <span className="text-xs text-[var(--ink-muted)] font-mono block mt-0.5">
              {serviceName}
            </span>
          </div>
        </div>

        <div className="flex items-center gap-2 flex-wrap">
          {/* Active / Inactive Badge */}
          <span
            data-testid="process-status-badge"
            className={`inline-flex items-center gap-1.5 text-xs px-2.5 py-0.5 rounded-full font-medium ${
              active
                ? 'bg-emerald-100 text-emerald-800 dark:bg-emerald-950/40 dark:text-emerald-300'
                : 'bg-red-100 text-red-800 dark:bg-red-950/40 dark:text-red-300'
            }`}
          >
            {active ? (
              <>
                <CheckCircle2 className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-400" />
                <span>Aktif</span>
              </>
            ) : (
              <>
                <XCircle className="w-3.5 h-3.5 text-red-600 dark:text-red-400" />
                <span>Nonaktif</span>
              </>
            )}
          </span>

          {/* SubState Badge */}
          <span className="text-xs font-mono px-2 py-0.5 rounded bg-[var(--raised)] border border-[var(--line)]">
            <span className={getSubStateColor()}>{subState}</span>
          </span>

          {/* Health Check Badge */}
          <span
            className={`inline-flex items-center gap-1.5 text-xs px-2.5 py-0.5 rounded-full font-medium ${
              healthy
                ? 'bg-blue-100 text-blue-800 dark:bg-blue-950/40 dark:text-blue-300'
                : 'bg-amber-100 text-amber-800 dark:bg-amber-950/40 dark:text-amber-300'
            }`}
            title={probeMessage || undefined}
          >
            <Radio className="w-3.5 h-3.5 text-blue-600 dark:text-blue-400" />
            <span>{healthy ? 'Sehat' : 'Belum Siap'}</span>
          </span>

          {/* Refresh Button */}
          <button
            onClick={loadStatus}
            disabled={isLoading}
            className="p-1.5 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] transition-colors border border-transparent hover:border-[var(--line)]"
            title="Muat ulang status proses"
          >
            <RefreshCw
              className={`w-3.5 h-3.5 ${isLoading ? 'animate-spin text-[var(--primary)]' : ''}`}
            />
          </button>
        </div>
      </div>

      {/* Error Message */}
      {error && (
        <div className="p-3 bg-red-500/10 border border-red-500/20 text-red-600 dark:text-red-400 rounded-[var(--radius-md)] text-xs flex items-center gap-2">
          <AlertTriangle className="w-4 h-4 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {/* 2. Process Telemetry Grid (PID, Uptime, Memory, CPU, Target) */}
      <div className="grid grid-cols-2 sm:grid-cols-5 gap-3">
        {/* PID */}
        <div className="p-3 bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-md)]">
          <div className="flex items-center gap-1.5 text-xs text-[var(--ink-muted)] mb-1">
            <Hash className="w-3.5 h-3.5" />
            <span>PID</span>
          </div>
          <div className="text-sm font-semibold font-mono text-[var(--ink)]">
            {pid ? pid : '-'}
          </div>
        </div>

        {/* Live Uptime */}
        <div className="p-3 bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-md)]">
          <div className="flex items-center gap-1.5 text-xs text-[var(--ink-muted)] mb-1">
            <Clock className="w-3.5 h-3.5" />
            <span>Waktu Aktif</span>
          </div>
          <div
            data-testid="process-uptime"
            className="text-sm font-semibold font-mono text-[var(--ink)]"
          >
            {formatUptime(uptimeSecs)}
          </div>
        </div>

        {/* Memory */}
        <div className="p-3 bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-md)]">
          <div className="flex items-center gap-1.5 text-xs text-[var(--ink-muted)] mb-1">
            <HardDrive className="w-3.5 h-3.5" />
            <span>Memori (RAM)</span>
          </div>
          <div className="text-sm font-semibold font-mono text-[var(--ink)]">
            {formatBytes(memoryBytes)}
          </div>
        </div>

        {/* CPU */}
        <div className="p-3 bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-md)]">
          <div className="flex items-center gap-1.5 text-xs text-[var(--ink-muted)] mb-1">
            <Cpu className="w-3.5 h-3.5" />
            <span>CPU Kumulatif</span>
          </div>
          <div className="text-sm font-semibold font-mono text-[var(--ink)]">
            {formatCpu(cpuUsageUsec)}
          </div>
        </div>

        {/* Port or Socket */}
        <div className="p-3 bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-md)] col-span-2 sm:col-span-1">
          <div className="flex items-center gap-1.5 text-xs text-[var(--ink-muted)] mb-1">
            <Radio className="w-3.5 h-3.5" />
            <span>Endpoint</span>
          </div>
          <div
            className="text-xs font-semibold font-mono text-[var(--ink)] truncate"
            title={portOrSocket || '-'}
          >
            {portOrSocket || '-'}
          </div>
        </div>
      </div>

      {/* 3. Lifecycle Controls & Log Collapsible Toggle */}
      <div className="flex flex-wrap items-center justify-between gap-3 pt-2">
        <div className="flex items-center gap-2 flex-wrap">
          {!active ? (
            <button
              onClick={() => handleAction('start')}
              disabled={actionLoading !== null}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-emerald-600 hover:bg-emerald-700 text-white transition-colors disabled:opacity-50"
            >
              <Play className="w-3.5 h-3.5" />
              <span>{actionLoading === 'start' ? 'Memulai...' : 'Mulai'}</span>
            </button>
          ) : (
            <button
              onClick={() => handleAction('stop')}
              disabled={actionLoading !== null}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-rose-500/20 text-rose-600 dark:text-rose-400 hover:bg-rose-500/10 transition-colors disabled:opacity-50"
            >
              <Square className="w-3.5 h-3.5" />
              <span>{actionLoading === 'stop' ? 'Menghentikan...' : 'Berhenti'}</span>
            </button>
          )}

          <button
            onClick={() => handleAction('restart')}
            disabled={actionLoading !== null}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink)] hover:bg-[var(--card)] transition-colors disabled:opacity-50"
          >
            <RotateCw
              className={`w-3.5 h-3.5 ${actionLoading === 'restart' ? 'animate-spin text-[var(--primary)]' : ''}`}
            />
            <span>{actionLoading === 'restart' ? 'Memulai ulang...' : 'Mulai Ulang'}</span>
          </button>

          <button
            onClick={() => handleAction('redeploy')}
            disabled={actionLoading !== null}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 transition-opacity disabled:opacity-50"
          >
            <Rocket className="w-3.5 h-3.5" />
            <span>{actionLoading === 'redeploy' ? 'Mendeploy ulang...' : 'Deploy Ulang'}</span>
          </button>
        </div>

        <button
          onClick={() => setIsLogOpen((prev) => !prev)}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--card)] transition-colors"
        >
          <Terminal className="w-3.5 h-3.5 text-[var(--primary)]" />
          <span>{isLogOpen ? 'Tutup Log Aplikasi' : 'Live Log Aplikasi (Stdout/Stderr)'}</span>
          {isLogOpen ? (
            <ChevronUp className="w-3.5 h-3.5" />
          ) : (
            <ChevronDown className="w-3.5 h-3.5" />
          )}
        </button>
      </div>

      {/* 4. Collapsible Live App Log Terminal */}
      {isLogOpen && (
        <div className="border border-[var(--line)] rounded-[var(--radius-md)] bg-black/90 p-4 space-y-3 font-mono text-xs">
          <div className="flex flex-wrap items-center justify-between gap-3 text-gray-300 border-b border-white/10 pb-2">
            <div className="flex items-center gap-3">
              <span className="font-semibold text-white flex items-center gap-1.5">
                <Terminal className="w-3.5 h-3.5 text-emerald-400" />
                Live Log (Stdout & Stderr)
              </span>

              <button
                onClick={() => setIsLiveTailActive((prev) => !prev)}
                className={`px-2 py-0.5 rounded text-[11px] font-semibold flex items-center gap-1 border transition-colors ${
                  isLiveTailActive
                    ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-400'
                    : 'border-white/20 bg-white/5 text-gray-400'
                }`}
              >
                {isLiveTailActive ? (
                  <>
                    <Pause className="w-3 h-3" />
                    Jeda Aliran
                  </>
                ) : (
                  <>
                    <Play className="w-3 h-3" />
                    Lanjutkan Aliran
                  </>
                )}
              </button>

              <div className="flex items-center gap-1.5 text-[11px] text-gray-400">
                <span
                  className={`w-2 h-2 rounded-full ${
                    isWsConnected ? 'bg-emerald-400 animate-pulse' : 'bg-gray-600'
                  }`}
                />
                <span>{isWsConnected ? 'WebSocket Terhubung' : 'Terputus (REST)'}</span>
              </div>
            </div>

            <div className="flex items-center gap-2">
              <div className="relative">
                <Search className="w-3 h-3 absolute left-2 top-1/2 -translate-y-1/2 text-gray-500" />
                <input
                  type="text"
                  placeholder="Cari log..."
                  value={logQueryFilter}
                  onChange={(e) => setLogQueryFilter(e.target.value)}
                  className="pl-7 pr-2 py-0.5 text-[11px] rounded bg-white/10 border border-white/15 text-white placeholder-gray-500 focus:outline-none focus:border-emerald-500 w-36"
                />
              </div>

              <select
                value={logLinesCount}
                onChange={(e) => setLogLinesCount(Number(e.target.value))}
                className="py-0.5 px-1.5 text-[11px] rounded bg-white/10 border border-white/15 text-white focus:outline-none"
              >
                <option value={50} className="bg-gray-900">50 baris</option>
                <option value={100} className="bg-gray-900">100 baris</option>
                <option value={250} className="bg-gray-900">250 baris</option>
                <option value={500} className="bg-gray-900">500 baris</option>
              </select>

              <button
                onClick={() => setLogLines([])}
                className="px-2 py-0.5 text-[11px] rounded border border-white/15 text-gray-400 hover:text-white transition-colors"
              >
                Bersihkan
              </button>
            </div>
          </div>

          <div
            ref={logTerminalRef}
            className="h-64 overflow-y-auto space-y-1 text-[11px] select-text text-gray-200"
          >
            {logLines.length === 0 ? (
              <div className="text-gray-500 text-center py-10">
                Belum ada catatan log aplikasi (stdout/stderr) yang terekam.
              </div>
            ) : (
              logLines.map((line, idx) => (
                <div key={idx} className="hover:bg-white/5 py-0.5 px-1 rounded break-all">
                  <span className="text-gray-500 select-none mr-2 font-mono text-[10px]">
                    {idx + 1}
                  </span>
                  <span>{line}</span>
                </div>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
};
