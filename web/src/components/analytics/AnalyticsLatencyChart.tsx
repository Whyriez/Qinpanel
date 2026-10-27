import React, { useState } from 'react';
import { SiteTimeseriesPointDto } from '../../types/api';

export interface AnalyticsLatencyChartProps {
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

export const AnalyticsLatencyChart: React.FC<AnalyticsLatencyChartProps> = ({
  points,
  height = 240,
  title = 'Latensi Respons (p50 / p95 / p99)',
  className = '',
}) => {
  const [hoverIdx, setHoverIdx] = useState<number | null>(null);

  if (!points || points.length === 0) {
    return (
      <div
        className={`flex items-center justify-center border rounded-lg p-6 bg-[var(--surface)] text-[var(--ink-muted)] text-sm ${className}`}
        style={{ height }}
      >
        Belum ada data latensi untuk periode ini
      </div>
    );
  }

  // Find max latency across all points
  const maxLatency = Math.max(
    ...points.map((p) => Math.max(p.latency_p99_ms || 0, p.latency_p95_ms || 0, p.latency_p50_ms || 0, p.latency_avg_ms || 0)),
    10
  );

  const svgWidth = 800;
  const paddingLeft = 55;
  const paddingRight = 20;
  const paddingTop = 25;
  const paddingBottom = 30;

  const plotWidth = svgWidth - paddingLeft - paddingRight;
  const plotHeight = height - paddingTop - paddingBottom;

  const coords = points.map((p, idx) => {
    const x = paddingLeft + (idx / (points.length - 1 || 1)) * plotWidth;
    const y50 = paddingTop + plotHeight - ((p.latency_p50_ms || 0) / maxLatency) * plotHeight;
    const y95 = paddingTop + plotHeight - ((p.latency_p95_ms || 0) / maxLatency) * plotHeight;
    const y99 = paddingTop + plotHeight - ((p.latency_p99_ms || 0) / maxLatency) * plotHeight;
    return { x, y50, y95, y99, point: p };
  });

  const path50 = `M ${coords.map((c) => `${c.x.toFixed(1)},${c.y50.toFixed(1)}`).join(' L ')}`;
  const path95 = `M ${coords.map((c) => `${c.x.toFixed(1)},${c.y95.toFixed(1)}`).join(' L ')}`;
  const path99 = `M ${coords.map((c) => `${c.x.toFixed(1)},${c.y99.toFixed(1)}`).join(' L ')}`;

  const yTicks = [0, 0.33, 0.66, 1].map((pct) => {
    const val = Math.round(maxLatency * pct);
    const y = paddingTop + plotHeight - pct * plotHeight;
    return { val, y };
  });

  const xTickIndices = [0, Math.floor(points.length * 0.25), Math.floor(points.length * 0.5), Math.floor(points.length * 0.75), points.length - 1];
  const uniqueXTicks = Array.from(new Set(xTickIndices)).filter((i) => i >= 0 && i < points.length);

  const activePoint = hoverIdx !== null ? coords[hoverIdx] : null;

  return (
    <div className={`relative flex flex-col bg-[var(--surface)] border border-[var(--line)] rounded-xl p-4 shadow-sm ${className}`}>
      <div className="flex flex-wrap items-center justify-between gap-2 mb-2">
        <h4 className="text-sm font-semibold text-[var(--ink)]">{title}</h4>
        {/* Legend */}
        <div className="flex items-center gap-4 text-xs">
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-full bg-blue-500" />
            <span className="text-[var(--ink-muted)]">p50</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-full bg-amber-500" />
            <span className="text-[var(--ink-muted)]">p95</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-full bg-rose-500" />
            <span className="text-[var(--ink-muted)]">p99</span>
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
                {t.val} ms
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

          {/* p99 Line (Red/Rose) */}
          <path
            d={path99}
            fill="none"
            stroke="#ef4444"
            strokeWidth="1.75"
            strokeLinecap="round"
            strokeLinejoin="round"
          />

          {/* p95 Line (Amber) */}
          <path
            d={path95}
            fill="none"
            stroke="#f59e0b"
            strokeWidth="1.75"
            strokeLinecap="round"
            strokeLinejoin="round"
          />

          {/* p50 Line (Blue/Primary) */}
          <path
            d={path50}
            fill="none"
            stroke="#3b82f6"
            strokeWidth="2.2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />

          {/* X Axis Labels */}
          {uniqueXTicks.map((idx) => {
            const pt = coords[idx];
            return (
              <text
                key={idx}
                x={pt.x}
                y={height - 8}
                textAnchor="middle"
                fontSize="10"
                fill="var(--ink-muted)"
              >
                {formatTime(pt.point.timestamp)}
              </text>
            );
          })}

          {/* Interactive Hover Columns */}
          {coords.map((c, i) => (
            <rect
              key={i}
              x={c.x - plotWidth / points.length / 2}
              y={paddingTop}
              width={plotWidth / points.length}
              height={plotHeight}
              fill="transparent"
              className="cursor-crosshair"
              onMouseEnter={() => setHoverIdx(i)}
            />
          ))}

          {/* Active Hover Marker */}
          {activePoint && (
            <g>
              <line
                x1={activePoint.x}
                y1={paddingTop}
                x2={activePoint.x}
                y2={paddingTop + plotHeight}
                stroke="var(--line)"
                strokeWidth="1"
                strokeDasharray="2 2"
              />
              <circle cx={activePoint.x} cy={activePoint.y50} r="3.5" fill="#3b82f6" stroke="var(--bg)" strokeWidth="1.5" />
              <circle cx={activePoint.x} cy={activePoint.y95} r="3.5" fill="#f59e0b" stroke="var(--bg)" strokeWidth="1.5" />
              <circle cx={activePoint.x} cy={activePoint.y99} r="3.5" fill="#ef4444" stroke="var(--bg)" strokeWidth="1.5" />
            </g>
          )}
        </svg>

        {/* Hover Tooltip Popup */}
        {activePoint && (
          <div
            className="absolute z-20 pointer-events-none px-3 py-2 text-xs rounded-lg shadow-xl backdrop-blur-md transition-all duration-75 border"
            style={{
              left: `${Math.min(Math.max((activePoint.x / svgWidth) * 100, 10), 85)}%`,
              top: '12px',
              backgroundColor: 'var(--surface-overlay, rgba(30, 41, 59, 0.95))',
              color: 'var(--ink, #ffffff)',
              borderColor: 'var(--line, rgba(255, 255, 255, 0.15))',
            }}
          >
            <div className="font-semibold text-gray-200 border-b border-gray-700/60 pb-1 mb-1.5 flex items-center justify-between gap-3">
              <span>{formatDateTime(activePoint.point.timestamp)}</span>
              <span className="text-[10px] text-gray-400 font-mono">
                avg: {activePoint.point.latency_avg_ms} ms
              </span>
            </div>
            <div className="space-y-1 text-[11px]">
              <div className="flex items-center justify-between gap-4">
                <span className="flex items-center gap-1.5 text-blue-400">
                  <span className="w-2 h-2 rounded-full bg-blue-400" /> p50 (median):
                </span>
                <span className="font-mono font-semibold text-white">{activePoint.point.latency_p50_ms} ms</span>
              </div>
              <div className="flex items-center justify-between gap-4">
                <span className="flex items-center gap-1.5 text-amber-400">
                  <span className="w-2 h-2 rounded-full bg-amber-400" /> p95:
                </span>
                <span className="font-mono font-semibold text-white">{activePoint.point.latency_p95_ms} ms</span>
              </div>
              <div className="flex items-center justify-between gap-4">
                <span className="flex items-center gap-1.5 text-rose-400">
                  <span className="w-2 h-2 rounded-full bg-rose-400" /> p99 (tail):
                </span>
                <span className="font-mono font-semibold text-white">{activePoint.point.latency_p99_ms} ms</span>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
