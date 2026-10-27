import React, { useState, useMemo } from 'react';
import { BreakdownItemDto, SiteBreakdownsResponse } from '../../types/api';

export interface BreakdownTableProps {
  breakdowns: SiteBreakdownsResponse | null;
  className?: string;
}

type DimensionKey =
  | 'paths'
  | 'referrers'
  | 'countries'
  | 'browsers'
  | 'os'
  | 'devices'
  | 'slow_urls'
  | 'error_urls';

interface DimensionConfig {
  key: DimensionKey;
  label: string;
  icon: string;
  columnHeader: string;
}

const DIMENSIONS: DimensionConfig[] = [
  { key: 'paths', label: 'Jalur (Paths)', icon: '📄', columnHeader: 'Path URL' },
  { key: 'referrers', label: 'Perujuk (Referrers)', icon: '🔗', columnHeader: 'Domain Perujuk' },
  { key: 'countries', label: 'Negara (GeoIP)', icon: '🌍', columnHeader: 'Negara' },
  { key: 'browsers', label: 'Browser', icon: '🌐', columnHeader: 'Peramban' },
  { key: 'os', label: 'Sistem Operasi', icon: '💻', columnHeader: 'Sistem Operasi' },
  { key: 'devices', label: 'Perangkat', icon: '📱', columnHeader: 'Tipe Perangkat' },
  { key: 'slow_urls', label: 'URL Lambat', icon: '⏱️', columnHeader: 'Jalur Eksekusi' },
  { key: 'error_urls', label: 'URL Error', icon: '⚠️', columnHeader: 'Jalur Error' },
];

export const BreakdownTable: React.FC<BreakdownTableProps> = ({
  breakdowns,
  className = '',
}) => {
  const [activeDimension, setActiveDimension] = useState<DimensionKey>('paths');
  const [searchQuery, setSearchQuery] = useState('');
  const [limit, setLimit] = useState<number>(10);

  const currentItems = useMemo<BreakdownItemDto[]>(() => {
    if (!breakdowns) return [];
    const list = breakdowns[activeDimension] || [];
    if (!searchQuery.trim()) return list;
    const q = searchQuery.toLowerCase();
    return list.filter((item) => item.name.toLowerCase().includes(q));
  }, [breakdowns, activeDimension, searchQuery]);

  const displayedItems = currentItems.slice(0, limit);
  const currentDimConfig = DIMENSIONS.find((d) => d.key === activeDimension) || DIMENSIONS[0];

  return (
    <div className={`bg-[var(--surface)] border border-[var(--line)] rounded-xl p-5 shadow-sm ${className}`}>
      {/* Header and Dimension Switcher */}
      <div className="flex flex-col gap-4 mb-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h3 className="text-base font-semibold text-[var(--ink)]">
            Rincian Dimensi Lalu Lintas
          </h3>
          <div className="flex items-center gap-2">
            <input
              type="text"
              placeholder={`Cari di ${currentDimConfig.label}...`}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="px-3 py-1.5 text-xs bg-[var(--bg)] border border-[var(--line)] rounded-lg text-[var(--ink)] placeholder-[var(--ink-muted)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)]"
            />
            <select
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value))}
              className="px-2.5 py-1.5 text-xs bg-[var(--bg)] border border-[var(--line)] rounded-lg text-[var(--ink)] focus:outline-none"
            >
              <option value={5}>Top 5</option>
              <option value={10}>Top 10</option>
              <option value={25}>Top 25</option>
              <option value={50}>Top 50</option>
            </select>
          </div>
        </div>

        {/* Tab Pills */}
        <div className="flex items-center gap-1.5 overflow-x-auto pb-1 border-b border-[var(--line)]">
          {DIMENSIONS.map((dim) => {
            const isActive = activeDimension === dim.key;
            const count = breakdowns ? (breakdowns[dim.key] || []).length : 0;
            return (
              <button
                key={dim.key}
                type="button"
                onClick={() => {
                  setActiveDimension(dim.key);
                  setSearchQuery('');
                }}
                className={`flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium whitespace-nowrap transition-colors ${
                  isActive
                    ? 'bg-[var(--primary)] text-white shadow-sm'
                    : 'text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface-hover)]'
                }`}
              >
                <span>{dim.icon}</span>
                <span>{dim.label}</span>
                {count > 0 && (
                  <span
                    className={`ml-1 text-[10px] px-1.5 py-0.2 rounded-full ${
                      isActive ? 'bg-white/20 text-white' : 'bg-[var(--line)] text-[var(--ink-muted)]'
                    }`}
                  >
                    {count}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      </div>

      {/* Table Content */}
      {displayedItems.length === 0 ? (
        <div className="flex flex-col items-center justify-center p-8 text-center text-sm text-[var(--ink-muted)]">
          <p>Tidak ada data ditemukan untuk {currentDimConfig.label}.</p>
          {searchQuery && (
            <button
              type="button"
              onClick={() => setSearchQuery('')}
              className="mt-2 text-xs text-[var(--primary)] hover:underline"
            >
              Hapus filter pencarian
            </button>
          )}
        </div>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-left text-xs border-collapse">
            <thead>
              <tr className="border-b border-[var(--line)] text-[var(--ink-muted)]">
                <th className="py-2.5 px-3 font-medium w-12 text-center">#</th>
                <th className="py-2.5 px-3 font-medium">{currentDimConfig.columnHeader}</th>
                <th className="py-2.5 px-3 font-medium text-right w-28">Jumlah</th>
                <th className="py-2.5 px-3 font-medium w-48 text-right">Porsi</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-[var(--line)]">
              {displayedItems.map((item, idx) => (
                <tr
                  key={`${item.name}-${idx}`}
                  className="hover:bg-[var(--surface-hover)] transition-colors group"
                >
                  <td className="py-2.5 px-3 text-center text-[var(--ink-muted)] font-mono">
                    {idx + 1}
                  </td>
                  <td className="py-2.5 px-3 font-medium text-[var(--ink)] break-all max-w-md">
                    <span className="font-mono text-[11px] select-all">{item.name}</span>
                  </td>
                  <td className="py-2.5 px-3 text-right text-[var(--ink)] font-mono font-medium">
                    {item.count.toLocaleString()}
                  </td>
                  <td className="py-2.5 px-3 text-right">
                    <div className="flex items-center justify-end gap-2.5">
                      <div className="w-24 bg-[var(--line)] h-2 rounded-full overflow-hidden flex-shrink-0">
                        <div
                          className="bg-[var(--primary)] h-full rounded-full transition-all duration-300"
                          style={{ width: `${Math.min(Math.max(item.percentage, 1), 100)}%` }}
                        />
                      </div>
                      <span className="w-10 text-[11px] font-mono text-[var(--ink-muted)]">
                        {item.percentage.toFixed(1)}%
                      </span>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
};
