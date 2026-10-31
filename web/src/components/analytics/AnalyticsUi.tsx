import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Activity, ArrowDownRight, ArrowUpRight, BarChart3 } from 'lucide-react';
import '../../styles/analytics.css';

export const analyticsBytes = (value: number) => {
  if (!Number.isFinite(value) || value < 0) return '—';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  const index = Math.min(4, Math.floor(Math.log2(Math.max(value, 1)) / 10));
  return `${(value / 1024 ** index).toLocaleString(undefined, { maximumFractionDigits: index ? 1 : 0 })} ${units[index]}`;
};
export function useAnalyticsCopy() {
  const { i18n } = useTranslation();
  const en = i18n.language.startsWith('en');
  return { en, text: (id: string, english: string) => en ? english : id };
}
export function AnalyticsMetric({ label, value, hint, change, icon, danger = false }: { label: string; value: ReactNode; hint?: ReactNode; change?: number | null; icon?: ReactNode; danger?: boolean }) {
  const { text } = useAnalyticsCopy();
  return <article className={`analytics-metric${danger ? ' is-danger' : ''}`}>
    <div className="analytics-metric-label"><span>{label}</span>{icon || <Activity size={16} aria-hidden="true" />}</div>
    <strong className="analytics-metric-value">{value}</strong>
    <div className="analytics-metric-foot">{change != null && Number.isFinite(change) ? <span className="analytics-change">{change >= 0 ? <ArrowUpRight size={14} aria-hidden="true" /> : <ArrowDownRight size={14} aria-hidden="true" />}{change > 0 ? '+' : ''}{change.toFixed(1)}% <span>{text('vs periode lalu', 'vs previous period')}</span></span> : hint || <span>{text('Periode terpilih', 'Selected period')}</span>}</div>
  </article>;
}
export function AnalyticsEmpty({ title, description, children }: { title: string; description?: string; children?: ReactNode }) {
  return <div className="analytics-empty"><span><BarChart3 size={25} aria-hidden="true" /></span><h3>{title}</h3>{description && <p>{description}</p>}{children}</div>;
}
export function AnalyticsLoading() {
  const { text } = useAnalyticsCopy();
  return <div role="status" className="analytics-loading"><span className="sr-only">{text('Memuat analitik...', 'Loading analytics...')}</span><div className="analytics-metric-grid" aria-hidden="true">{[0, 1, 2, 3].map(value => <div key={value} className="analytics-skeleton-metric" />)}</div><div className="analytics-skeleton-chart" aria-hidden="true" /></div>;
}
export function AnalyticsRangePicker<T extends string>({ options, value, onChange, label }: { options: { value: T; label: string }[]; value: T; onChange: (value: T) => void; label: string }) {
  return <div className="analytics-segments" role="group" aria-label={label}>{options.map(option => <button type="button" key={option.value} aria-pressed={value === option.value} onClick={() => onChange(option.value)}>{option.label}</button>)}</div>;
}
