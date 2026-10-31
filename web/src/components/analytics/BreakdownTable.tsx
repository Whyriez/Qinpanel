import { useState } from 'react';
import { Search, X } from 'lucide-react';
import type { SiteBreakdownsResponse } from '../../types/api';
import { useAnalyticsCopy } from './AnalyticsUi';
export interface BreakdownTableProps { breakdowns: SiteBreakdownsResponse | null; className?: string }
type DimensionKey = 'paths' | 'referrers' | 'countries' | 'browsers' | 'os' | 'devices' | 'slow_urls' | 'error_urls';
export function BreakdownTable({ breakdowns, className = '' }: BreakdownTableProps) {
  const { text } = useAnalyticsCopy();
  const [dimension, setDimension] = useState<DimensionKey>('paths'), [search, setSearch] = useState(''), [limit, setLimit] = useState(10);
  const dimensions: { key: DimensionKey; label: string }[] = [
    { key: 'paths', label: text('Halaman', 'Pages') }, { key: 'referrers', label: text('Sumber trafik', 'Referrers') }, { key: 'countries', label: text('Negara', 'Countries') }, { key: 'browsers', label: 'Browser' }, { key: 'os', label: text('Sistem operasi', 'Operating systems') }, { key: 'devices', label: text('Perangkat', 'Devices') }, { key: 'slow_urls', label: text('URL lambat', 'Slow URLs') }, { key: 'error_urls', label: text('URL error', 'Error URLs') },
  ];
  const label = dimensions.find(item => item.key === dimension)!.label;
  const all = breakdowns?.[dimension] ?? [];
  const items = all.filter(item => item.name.toLocaleLowerCase().includes(search.trim().toLocaleLowerCase()));
  return <section className={`analytics-breakdowns analytics-breakdown-card ${className}`} aria-label={text('Rincian trafik', 'Traffic breakdowns')}>
    <header><div><h3>{text('Kenali sumber trafik Anda', 'Understand your traffic')}</h3><p>{text('Telusuri halaman, sumber kunjungan, dan perangkat yang paling aktif.', 'Explore your most active pages, traffic sources and devices.')}</p></div><span>{text('Maksimal 15 hasil per dimensi', 'Up to 15 results per dimension')}</span></header>
    <div className="analytics-breakdown-tabs" role="group" aria-label={text('Dimensi trafik', 'Traffic dimension')}>{dimensions.map(item => <button type="button" key={item.key} aria-pressed={dimension === item.key} onClick={() => { setDimension(item.key); setSearch(''); }}>{item.label}</button>)}</div>
    <div className="analytics-table-toolbar"><label className="analytics-table-search"><Search size={15} aria-hidden="true" /><input type="search" aria-label={text('Cari rincian trafik', 'Search traffic breakdowns')} placeholder={`${text('Cari', 'Search')} ${label.toLowerCase()}...`} value={search} onChange={event => setSearch(event.target.value)} />{search && <button type="button" aria-label={text('Hapus pencarian rincian', 'Clear breakdown search')} onClick={() => setSearch('')}><X size={14} /></button>}</label><label>{text('Tampilkan', 'Show')}<select aria-label={text('Jumlah rincian', 'Breakdown rows')} value={limit} onChange={event => setLimit(Number(event.target.value))}><option value={5}>5</option><option value={10}>10</option><option value={15}>15</option></select></label></div>
    {items.length ? <div className="analytics-table-scroll"><table><thead><tr><th scope="col">{label}</th><th scope="col">{text('Jumlah', 'Count')}</th><th scope="col">{text('Porsi', 'Share')}</th></tr></thead><tbody>{items.slice(0, limit).map((item, index) => <tr key={`${item.name}:${index}`}><td><span className="analytics-table-rank">{index + 1}</span><span className={dimension === 'paths' || dimension.endsWith('urls') ? 'analytics-table-path' : ''}>{item.name}</span></td><td>{item.count.toLocaleString()}</td><td><div className="analytics-table-share"><span><i style={{ width: `${Math.max(0, Math.min(100, item.percentage))}%` }} /></span><strong>{item.percentage.toFixed(1)}%</strong></div></td></tr>)}</tbody></table></div> : <div className="analytics-breakdown-empty"><Search size={22} /><h4>{search ? text('Tidak ada hasil yang cocok', 'No matching results') : text('Belum ada rincian untuk periode ini', 'No breakdowns for this period')}</h4><p>{search ? text('Coba nama lain atau hapus pencarian.', 'Try another name or clear the search.') : text('Ubah dimensi atau periode untuk melihat data lainnya.', 'Choose another dimension or period to explore more data.')}</p></div>}
    <footer>{Math.min(items.length, limit)} / {items.length} {text('hasil tersaji', 'available results')}</footer>
  </section>;
}
