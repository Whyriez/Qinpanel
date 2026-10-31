import type { SiteTimeseriesPointDto } from '../../types/api';
import { AnalyticsPlot } from './AnalyticsPlot';
import { useAnalyticsCopy } from './AnalyticsUi';
export interface AnalyticsBarChartProps { points: SiteTimeseriesPointDto[]; height?: number; title?: string; className?: string }
export function AnalyticsBarChart({ points, height = 230, title, className }: AnalyticsBarChartProps) {
  const { text } = useAnalyticsCopy();
  return <AnalyticsPlot timestamps={points.map(point => point.timestamp)} series={[
    { label: '4xx', color: 'var(--accent)', values: points.map(point => point.status_4xx) },
    { label: '5xx', color: 'var(--danger)', values: points.map(point => point.status_5xx) },
  ]} bars title={title || text('Respons error', 'Error responses')} description={text('Jumlah respons 4xx dan 5xx per interval.', 'Number of 4xx and 5xx responses per interval.')} height={height} className={className} emptyMessage={text('Belum ada data error untuk periode ini', 'No error samples for this period')} />;
}
