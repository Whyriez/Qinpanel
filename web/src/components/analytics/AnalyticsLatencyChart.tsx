import type { SiteTimeseriesPointDto } from '../../types/api';
import { AnalyticsPlot } from './AnalyticsPlot';
import { useAnalyticsCopy } from './AnalyticsUi';
export interface AnalyticsLatencyChartProps { points: SiteTimeseriesPointDto[]; height?: number; title?: string; className?: string }
export function AnalyticsLatencyChart({ points, height = 230, title, className }: AnalyticsLatencyChartProps) {
  const { text } = useAnalyticsCopy();
  return <AnalyticsPlot timestamps={points.map(point => point.timestamp)} series={[
    { label: 'p50', color: 'var(--primary)', values: points.map(point => point.latency_p50_ms) },
    { label: 'p95', color: 'var(--info)', values: points.map(point => point.latency_p95_ms) },
    { label: 'p99', color: 'var(--accent)', values: points.map(point => point.latency_p99_ms) },
  ]} title={title || text('Waktu respons', 'Response time')} description={text('Persentil latensi dalam milidetik.', 'Latency percentiles in milliseconds.')} format={value => `${value.toLocaleString()} ms`} height={height} className={className} emptyMessage={text('Belum ada data latensi untuk periode ini', 'No latency samples for this period')} />;
}
