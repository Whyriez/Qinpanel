import React, { useState } from 'react';
import { SiteTimeseriesPointDto } from '../../types/api';

export interface AnalyticsBarChartProps {
  points: SiteTimeseriesPointDto[];
  height?: number;
  title?: string;
  className?: string;
}

function formatTime(ts: number): string {
  const d = new Date(ts * 1000);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function formatDateTime(ts: number): string {
  const d = new Date(ts * 1000);
  return `${d.toLocaleDateString([], { month: 'short', day: 'numeric' })} ${d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`;
}

export const AnalyticsBarChart: React.FC<AnalyticsBarChartProps> = ({
  points,
  height = 220,
  title = 'Distribusi Kode Error (4xx & 5xx)',
  className = '',
}) => {
  const [hoverIdx, setHoverIdx] = useState<number | null>(null);

  if (!points || points.length === 0) {
    return (
      <div
        className={`flex items-center justify-center border rounded-lg p-6 bg-[var(--surface)] text-[var(--ink-muted)] text-sm ${className}`}
        style={{ height }}
      >
        Belum ada data error untuk periode ini
      </div>
    );
  }

  const total4xx = points.reduce((acc, p) => acc + (p.status_4xx || 0), 0);
  const total5xx = points.reduce((acc, p) => acc + (p.status_5xx || 0), 0);
  const maxErrors = Math.max(...points.map((p) => (p.status_4xx || 0) + (p.status_5xx || 0)), 5);

  const svgWidth = 800;
  const paddingLeft = 55;
  const paddingRight = 20;
  const paddingTop = 25;
  const paddingBottom = 30;

  const plotWidth = svgWidth - paddingLeft - paddingRight;
  const plotHeight = height - paddingTop - paddingBottom;
  const barWidth = Math.max(Math.min((plotWidth / points.length) * 0.7, 16), 3);

  const yTicks = [0, 0.5, 1].map((pct) => {
    const val = Math.round(maxErrors * pct);
    const y = paddingTop + plotHeight - pct * plotHeight;
    return { val, y };
  });

  const xTickIndices = [0, Math.floor(points.length * 0.5), points.length - 1];
  const uniqueXTicks = Array.from(new Set(xTickIndices)).filter((i) => i >= 0 && i < points.length);

  const activePoint = hoverIdx !== null ? points[hoverIdx] : null;

  return (
    <div className={`relative flex flex-col bg-[var(--surface)] border border-[var(--line)] rounded-xl p-4 shadow-sm ${className}`}>
      <div className="flex flex-wrap items-center justify-between gap-2 mb-2">
        <h4 className="text-sm font-semibold text-[var(--ink)]">{title}</h4>
        <div className="flex items-center gap-4 text-xs">
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-sm bg-amber-500" />
            <span className="text-[var(--ink-muted)]">4xx ({total4xx.toLocaleString()})</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-sm bg-rose-500" />
            <span className="text-[var(--ink-muted)]">5xx ({total5xx.toLocaleString()})</span>
          </div>
        </div>
      </div>

      <div className="relative w-full overflow-hidden">
        <svg
          viewBox={`0 0 ${svgWidth} ${height}`}
          className="w-full h-auto select-none"
          onMouseLeave={() => setHoverIdx(null)}
          role="img"
          aria-label={title}
        >
          {/* Grid lines horizontal */}
          {yTicks.map((t, idx) => (
            <g key={idx}>
              <line
                x1={paddingLeft}
                y1={t.y}
                x2={svgWidth - paddingRight}
                y2={t.y}
                stroke="var(--line)"
                strokeDasharray="3 3"
                strokeOpacity="0.6"
              />
              <text
                x={paddingLeft - 8}
                y={t.y + 4}
                textAnchor="end"
                fontSize="10"
                fill="var(--ink-muted)"
              >
                {t.val}
              </text>
            </g>
          ))}

          {/* X Axis line */}
          <line
            x1={paddingLeft}
            y1={paddingTop + plotHeight}
            x2={svgWidth - paddingRight}
            y2={paddingTop + plotHeight}
            stroke="var(--line)"
          />

          {/* Bars */}
          {points.map((p, idx) => {
            const centerX = paddingLeft + (idx / (points.length - 1 || 1)) * plotWidth;
            const x = centerX - barWidth / 2;
            const h4 = ((p.status_4xx || 0) / maxErrors) * plotHeight;
            const h5 = ((p.status_5xx || 0) / maxErrors) * plotHeight;
            const y5 = paddingTop + plotHeight - h5;
            const y4 = y5 - h4;

            return (
              <g key={idx} onMouseEnter={() => setHoverIdx(idx)}>
                {/* 5xx bottom bar */}
                {h5 > 0 && (
                  <rect
                    x={x}
                    y={y5}
                    width={barWidth}
                    height={h5}
                    fill="#ef4444"
                    rx={h4 === 0 ? 2 : 0}
                  />
                )}
                {/* 4xx stacked bar */}
                {h4 > 0 && (
                  <rect
                    x={x}
                    y={y4}
                    width={barWidth}
                    height={h4}
                    fill="#f59e0b"
                    rx={2}
                  />
                )}
                {/* Invisible hover trigger */}
                <rect
                  x={centerX - plotWidth / points.length / 2}
                  y={paddingTop}
                  width={plotWidth / points.length}
                  height={plotHeight}
                  fill="transparent"
                  className="cursor-crosshair"
                />
              </g>
            );
          })}

          {/* X Axis Labels */}
          {uniqueXTicks.map((idx) => {
            const pt = points[idx];
            const x = paddingLeft + (idx / (points.length - 1 || 1)) * plotWidth;
            return (
              <text
                key={idx}
                x={x}
                y={height - 8}
                textAnchor="middle"
                fontSize="10"
                fill="var(--ink-muted)"
              >
                {formatTime(pt.timestamp)}
              </text>
            );
          })}
        </svg>

        {/* Hover Tooltip Popup */}
        {activePoint && hoverIdx !== null && (
          <div
            className="absolute z-20 pointer-events-none px-3 py-2 text-xs rounded-lg shadow-xl backdrop-blur-md transition-all duration-75 border"
            style={{
              left: `${Math.min(Math.max((hoverIdx / points.length) * 100, 10), 85)}%`,
              top: '12px',
              backgroundColor: 'var(--surface-overlay, rgba(30, 41, 59, 0.95))',
              color: 'var(--ink, #ffffff)',
              borderColor: 'var(--line, rgba(255, 255, 255, 0.15))',
            }}
          >
            <div className="font-semibold text-gray-200 border-b border-gray-700/60 pb-1 mb-1.5 flex items-center justify-between gap-3">
              <span>{formatDateTime(activePoint.timestamp)}</span>
            </div>
            <div className="space-y-1 text-[11px]">
              <div className="flex items-center justify-between gap-4">
                <span className="text-amber-400">Status 4xx:</span>
                <span className="font-mono font-semibold text-white">{activePoint.status_4xx || 0}</span>
              </div>
              <div className="flex items-center justify-between gap-4">
                <span className="text-rose-400">Status 5xx:</span>
                <span className="font-mono font-semibold text-white">{activePoint.status_5xx || 0}</span>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
