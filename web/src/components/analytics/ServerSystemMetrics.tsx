import { useState } from 'react';
import type { ServerTimeseriesPointDto } from '../../types/api';
import { AnalyticsPlot, type PlotSeries } from './AnalyticsPlot';
import { analyticsBytes, AnalyticsMetric, AnalyticsRangePicker, useAnalyticsCopy } from './AnalyticsUi';
export interface ServerSystemMetricsProps { points: ServerTimeseriesPointDto[]; className?: string }
export function ServerSystemMetrics({ points, className = '' }: ServerSystemMetricsProps) {
  const { text } = useAnalyticsCopy();
  const [tab, setTab] = useState<'cpu' | 'memory' | 'load' | 'network' | 'io'>('cpu');
  const latest = points.at(-1);
  const config: Record<typeof tab, { title: string; series: PlotSeries[]; format: (value: number) => string }> = {
    cpu: { title: text('Penggunaan CPU', 'CPU usage'), series: [{ label: text('Rata-rata', 'Average'), color: 'var(--primary)', values: points.map(p => p.cpu_avg) }, { label: text('Puncak', 'Peak'), color: 'var(--info)', values: points.map(p => p.cpu_max) }], format: value => `${value.toFixed(1)}%` },
    memory: { title: text('Memori & swap', 'Memory & swap'), series: [{ label: 'RAM', color: 'var(--primary)', values: points.map(p => p.memory_used_avg_bytes) }, { label: 'Swap', color: 'var(--info)', values: points.map(p => p.swap_used_avg_bytes) }], format: analyticsBytes },
    load: { title: 'Load average', series: [{ label: text('Rata-rata 1 menit', '1-minute average'), color: 'var(--primary)', values: points.map(p => p.load1_avg) }, { label: text('Puncak', 'Peak'), color: 'var(--info)', values: points.map(p => p.load1_max) }], format: value => value.toFixed(2) },
    network: { title: text('Lalu lintas jaringan', 'Network traffic'), series: [{ label: 'RX', color: 'var(--primary)', values: points.map(p => p.net_rx_avg_bytes_sec) }, { label: 'TX', color: 'var(--info)', values: points.map(p => p.net_tx_avg_bytes_sec) }], format: value => `${analyticsBytes(value)}/s` },
    io: { title: text('I/O disk', 'Disk I/O'), series: [{ label: text('Baca', 'Read'), color: 'var(--primary)', values: points.map(p => p.io_read_avg_bytes_sec) }, { label: text('Tulis', 'Write'), color: 'var(--info)', values: points.map(p => p.io_write_avg_bytes_sec) }], format: value => `${analyticsBytes(value)}/s` },
  };
  return <section className={`analytics-server-metrics ${className}`} aria-label={text('Metrik sistem server', 'Server system metrics')}>
    {latest && <div className="analytics-metric-grid"><AnalyticsMetric label="CPU" value={`${latest.cpu_avg.toFixed(1)}%`} hint={`${text('Puncak', 'Peak')}: ${latest.cpu_max.toFixed(1)}%`} /><AnalyticsMetric label={text('Memori terpakai', 'Used memory')} value={analyticsBytes(latest.memory_used_avg_bytes)} hint={`Swap: ${analyticsBytes(latest.swap_used_avg_bytes)}`} /><AnalyticsMetric label="Load average" value={latest.load1_avg.toFixed(2)} hint={text('Sampel terakhir / 1 menit', 'Last sample / 1 minute')} /><AnalyticsMetric label={text('Disk terpakai', 'Used disk')} value={analyticsBytes(latest.disk_used_avg_bytes)} hint={text('Sampel terakhir', 'Last sample')} /></div>}
    <AnalyticsRangePicker value={tab} onChange={setTab} label={text('Jenis metrik server', 'Server metric type')} options={[{ value: 'cpu', label: 'CPU' }, { value: 'memory', label: text('Memori', 'Memory') }, { value: 'load', label: 'Load' }, { value: 'network', label: text('Jaringan', 'Network') }, { value: 'io', label: 'I/O disk' }]} />
    <AnalyticsPlot timestamps={points.map(p => p.timestamp)} title={config[tab].title} description={text('Sumber daya host sepanjang periode terpilih.', 'Host resources over the selected period.')} series={config[tab].series} format={config[tab].format} emptyMessage={text('Belum ada data metrik server untuk periode ini', 'No server metric samples for this period')} />
  </section>;
}
