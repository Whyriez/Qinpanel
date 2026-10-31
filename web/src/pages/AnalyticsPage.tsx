import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { AlertTriangle, BarChart3, CheckCircle2, ChevronDown, Globe, RefreshCw, Server, Database } from 'lucide-react';
import { useAuth } from '../context/AuthContext';
import { fetchSites, fetchSystemAnalyticsTimeseries, fetchSystemInsights } from '../api/client';
import type { Site, SystemTimeseriesResponse, SystemInsightsResponse } from '../types/api';
import { SiteAnalyticsTab } from '../components/sites/SiteAnalyticsTab';
import { ServerSystemMetrics } from '../components/analytics/ServerSystemMetrics';
import { AnalyticsRetentionCard } from '../components/analytics/AnalyticsRetentionCard';
import { summarizeSystemInsights } from '../components/analytics/systemInsights';
import { AnalyticsEmpty, AnalyticsLoading, AnalyticsRangePicker, useAnalyticsCopy } from '../components/analytics/AnalyticsUi';

export function AnalyticsPage() {
  const { user } = useAuth();
  return <AnalyticsWorkspace key={`${user?.id}:${user?.role}`} admin={user?.role === 'admin'} />;
}
function AnalyticsWorkspace({ admin }: { admin: boolean }) {
  const { en, text } = useAnalyticsCopy();
  const [scope, setScope] = useState<'site' | 'server'>('site');
  const [sites, setSites] = useState<Site[]>([]), [selectedSiteId, setSelectedSiteId] = useState<number | null>(null);
  const [loadingSites, setLoadingSites] = useState(true), [sitesError, setSitesError] = useState(''), [sitesRefresh, setSitesRefresh] = useState(0);
  const [serverMetrics, setServerMetrics] = useState<SystemTimeseriesResponse | null>(null), [systemInsights, setSystemInsights] = useState<SystemInsightsResponse | null>(null);
  const [loadingServer, setLoadingServer] = useState(true), [serverError, setServerError] = useState(''), [insightsError, setInsightsError] = useState('');
  const [serverRange, setServerRange] = useState<'1h' | '6h' | '24h' | '7d'>('24h'), [retentionOpen, setRetentionOpen] = useState(false);
  const requestSequence = useRef(0);
  useEffect(() => {
    let active = true; setLoadingSites(true); setSitesError('');
    fetchSites().then(list => {
      if (!active) return;
      setSites(list); setSelectedSiteId(previous => list.some(site => site.id === previous) ? previous : list[0]?.id ?? null);
    }).catch(cause => { if (active) { setSites([]); setSitesError(cause instanceof Error ? cause.message : en ? 'Could not load sites.' : 'Daftar situs belum dapat dimuat.'); } })
      .finally(() => { if (active) setLoadingSites(false); });
    return () => { active = false; };
  }, [sitesRefresh, en]);
  const loadServer = useCallback(async () => {
    if (!admin) return;
    const sequence = ++requestSequence.current;
    setLoadingServer(true); setServerError(''); setInsightsError(''); setServerMetrics(null); setSystemInsights(null);
    const to = Math.floor(Date.now() / 1000), duration = { '1h': 3600, '6h': 21600, '24h': 86400, '7d': 604800 }[serverRange];
    const [metrics, insights] = await Promise.allSettled([fetchSystemAnalyticsTimeseries({ from: to - duration, to }), fetchSystemInsights()]);
    if (sequence !== requestSequence.current) return;
    if (metrics.status === 'fulfilled') setServerMetrics(metrics.value); else setServerError(metrics.reason instanceof Error ? metrics.reason.message : en ? 'Could not load server metrics.' : 'Metrik server belum dapat dimuat.');
    if (insights.status === 'fulfilled') setSystemInsights(insights.value); else setInsightsError(insights.reason instanceof Error ? insights.reason.message : en ? 'Could not load system insights.' : 'Insight sistem belum dapat dimuat.');
    setLoadingServer(false);
  }, [admin, serverRange, en]);
  useEffect(() => { if (scope === 'server') void loadServer(); return () => { requestSequence.current++; }; }, [scope, loadServer]);
  const selectedSite = sites.find(site => site.id === selectedSiteId);
  const summary = useMemo(() => systemInsights ? summarizeSystemInsights(systemInsights, sites) : null, [systemInsights, sites]);
  return <div className="analytics-page analytics-workspace">
    <header className="analytics-page-header"><div className="analytics-page-title"><span className="analytics-page-symbol"><BarChart3 size={23} aria-hidden="true" /></span><div><h1>{text('Analitik', 'Analytics')}</h1><p>{scope === 'server' ? text('Pantau sumber daya server dan kondisi seluruh situs.', 'Monitor server resources and the health of your sites.') : text('Pahami trafik, pengunjung, dan performa situs Anda.', 'Understand your site traffic, visitors and performance.')}</p></div></div>
      {admin && <div className="analytics-scope" role="group" aria-label={text('Lingkup analitik', 'Analytics scope')}><button type="button" aria-pressed={scope === 'site'} onClick={() => setScope('site')}><Globe size={15} />{text('Situs', 'Sites')}</button><button type="button" aria-pressed={scope === 'server'} onClick={() => setScope('server')}><Server size={15} />{text('Server', 'Server')}</button></div>}
    </header>
    {scope === 'site' ? <>
      <div className="analytics-site-picker"><div className="analytics-site-picker-label"><Globe size={18} aria-hidden="true" /><label htmlFor="analytics-site-select">{text('Situs yang dianalisis', 'Analyzed site')}</label></div><div className="analytics-site-select"><select id="analytics-site-select" value={selectedSiteId ?? ''} disabled={loadingSites || !sites.length} onChange={event => setSelectedSiteId(Number(event.target.value))}>{!sites.length && <option value="">{loadingSites ? text('Memuat situs...', 'Loading sites...') : text('Belum ada situs', 'No sites yet')}</option>}{sites.map(site => <option key={site.id} value={site.id}>{site.primary_domain}</option>)}</select><ChevronDown size={15} aria-hidden="true" /></div>{selectedSite && <span className="analytics-runtime-tag">{selectedSite.kind === 'static' ? text('Situs statis', 'Static site') : selectedSite.kind}</span>}<button type="button" className="analytics-picker-refresh" disabled={loadingSites} aria-label={text('Muat ulang daftar situs', 'Refresh site list')} onClick={() => setSitesRefresh(value => value + 1)}><RefreshCw size={15} aria-hidden="true" /></button></div>
      {sitesError && <div role="alert" className="analytics-error"><span>{sitesError}</span><button type="button" onClick={() => setSitesRefresh(value => value + 1)}>{text('Coba lagi', 'Retry')}</button></div>}
      {loadingSites ? <AnalyticsLoading /> : selectedSite ? <SiteAnalyticsTab key={selectedSite.id} siteId={selectedSite.id} siteDomain={selectedSite.primary_domain} siteRuntime={selectedSite.kind} /> : !sitesError && <AnalyticsEmpty title={text('Belum ada situs untuk dianalisis', 'No sites to analyze yet')} description={text('Analitik akan tersedia setelah situs ditambahkan dan menerima trafik.', 'Analytics becomes available after a site is added and receives traffic.')} />}
    </> : admin && <div className="analytics-server">
      <div className="analytics-toolbar"><AnalyticsRangePicker value={serverRange} onChange={setServerRange} label={text('Periode metrik server', 'Server metrics period')} options={[{ value: '1h', label: text('1 jam', '1 hour') }, { value: '6h', label: text('6 jam', '6 hours') }, { value: '24h', label: text('24 jam', '24 hours') }, { value: '7d', label: text('7 hari', '7 days') }]} /><div className="analytics-actions"><button type="button" disabled={loadingServer} onClick={() => void loadServer()}><RefreshCw size={15} />{text('Muat ulang', 'Refresh')}</button></div></div>
      {loadingServer && <AnalyticsLoading />}
      {serverError && <div role="alert" className="analytics-error">{serverError}</div>}
      {serverMetrics && <ServerSystemMetrics points={serverMetrics.points} />}
      {insightsError && <div role="alert" className="analytics-error">{insightsError}</div>}
      {summary && <section className="analytics-server-insights"><header><div><h2>{text('Kondisi seluruh situs', 'Site health overview')}</h2><p>{text('Ringkasan hasil deteksi anomali sistem.', 'A summary of system anomaly detection.')}</p></div><span className="analytics-health-badge">{summary.total_critical + summary.total_warning > 0 ? <><AlertTriangle size={15} />{summary.total_critical} {text('kritis', 'critical')} / {summary.total_warning} {text('peringatan', 'warnings')}</> : <><CheckCircle2 size={15} />{text('Tidak ada peringatan terdeteksi', 'No warnings detected')}</>}</span></header>
        {summary.site_insights.length ? <div className="analytics-issue-list">{summary.site_insights.map(site => <article key={site.site_id ?? 'host'}><div><strong>{site.site_name}</strong><p>{site.insights.map(insight => insight.title).join(' / ')}</p></div>{site.can_review && site.site_id !== null && <button type="button" onClick={() => { setSelectedSiteId(site.site_id); setScope('site'); }}>{text('Tinjau situs', 'Review site')}</button>}</article>)}</div> : <p className="analytics-caption">{text('Tidak ada anomali yang dilaporkan pada evaluasi terakhir.', 'No anomalies were reported in the last evaluation.')}</p>}
      </section>}
      <details className="analytics-retention" open={retentionOpen} onToggle={event => setRetentionOpen(event.currentTarget.open)}><summary><Database size={17} />{text('Penyimpanan & retensi analitik', 'Analytics storage & retention')}<ChevronDown size={16} /></summary>{retentionOpen && <AnalyticsRetentionCard onRefreshParent={() => void loadServer()} />}</details>
    </div>}
  </div>;
}
