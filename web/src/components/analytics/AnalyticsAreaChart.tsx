import type { SiteTimeseriesPointDto, SiteDeployment } from '../../types/api';
import { AnalyticsPlot } from './AnalyticsPlot';
import { analyticsBytes, useAnalyticsCopy } from './AnalyticsUi';
export interface AnalyticsAreaChartProps { points: SiteTimeseriesPointDto[]; deployments?: SiteDeployment[]; height?: number; metric?: 'requests' | 'bytes' | 'visitors'; title?: string; className?: string }
export function AnalyticsAreaChart({ points, deployments = [], height = 270, metric = 'requests', title, className }: AnalyticsAreaChartProps) {
  const { text } = useAnalyticsCopy();
  const label = metric === 'bytes' ? text('Transfer data', 'Data transfer') : metric === 'visitors' ? text('Pengunjung', 'Visitors') : text('Permintaan', 'Requests');
  const values = points.map(point => metric === 'bytes' ? point.bytes : metric === 'visitors' ? point.visitors : point.requests);
  return <AnalyticsPlot timestamps={points.map(point => point.timestamp)} series={[{ label, color: 'var(--primary)', values }]} readoutSeries={[
    { label: text('Permintaan', 'Requests'), color: 'var(--primary)', values: points.map(point => point.requests), format: value => value.toLocaleString() },
    { label: text('Pengunjung', 'Visitors'), color: 'var(--info)', values: points.map(point => point.visitors), format: value => value.toLocaleString() },
    { label: text('Transfer', 'Transfer'), color: 'var(--ink-muted)', values: points.map(point => point.bytes), format: analyticsBytes },
    { label: 'Cache hit', color: 'var(--ink-muted)', values: points.map(point => point.requests ? point.cache_hit / point.requests * 100 : 0), format: value => `${Math.round(value)}%` },
  ]} area title={title || text('Lalu lintas permintaan', 'Request traffic')} description={text('Perubahan trafik sepanjang periode terpilih. Garis putus-putus menandai deploy.', 'Traffic over the selected period. Dashed markers indicate deployments.')} height={height} format={metric === 'bytes' ? analyticsBytes : undefined} deployments={deployments} className={className} emptyMessage={text('Belum ada data metrik lalu lintas untuk periode ini', 'No traffic samples for this period')} />;
}
