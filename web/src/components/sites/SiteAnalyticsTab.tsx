import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { Activity, ArrowDownToLine, Clock3, HardDrive, RefreshCw, Users, SlidersHorizontal, ShieldCheck, Cpu, ListFilter, ChartNoAxesCombined } from 'lucide-react';
import { fetchSiteAnalyticsOverview, fetchSiteAnalyticsTimeseries, fetchSiteAnalyticsBreakdowns, fetchSiteAppMetrics, fetchSiteSlowlog, getSiteAnalyticsExportUrl, fetchSiteDeployments } from '../../api/client';
import type { SiteAnalyticsOverviewResponse, SiteTimeseriesResponse, SiteBreakdownsResponse, SiteAppMetricsResponse, SiteSlowlogResponse, SiteDeployment } from '../../types/api';
import { AnalyticsAreaChart } from '../analytics/AnalyticsAreaChart';
import { AnalyticsLatencyChart } from '../analytics/AnalyticsLatencyChart';
import { AnalyticsBarChart } from '../analytics/AnalyticsBarChart';
import { BreakdownTable } from '../analytics/BreakdownTable';
import { SlowlogExplorer } from '../analytics/SlowlogExplorer';
import { AppMetricsCard } from '../analytics/AppMetricsCard';
import { SiteInsightsCard } from '../analytics/SiteInsightsCard';
import { SecurityAnalyticsTab } from '../analytics/SecurityAnalyticsTab';
import { analyticsBytes, AnalyticsEmpty, AnalyticsLoading, AnalyticsMetric, AnalyticsRangePicker, useAnalyticsCopy } from '../analytics/AnalyticsUi';

export interface SiteAnalyticsTabProps { siteId: number; siteDomain: string; siteRuntime?: string }
type TimePreset = '1h' | '6h' | '24h' | '7d' | '30d' | 'custom';
type DetailTab = 'traffic' | 'breakdowns' | 'runtime' | 'slowlog' | 'security';
interface Snapshot {
  siteId: number; overview: SiteAnalyticsOverviewResponse; timeseries: SiteTimeseriesResponse; breakdowns: SiteBreakdownsResponse;
  app: SiteAppMetricsResponse | null; slowlog: SiteSlowlogResponse | null; deployments: SiteDeployment[];
  from: number; to: number; interval: string; receivedAt: number;
}
const durations = { '1h': 3600, '6h': 21600, '24h': 86400, '7d': 604800, '30d': 2592000 };
const localDate = (timestamp: number) => { const date = new Date(timestamp * 1000); return new Date(date.getTime() - date.getTimezoneOffset() * 60000).toISOString().slice(0, 16); };

export function SiteAnalyticsTab({ siteId, siteDomain, siteRuntime = 'php' }: SiteAnalyticsTabProps) {
  const { en, text } = useAnalyticsCopy();
  const id = useId();
  const [preset, setPreset] = useState<TimePreset>('24h'), [interval, setIntervalOption] = useState('auto');
  const [compare, setCompare] = useState(true), [autoRefresh, setAutoRefresh] = useState(false), [filters, setFilters] = useState(false);
  const [customRange, setCustomRange] = useState(() => { const to = Math.floor(Date.now() / 1000); return { from: to - 86400, to }; });
  const [customFrom, setCustomFrom] = useState(() => localDate(customRange.from)), [customTo, setCustomTo] = useState(() => localDate(customRange.to));
  const [data, setData] = useState<Snapshot | null>(null), [loading, setLoading] = useState(true), [refreshing, setRefreshing] = useState(false), [error, setError] = useState('');
  const [activeTab, setActiveTab] = useState<DetailTab>('traffic');
  const sequence = useRef(0), pending = useRef(false);
  const getTimeRange = useCallback(() => {
    if (preset === 'custom') return customRange;
    const to = Math.floor(Date.now() / 1000); return { from: to - durations[preset], to };
  }, [preset, customRange]);
  const loadData = useCallback(async (silent = false) => {
    if (silent && pending.current) return;
    const revision = ++sequence.current; pending.current = true;
    if (silent) setRefreshing(true); else { setLoading(true); setData(null); }
    setError('');
    const { from, to } = getTimeRange();
    try {
      const [overview, timeseries, breakdowns, app, slowlog, deployments] = await Promise.all([
        fetchSiteAnalyticsOverview(siteId, { from, to, interval, compare }), fetchSiteAnalyticsTimeseries(siteId, { from, to, interval }), fetchSiteAnalyticsBreakdowns(siteId, { from, to, limit: 15 }),
        fetchSiteAppMetrics(siteId, { from, to }).catch(() => null), siteRuntime === 'php' ? fetchSiteSlowlog(siteId, { from, to }).catch(() => null) : Promise.resolve(null), fetchSiteDeployments(siteId).catch(() => []),
      ]);
      if (revision === sequence.current) setData({ siteId, overview, timeseries, breakdowns, app, slowlog, deployments: Array.isArray(deployments) ? deployments : [], from, to, interval, receivedAt: Date.now() });
    } catch (cause) {
      if (revision === sequence.current) setError(cause instanceof Error ? cause.message : en ? 'Could not load analytics.' : 'Analitik belum dapat dimuat.');
    } finally {
      if (revision === sequence.current) { pending.current = false; setLoading(false); setRefreshing(false); }
    }
  }, [siteId, siteRuntime, getTimeRange, interval, compare, en]);
  useEffect(() => { void loadData(); return () => { sequence.current++; pending.current = false; }; }, [loadData]);
  useEffect(() => {
    if (!autoRefresh) return;
    const timer = window.setInterval(() => { if (!document.hidden) void loadData(true); }, 30000);
    return () => window.clearInterval(timer);
  }, [autoRefresh, loadData]);
  const snapshot = data?.siteId === siteId ? data : null;
  const cur = snapshot?.overview.current, comparison = compare ? snapshot?.overview.comparison : null;
  const fromDraft = Math.floor(new Date(customFrom).getTime() / 1000), toDraft = Math.floor(new Date(customTo).getTime() / 1000);
  const invalidCustom = !Number.isFinite(fromDraft) || !Number.isFinite(toDraft) || fromDraft >= toDraft;
  const tabs = [
    { value: 'traffic' as const, label: text('Trafik', 'Traffic'), Icon: ChartNoAxesCombined },
    { value: 'breakdowns' as const, label: text('Rincian', 'Breakdowns'), Icon: ListFilter },
    { value: 'runtime' as const, label: 'Runtime', Icon: Cpu },
    ...(siteRuntime === 'php' ? [{ value: 'slowlog' as const, label: 'Slowlog', Icon: Clock3 }] : []),
    { value: 'security' as const, label: text('Keamanan', 'Security'), Icon: ShieldCheck },
  ];
  return <section className="analytics-site analytics-workspace" aria-label={text('Analitik situs', 'Site analytics')}>
    <div className="analytics-toolbar">
      <AnalyticsRangePicker value={preset} onChange={setPreset} label={text('Periode analitik', 'Analytics period')} options={[
        { value: '1h', label: text('1 jam', '1 hour') }, { value: '6h', label: text('6 jam', '6 hours') }, { value: '24h', label: text('24 jam', '24 hours') }, { value: '7d', label: text('7 hari', '7 days') }, { value: '30d', label: text('30 hari', '30 days') }, { value: 'custom', label: text('Kustom', 'Custom') },
      ]} />
      <div className="analytics-actions"><button type="button" aria-expanded={filters} onClick={() => setFilters(value => !value)}><SlidersHorizontal size={15} aria-hidden="true" />{text('Opsi tampilan', 'Display options')}</button><a href={snapshot ? getSiteAnalyticsExportUrl(siteId, { from: snapshot.from, to: snapshot.to, interval: snapshot.interval }) : undefined} aria-disabled={!snapshot} download={`analytics-${siteDomain}-${preset}.csv`}><ArrowDownToLine size={15} aria-hidden="true" />CSV</a><button type="button" disabled={loading || refreshing} onClick={() => void loadData()} aria-label={text('Muat ulang analitik', 'Refresh analytics')}><RefreshCw size={15} className={refreshing ? 'animate-spin' : ''} aria-hidden="true" /></button></div>
    </div>
    {preset === 'custom' && <form className="analytics-custom-range" onSubmit={event => { event.preventDefault(); if (!invalidCustom) setCustomRange({ from: fromDraft, to: toDraft }); }}><label>{text('Mulai', 'From')}<input type="datetime-local" required value={customFrom} onChange={event => setCustomFrom(event.target.value)} /></label><label>{text('Sampai', 'To')}<input type="datetime-local" required value={customTo} onChange={event => setCustomTo(event.target.value)} /></label><button type="submit" disabled={invalidCustom}>{text('Terapkan periode', 'Apply period')}</button>{invalidCustom && <p role="alert">{text('Waktu selesai harus setelah waktu mulai.', 'The end time must be after the start time.')}</p>}</form>}
    {filters && <div className="analytics-options"><label>{text('Interval data', 'Data interval')}<select aria-label={text('Interval data', 'Data interval')} value={interval} onChange={event => setIntervalOption(event.target.value)}><option value="auto">{text('Otomatis', 'Automatic')}</option><option value="1m">{text('1 menit', '1 minute')}</option><option value="1h">{text('1 jam', '1 hour')}</option><option value="1d">{text('1 hari', '1 day')}</option></select></label><label className="analytics-toggle"><input type="checkbox" checked={compare} onChange={event => setCompare(event.target.checked)} />{text('Bandingkan periode lalu', 'Compare previous period')}</label><label className="analytics-toggle"><input type="checkbox" checked={autoRefresh} onChange={event => setAutoRefresh(event.target.checked)} />{text('Perbarui otomatis setiap 30 detik', 'Refresh every 30 seconds')}</label></div>}
    <div className="analytics-period-caption"><span>{snapshot ? `${new Date(snapshot.from * 1000).toLocaleString(en ? 'en-GB' : 'id-ID', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })} - ${new Date(snapshot.to * 1000).toLocaleString(en ? 'en-GB' : 'id-ID', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}` : text('Ringkasan periode terpilih', 'Selected period overview')}</span><span role="status">{refreshing ? text('Memperbarui...', 'Updating...') : snapshot ? `${text('Diperbarui', 'Updated')} ${new Date(snapshot.receivedAt).toLocaleTimeString(en ? 'en-GB' : 'id-ID', { hour: '2-digit', minute: '2-digit' })}` : ''}</span></div>
    {error && <div className="analytics-error" role="alert"><span>{error}{snapshot && <small>{text('Data terakhir ditampilkan. Pembaruan belum berhasil.', 'Showing the last received data. Refresh did not succeed.')}</small>}</span><button type="button" onClick={() => void loadData()}>{text('Coba lagi', 'Retry')}</button></div>}
    {loading ? <AnalyticsLoading /> : snapshot && cur ? <>
      <div className="analytics-metric-grid">
        <AnalyticsMetric label={text('Permintaan', 'Requests')} value={cur.total_requests.toLocaleString()} change={comparison?.requests_change_percent} icon={<Activity size={16} />} />
        <AnalyticsMetric label={text('Pengunjung unik', 'Unique visitors')} value={cur.unique_visitors.toLocaleString()} change={comparison?.visitors_change_percent} icon={<Users size={16} />} />
        <AnalyticsMetric label={text('Transfer data', 'Data transfer')} value={analyticsBytes(cur.total_bytes)} change={comparison?.bytes_change_percent} icon={<HardDrive size={16} />} />
        <AnalyticsMetric label={text('Waktu respons p95', 'Response time p95')} value={<>{cur.latency_p95_ms.toLocaleString()}<small>ms</small></>} hint={`p50: ${cur.latency_p50_ms} ms / p99: ${cur.latency_p99_ms} ms`} icon={<Clock3 size={16} />} />
      </div>
      <div className="analytics-health-strip"><span>{text('Tingkat error', 'Error rate')}<strong className={cur.error_rate_percent > 1 ? 'analytics-danger' : ''}>{cur.error_rate_percent.toFixed(2)}%</strong></span><span>Cache hit<strong>{Math.round(cur.cache_hit_ratio * 100)}%</strong></span><span>{text('Interval', 'Interval')}<strong>{snapshot.overview.interval_used}</strong></span></div>
      <div className="analytics-detail-tabs" role="tablist" aria-label={text('Rincian analitik', 'Analytics details')} onKeyDown={event => {
        const elements = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]'));
        const index = elements.indexOf(document.activeElement as HTMLButtonElement);
        const next = event.key === 'ArrowRight' ? (index + 1) % elements.length : event.key === 'ArrowLeft' ? (index - 1 + elements.length) % elements.length : event.key === 'Home' ? 0 : event.key === 'End' ? elements.length - 1 : -1;
        if (next >= 0) { event.preventDefault(); elements[next].focus(); elements[next].click(); }
      }}>{tabs.map(({ value, label, Icon }) => <button type="button" key={value} id={`${id}-${value}`} role="tab" aria-controls={`${id}-panel`} aria-selected={activeTab === value} tabIndex={activeTab === value ? 0 : -1} onClick={() => setActiveTab(value)}><Icon size={16} aria-hidden="true" />{label}</button>)}</div>
      <div id={`${id}-panel`} role="tabpanel" aria-labelledby={`${id}-${activeTab}`} className="analytics-tab-content">
        {activeTab === 'traffic' && <>
          {!cur.total_requests && <div className="analytics-info">{text('Belum ada permintaan pada periode ini. Ubah periode untuk menelusuri trafik sebelumnya.', 'No requests in this period. Change the period to explore earlier traffic.')}</div>}
          <AnalyticsAreaChart points={snapshot.timeseries.points} deployments={snapshot.deployments} />
          <div className="analytics-chart-grid"><AnalyticsLatencyChart points={snapshot.timeseries.points} /><AnalyticsBarChart points={snapshot.timeseries.points} /></div>
          <SiteInsightsCard key={siteId} siteId={siteId} siteDomain={siteDomain} className="analytics-insights" />
        </>}
        {activeTab === 'breakdowns' && <BreakdownTable breakdowns={snapshot.breakdowns} className="analytics-breakdowns" />}
        {activeTab === 'runtime' && (snapshot.app ? <AppMetricsCard metrics={snapshot.app} className="analytics-runtime" /> : <AnalyticsEmpty title={text('Metrik runtime belum tersedia', 'Runtime metrics unavailable')} description={text('Data runtime untuk situs ini belum dapat dimuat. Coba muat ulang analitik.', 'Runtime data could not be loaded for this site. Try refreshing analytics.')} />)}
        {activeTab === 'slowlog' && (snapshot.slowlog ? <SlowlogExplorer events={snapshot.slowlog.events} className="analytics-slowlog" /> : <AnalyticsEmpty title={text('Slowlog belum tersedia', 'Slowlog unavailable')} description={text('Sampel slowlog belum dapat dimuat untuk periode ini.', 'Slowlog samples could not be loaded for this period.')} />)}
        {activeTab === 'security' && <SecurityAnalyticsTab siteId={siteId} siteDomain={siteDomain} timeRange={{ from: snapshot.from, to: snapshot.to }} />}
      </div>
    </> : !error && <AnalyticsEmpty title={text('Belum ada data analitik', 'No analytics data yet')} />}
  </section>;
}
