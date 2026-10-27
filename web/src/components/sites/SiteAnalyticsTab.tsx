import React, { useState, useEffect, useCallback } from 'react';
import {
  RefreshCw,
  Download,
  Clock,
  TrendingUp,
  TrendingDown,
  Activity,
  Layers,
  Zap,
} from 'lucide-react';
import {
  fetchSiteAnalyticsOverview,
  fetchSiteAnalyticsTimeseries,
  fetchSiteAnalyticsBreakdowns,
  fetchSiteAppMetrics,
  fetchSiteSlowlog,
  getSiteAnalyticsExportUrl,
  fetchSiteDeployments,
} from '../../api/client';
import {
  SiteAnalyticsOverviewResponse,
  SiteTimeseriesResponse,
  SiteBreakdownsResponse,
  SiteAppMetricsResponse,
  SiteSlowlogResponse,
  SiteDeployment,
} from '../../types/api';
import { AnalyticsAreaChart } from '../analytics/AnalyticsAreaChart';
import { AnalyticsLatencyChart } from '../analytics/AnalyticsLatencyChart';
import { AnalyticsBarChart } from '../analytics/AnalyticsBarChart';
import { BreakdownTable } from '../analytics/BreakdownTable';
import { SlowlogExplorer } from '../analytics/SlowlogExplorer';
import { AppMetricsCard } from '../analytics/AppMetricsCard';
import { SiteInsightsCard } from '../analytics/SiteInsightsCard';
import { SecurityAnalyticsTab } from '../analytics/SecurityAnalyticsTab';

export interface SiteAnalyticsTabProps {
  siteId: number;
  siteDomain: string;
  siteRuntime?: string;
}

type TimePreset = '1h' | '6h' | '24h' | '7d' | '30d' | 'custom';

function formatBytes(bytes: number): string {
  if (!bytes || bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

export const SiteAnalyticsTab: React.FC<SiteAnalyticsTabProps> = ({
  siteId,
  siteDomain,
  siteRuntime = 'php',
}) => {
  // Filters
  const [preset, setPreset] = useState<TimePreset>('24h');
  const [interval, setIntervalOption] = useState<string>('auto');
  const [compare, setCompare] = useState<boolean>(true);
  const [autoRefresh, setAutoRefresh] = useState<boolean>(false);

  // Custom date range
  const [customFrom, setCustomFrom] = useState<string>('');
  const [customTo, setCustomTo] = useState<string>('');

  // Data States
  const [overview, setOverview] = useState<SiteAnalyticsOverviewResponse | null>(null);
  const [timeseries, setTimeseries] = useState<SiteTimeseriesResponse | null>(null);
  const [breakdowns, setBreakdowns] = useState<SiteBreakdownsResponse | null>(null);
  const [appMetrics, setAppMetrics] = useState<SiteAppMetricsResponse | null>(null);
  const [slowlog, setSlowlog] = useState<SiteSlowlogResponse | null>(null);
  const [deployments, setDeployments] = useState<SiteDeployment[]>([]);

  // UI State
  const [loading, setLoading] = useState<boolean>(true);
  const [refreshing, setRefreshing] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [activeSubTab, setActiveSubTab] = useState<'traffic' | 'breakdowns' | 'runtime' | 'slowlog' | 'security'>('traffic');

  // Compute timestamps based on preset
  const getTimeRange = useCallback(() => {
    const now = Math.floor(Date.now() / 1000);
    let from = now - 24 * 3600;

    if (preset === '1h') from = now - 3600;
    else if (preset === '6h') from = now - 6 * 3600;
    else if (preset === '24h') from = now - 24 * 3600;
    else if (preset === '7d') from = now - 7 * 24 * 3600;
    else if (preset === '30d') from = now - 30 * 24 * 3600;
    else if (preset === 'custom' && customFrom && customTo) {
      from = Math.floor(new Date(customFrom).getTime() / 1000);
      const to = Math.floor(new Date(customTo).getTime() / 1000);
      return { from, to };
    }

    return { from, to: now };
  }, [preset, customFrom, customTo]);

  const loadData = useCallback(async (isSilent = false) => {
    if (!isSilent) setLoading(true);
    else setRefreshing(true);
    setError(null);

    const { from, to } = getTimeRange();

    try {
      const [ovRes, tsRes, bdRes, apRes, slRes] = await Promise.all([
        fetchSiteAnalyticsOverview(siteId, { from, to, interval, compare }),
        fetchSiteAnalyticsTimeseries(siteId, { from, to, interval }),
        fetchSiteAnalyticsBreakdowns(siteId, { from, to, limit: 15 }),
        fetchSiteAppMetrics(siteId, { from, to }).catch(() => null),
        siteRuntime === 'php' ? fetchSiteSlowlog(siteId, { from, to }).catch(() => null) : Promise.resolve(null),
      ]);

      setOverview(ovRes);
      setTimeseries(tsRes);
      setBreakdowns(bdRes);
      setAppMetrics(apRes);
      setSlowlog(slRes);

      // Fetch deployments for deploy markers
      try {
        const depRes = await fetchSiteDeployments(siteId);
        setDeployments(Array.isArray(depRes) ? depRes : []);
      } catch {
        // ignore deployment fetch error
      }
    } catch (err: unknown) {
      const e = err as Error;
      setError(e.message || 'Gagal memuat data analitik situs');
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [siteId, siteRuntime, getTimeRange, interval, compare]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Auto refresh interval
  useEffect(() => {
    if (!autoRefresh) return;
    const intervalTimer = setInterval(() => {
      loadData(true);
    }, 30000);
    return () => clearInterval(intervalTimer);
  }, [autoRefresh, loadData]);

  const { from, to } = getTimeRange();
  const exportUrl = getSiteAnalyticsExportUrl(siteId, { from, to, interval });

  const cur = overview?.current;
  const comp = overview?.comparison;

  return (
    <div className="space-y-6">
      {/* Control Bar: Range Preset, Interval, Compare, Export, Refresh */}
      <div className="flex flex-col lg:flex-row lg:items-center justify-between gap-4 p-4 bg-[var(--surface)] border border-[var(--line)] rounded-xl shadow-sm">
        {/* Presets */}
        <div className="flex flex-wrap items-center gap-1.5">
          {(['1h', '6h', '24h', '7d', '30d', 'custom'] as TimePreset[]).map((p) => (
            <button
              key={p}
              type="button"
              onClick={() => setPreset(p)}
              className={`px-3 py-1.5 text-xs font-semibold rounded-lg transition-colors ${
                preset === p
                  ? 'bg-[var(--primary)] text-white shadow-xs'
                  : 'bg-[var(--bg)] border border-[var(--line)] text-[var(--ink-muted)] hover:text-[var(--ink)]'
              }`}
            >
              {p === '1h' && '1 Jam'}
              {p === '6h' && '6 Jam'}
              {p === '24h' && '24 Jam'}
              {p === '7d' && '7 Hari'}
              {p === '30d' && '30 Hari'}
              {p === 'custom' && 'Kustom'}
            </button>
          ))}

          {/* Custom Date Inputs */}
          {preset === 'custom' && (
            <div className="flex items-center gap-1.5 ml-2">
              <input
                type="datetime-local"
                value={customFrom}
                onChange={(e) => setCustomFrom(e.target.value)}
                className="px-2 py-1 text-xs bg-[var(--bg)] border border-[var(--line)] rounded text-[var(--ink)]"
                title="Waktu Mulai"
              />
              <span className="text-xs text-[var(--ink-muted)]">-</span>
              <input
                type="datetime-local"
                value={customTo}
                onChange={(e) => setCustomTo(e.target.value)}
                className="px-2 py-1 text-xs bg-[var(--bg)] border border-[var(--line)] rounded text-[var(--ink)]"
                title="Waktu Selesai"
              />
            </div>
          )}

          {/* Interval */}
          <select
            value={interval}
            onChange={(e) => setIntervalOption(e.target.value)}
            className="ml-2 px-2.5 py-1.5 text-xs bg-[var(--bg)] border border-[var(--line)] rounded-lg text-[var(--ink)] focus:outline-none"
            title="Interval agregasi waktu"
          >
            <option value="auto">Interval: Otomatis</option>
            <option value="1m">Interval: 1 Menit</option>
            <option value="1h">Interval: 1 Jam</option>
            <option value="1d">Interval: 1 Hari</option>
          </select>
        </div>

        {/* Action Controls */}
        <div className="flex flex-wrap items-center gap-3">
          {/* Compare toggle */}
          <label className="flex items-center gap-2 text-xs text-[var(--ink-muted)] cursor-pointer select-none">
            <input
              type="checkbox"
              checked={compare}
              onChange={(e) => setCompare(e.target.checked)}
              className="rounded border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
            />
            <span>Bandingkan Periode Lalu</span>
          </label>

          {/* Auto refresh toggle */}
          <label className="flex items-center gap-2 text-xs text-[var(--ink-muted)] cursor-pointer select-none">
            <input
              type="checkbox"
              checked={autoRefresh}
              onChange={(e) => setAutoRefresh(e.target.checked)}
              className="rounded border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)]"
            />
            <span>Auto-Refresh (30s)</span>
          </label>

          {/* Export CSV */}
          <a
            href={exportUrl}
            download={`analytics-${siteDomain}-${preset}.csv`}
            className="px-3 py-1.5 text-xs font-semibold rounded-lg border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] hover:bg-[var(--surface-hover)] transition-colors flex items-center gap-1.5"
            title="Ekspor CSV"
          >
            <Download className="w-3.5 h-3.5 text-[var(--ink-muted)]" />
            <span>Ekspor CSV</span>
          </a>

          {/* Manual Refresh */}
          <button
            type="button"
            onClick={() => loadData(false)}
            disabled={loading || refreshing}
            className="p-1.5 rounded-lg border border-[var(--line)] bg-[var(--bg)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface-hover)] transition-colors"
            title="Perbarui Data"
          >
            <RefreshCw className={`w-4 h-4 ${refreshing ? 'animate-spin text-[var(--primary)]' : ''}`} />
          </button>
        </div>
      </div>

      {/* Error Banner */}
      {error && (
        <div className="p-4 bg-rose-500/10 border border-rose-500/20 rounded-xl text-xs text-rose-500 flex items-center justify-between">
          <span>{error}</span>
          <button
            type="button"
            onClick={() => loadData(false)}
            className="font-semibold underline hover:no-underline"
          >
            Coba lagi
          </button>
        </div>
      )}

      {/* Loading Skeleton */}
      {loading && !overview ? (
        <div className="space-y-4 animate-pulse">
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
            {[1, 2, 3, 4].map((i) => (
              <div key={i} className="h-24 bg-[var(--surface)] border border-[var(--line)] rounded-xl" />
            ))}
          </div>
          <div className="h-64 bg-[var(--surface)] border border-[var(--line)] rounded-xl" />
        </div>
      ) : (
        <>
          {/* Anomaly & Insight Engine (Phase 9 - P9-T7) */}
          <SiteInsightsCard siteId={siteId} siteDomain={siteDomain} />

          {/* KPI Summary Cards */}
          {cur && (
            <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
              {/* Total Requests */}
              <div className="p-4 bg-[var(--surface)] border border-[var(--line)] rounded-xl shadow-xs">
                <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
                  <span>Total Permintaan</span>
                  <Activity className="w-4 h-4 text-blue-500" />
                </div>
                <div className="text-2xl font-bold font-mono text-[var(--ink)]">
                  {cur.total_requests.toLocaleString()}
                </div>
                {comp && (
                  <div className="flex items-center gap-1.5 mt-2 text-xs">
                    {comp.requests_change_percent >= 0 ? (
                      <span className="flex items-center gap-0.5 text-emerald-500 font-medium">
                        <TrendingUp className="w-3.5 h-3.5" /> +{comp.requests_change_percent.toFixed(1)}%
                      </span>
                    ) : (
                      <span className="flex items-center gap-0.5 text-rose-500 font-medium">
                        <TrendingDown className="w-3.5 h-3.5" /> {comp.requests_change_percent.toFixed(1)}%
                      </span>
                    )}
                    <span className="text-[var(--ink-muted)]">vs periode lalu</span>
                  </div>
                )}
              </div>

              {/* Total Bandwidth */}
              <div className="p-4 bg-[var(--surface)] border border-[var(--line)] rounded-xl shadow-xs">
                <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
                  <span>Bandwidth (Egress)</span>
                  <Zap className="w-4 h-4 text-amber-500" />
                </div>
                <div className="text-2xl font-bold font-mono text-[var(--ink)]">
                  {formatBytes(cur.total_bytes)}
                </div>
                {comp && (
                  <div className="flex items-center gap-1.5 mt-2 text-xs">
                    {comp.bytes_change_percent >= 0 ? (
                      <span className="flex items-center gap-0.5 text-emerald-500 font-medium">
                        <TrendingUp className="w-3.5 h-3.5" /> +{comp.bytes_change_percent.toFixed(1)}%
                      </span>
                    ) : (
                      <span className="flex items-center gap-0.5 text-rose-500 font-medium">
                        <TrendingDown className="w-3.5 h-3.5" /> {comp.bytes_change_percent.toFixed(1)}%
                      </span>
                    )}
                    <span className="text-[var(--ink-muted)]">vs periode lalu</span>
                  </div>
                )}
              </div>

              {/* Unique Visitors */}
              <div className="p-4 bg-[var(--surface)] border border-[var(--line)] rounded-xl shadow-xs">
                <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
                  <span>Pengunjung Unik</span>
                  <Layers className="w-4 h-4 text-purple-500" />
                </div>
                <div className="text-2xl font-bold font-mono text-[var(--ink)]">
                  {cur.unique_visitors.toLocaleString()}
                </div>
                <div className="flex items-center gap-2 mt-2 text-xs text-[var(--ink-muted)]">
                  <span>Cache Hit:</span>
                  <span className="font-semibold text-emerald-500 font-mono">
                    {Math.round(cur.cache_hit_ratio * 100)}%
                  </span>
                </div>
              </div>

              {/* Latency & Error Rate */}
              <div className="p-4 bg-[var(--surface)] border border-[var(--line)] rounded-xl shadow-xs">
                <div className="flex items-center justify-between text-xs text-[var(--ink-muted)] mb-1">
                  <span>Latensi & Error Rate</span>
                  <Clock className="w-4 h-4 text-emerald-500" />
                </div>
                <div className="flex items-baseline gap-2">
                  <span className="text-2xl font-bold font-mono text-[var(--ink)]">
                    {cur.latency_p50_ms} ms
                  </span>
                  <span className="text-xs text-[var(--ink-muted)] font-mono">
                    (p95: {cur.latency_p95_ms}ms)
                  </span>
                </div>
                <div className="flex items-center gap-2 mt-2 text-xs">
                  <span className="text-[var(--ink-muted)]">Tingkat Error:</span>
                  <span
                    className={`font-semibold font-mono ${
                      cur.error_rate_percent > 1 ? 'text-rose-500' : 'text-emerald-500'
                    }`}
                  >
                    {cur.error_rate_percent.toFixed(2)}%
                  </span>
                  <span className="text-[10px] text-[var(--ink-muted)]">
                    ({cur.status_4xx + cur.status_5xx} err)
                  </span>
                </div>
              </div>
            </div>
          )}

          {/* Sub Navigation: Traffic & Latency, Breakdown, App Runtime, Slowlog */}
          <div className="flex items-center gap-2 border-b border-[var(--line)] text-xs font-medium">
            <button
              type="button"
              onClick={() => setActiveSubTab('traffic')}
              className={`px-4 py-2 border-b-2 transition-colors ${
                activeSubTab === 'traffic'
                  ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                  : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
              }`}
            >
              Grafik Lalu Lintas & Latensi
            </button>
            <button
              type="button"
              onClick={() => setActiveSubTab('breakdowns')}
              className={`px-4 py-2 border-b-2 transition-colors ${
                activeSubTab === 'breakdowns'
                  ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                  : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
              }`}
            >
              Rincian Dimensi (8 Dimensi)
            </button>
            <button
              type="button"
              onClick={() => setActiveSubTab('runtime')}
              className={`px-4 py-2 border-b-2 transition-colors ${
                activeSubTab === 'runtime'
                  ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                  : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
              }`}
            >
              Metrik Runtime & Cgroup
            </button>
            {siteRuntime === 'php' && (
              <button
                type="button"
                onClick={() => setActiveSubTab('slowlog')}
                className={`px-4 py-2 border-b-2 transition-colors ${
                  activeSubTab === 'slowlog'
                    ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                    : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
                }`}
              >
                PHP Slowlog Explorer
                {slowlog && slowlog.events.length > 0 && (
                  <span className="ml-1.5 px-1.5 py-0.2 rounded-full bg-amber-500/20 text-amber-500 text-[10px]">
                    {slowlog.events.length}
                  </span>
                )}
              </button>
            )}
            <button
              type="button"
              onClick={() => setActiveSubTab('security')}
              className={`px-4 py-2 border-b-2 transition-colors ${
                activeSubTab === 'security'
                  ? 'border-[var(--primary)] text-[var(--primary)] font-semibold'
                  : 'border-transparent text-[var(--ink-muted)] hover:text-[var(--ink)]'
              }`}
            >
              Keamanan & Proteksi IP
            </button>
          </div>

          {/* Sub Tab: Traffic & Latency */}
          {activeSubTab === 'traffic' && timeseries && (
            <div className="space-y-6">
              {/* Traffic Area Chart with Deploy Markers */}
              <AnalyticsAreaChart
                points={timeseries.points}
                deployments={deployments}
                height={260}
                metric="requests"
                title="Lalu Lintas Permintaan (HTTP Requests) & Penanda Rilis Deploy"
              />

              {/* 2 Grid: Latency Chart & Errors Bar Chart */}
              <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
                <AnalyticsLatencyChart
                  points={timeseries.points}
                  height={220}
                  title="Latensi Respons (p50 / p95 / p99)"
                />
                <AnalyticsBarChart
                  points={timeseries.points}
                  height={220}
                  title="Distribusi Kode Error (4xx & 5xx)"
                />
              </div>
            </div>
          )}

          {/* Sub Tab: Breakdowns */}
          {activeSubTab === 'breakdowns' && (
            <BreakdownTable breakdowns={breakdowns} />
          )}

          {/* Sub Tab: Runtime & Cgroup */}
          {activeSubTab === 'runtime' && (
            <AppMetricsCard metrics={appMetrics} />
          )}

          {/* Sub Tab: Slowlog */}
          {activeSubTab === 'slowlog' && (
            <SlowlogExplorer events={slowlog?.events || []} />
          )}

          {/* Sub Tab: Security Analytics */}
          {activeSubTab === 'security' && (
            <SecurityAnalyticsTab
              siteId={siteId}
              siteDomain={siteDomain}
              timeRange={getTimeRange()}
            />
          )}
        </>
      )}
    </div>
  );
};
