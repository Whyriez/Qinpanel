import { useId, useRef, useState, useEffect } from 'react';
import type { SiteDeployment } from '../../types/api';
import { AnalyticsEmpty, useAnalyticsCopy } from './AnalyticsUi';

export interface PlotSeries { label: string; color: string; values: number[]; format?: (value: number) => string }
export function analyticsX(timestamp: number, first: number, last: number, width: number) {
  return last === first ? width / 2 : (timestamp - first) / (last - first) * width;
}
export function AnalyticsPlot({ timestamps, series, readoutSeries = series, title, description, height = 240, area = false, bars = false, format = value => value.toLocaleString(), deployments = [], className = '', emptyMessage }: {
  timestamps: number[]; series: PlotSeries[]; title: string; description?: string; height?: number; area?: boolean; bars?: boolean;
  format?: (value: number) => string; readoutSeries?: PlotSeries[]; deployments?: SiteDeployment[]; className?: string; emptyMessage: string;
}) {
  const { en, text } = useAnalyticsCopy();
  const id = useId(), container = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(800), [selected, setSelected] = useState<number | null>(null);
  useEffect(() => {
    if (!container.current) return;
    const observer = new ResizeObserver(entries => setWidth(Math.max(220, entries[0].contentRect.width)));
    observer.observe(container.current);
    return () => observer.disconnect();
  }, []);
  const shortTime = (ts: number) => new Date(ts * 1000).toLocaleString(en ? 'en-GB' : 'id-ID', timestamps.at(-1)! - timestamps[0] >= 86400 ? { day: 'numeric', month: 'short' } : { hour: '2-digit', minute: '2-digit' });
  const fullTime = (ts: number) => new Date(ts * 1000).toLocaleString(en ? 'en-GB' : 'id-ID', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
  const left = 48, right = 14, top = 20, bottom = 32, plotWidth = width - left - right, plotHeight = height - top - bottom;
  const max = Math.max(1, ...timestamps.map((_, index) => bars ? series.reduce((sum, row) => sum + (row.values[index] || 0), 0) : Math.max(...series.map(row => row.values[index] || 0))));
  const x = (index: number) => left + analyticsX(timestamps[index], timestamps[0], timestamps.at(-1)!, plotWidth);
  const y = (value: number) => top + plotHeight - value / max * plotHeight;
  const active = Math.min(selected ?? timestamps.length - 1, timestamps.length - 1);
  const ticks = [...new Set([0, ...(width >= 440 ? [Math.floor((timestamps.length - 1) / 2)] : []), timestamps.length - 1])];
  const choose = (clientX: number, target: SVGSVGElement) => {
    const bounds = target.getBoundingClientRect();
    const pointer = (clientX - bounds.left) * width / bounds.width;
    const closest = timestamps.reduce((best, _, index) => Math.abs(x(index) - pointer) < Math.abs(x(best) - pointer) ? index : best, 0);
    setSelected(closest);
  };
  return <section className={`analytics-chart ${className}`} aria-label={title}>
    <header className="analytics-chart-heading"><div><h3>{title}</h3>{description && <p>{description}</p>}</div><div className="analytics-legend">{series.map(row => <span key={row.label}><i style={{ background: row.color }} />{row.label}</span>)}</div></header>
    <div ref={container} className="analytics-plot">
      {!timestamps.length ? <AnalyticsEmpty title={emptyMessage} description={text('Data akan muncul setelah ada sampel pada periode ini.', 'Data appears when samples are available for this period.')} /> : <>
        <svg viewBox={`0 0 ${width} ${height}`} role="img" tabIndex={0} aria-label={`${title}. ${text('Gunakan panah kiri dan kanan untuk melihat sampel.', 'Use left and right arrows to inspect samples.')}`} onPointerMove={event => choose(event.clientX, event.currentTarget)} onPointerDown={event => choose(event.clientX, event.currentTarget)} onKeyDown={event => {
          const next = event.key === 'ArrowLeft' ? Math.max(0, active - 1) : event.key === 'ArrowRight' ? Math.min(timestamps.length - 1, active + 1) : event.key === 'Home' ? 0 : event.key === 'End' ? timestamps.length - 1 : null;
          if (next !== null) { event.preventDefault(); setSelected(next); }
        }}>
          <defs><linearGradient id={id} x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stopColor="var(--primary)" stopOpacity=".14" /><stop offset="100%" stopColor="var(--primary)" stopOpacity=".01" /></linearGradient></defs>
          {[0, .5, 1].map(fraction => <g key={fraction}><line x1={left} x2={width - right} y1={y(max * fraction)} y2={y(max * fraction)} stroke="var(--line)" strokeDasharray={fraction ? '3 5' : undefined} /><text x={left - 9} y={y(max * fraction) + 4} textAnchor="end">{new Intl.NumberFormat(en ? 'en' : 'id', { notation: 'compact', maximumFractionDigits: 1 }).format(max * fraction)}</text></g>)}
          {series.map((row, seriesIndex) => {
            const path = timestamps.map((_, index) => `${index ? 'L' : 'M'} ${x(index)},${y(row.values[index] || 0)}`).join(' ');
            return <g key={row.label}>{bars ? timestamps.map((_, index) => {
              const previous = series.slice(0, seriesIndex).reduce((sum, earlier) => sum + (earlier.values[index] || 0), 0);
              return <rect key={index} x={x(index) - Math.min(16, plotWidth / timestamps.length * .65) / 2} y={y(previous + row.values[index])} width={Math.min(16, plotWidth / timestamps.length * .65)} height={Math.max(0, row.values[index] / max * plotHeight)} fill={row.color} rx="2" />;
            }) : <>{area && timestamps.length > 1 && <path d={`${path} L ${x(timestamps.length - 1)},${y(0)} L ${x(0)},${y(0)} Z`} fill={`url(#${id})`} />}<path d={path} fill="none" stroke={row.color} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />{timestamps.length === 1 && <circle cx={x(0)} cy={y(row.values[0])} r="4" fill={row.color} />}</>}</g>;
          })}
          {deployments.filter(deploy => deploy.created_at >= timestamps[0] && deploy.created_at <= timestamps.at(-1)!).map(deploy => <g key={deploy.id}><title>{`Deploy ${deploy.release_id.slice(0, 6)}`}</title><line x1={left + analyticsX(deploy.created_at, timestamps[0], timestamps.at(-1)!, plotWidth)} x2={left + analyticsX(deploy.created_at, timestamps[0], timestamps.at(-1)!, plotWidth)} y1={top} y2={y(0)} stroke="var(--accent)" strokeDasharray="4 4" /><circle cx={left + analyticsX(deploy.created_at, timestamps[0], timestamps.at(-1)!, plotWidth)} cy={top} r="3" fill="var(--accent)" /></g>)}
          {ticks.map(index => <text key={index} x={x(index)} y={height - 7} textAnchor={index === 0 && timestamps.length > 1 ? 'start' : index === timestamps.length - 1 && timestamps.length > 1 ? 'end' : 'middle'}>{shortTime(timestamps[index])}</text>)}
          {selected !== null && <g><line x1={x(active)} x2={x(active)} y1={top} y2={y(0)} stroke="var(--ink-muted)" strokeDasharray="2 4" />{!bars && series.map(row => <circle key={row.label} cx={x(active)} cy={y(row.values[active])} r="3.5" fill="var(--surface)" stroke={row.color} strokeWidth="2" />)}</g>}
        </svg>
        <div className="analytics-chart-readout" aria-live="polite"><time>{fullTime(timestamps[active])}</time>{readoutSeries.map(row => <span key={row.label}><i style={{ background: row.color }} />{row.label}<strong>{(row.format || format)(row.values[active])}</strong></span>)}</div>
      </>}
    </div>
  </section>;
}
