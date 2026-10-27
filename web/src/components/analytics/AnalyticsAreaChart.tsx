import React, { useState, useId } from 'react';
import { SiteTimeseriesPointDto, SiteDeployment } from '../../types/api';

export interface AnalyticsAreaChartProps {
  points: SiteTimeseriesPointDto[];
  deployments?: SiteDeployment[];
  height?: number;
  metric?: 'requests' | 'bytes' | 'visitors';
  title?: string;
  className?: string;
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

function formatTime(ts: number): string {
  const d = new Date(ts * 1000);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function formatDateTime(ts: number): string {
  const d = new Date(ts * 1000);
  return `${d.toLocaleDateString([], { month: 'short', day: 'numeric' })} ${d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`;
}

export const AnalyticsAreaChart: React.FC<AnalyticsAreaChartProps> = ({
  points,
  deployments = [],
  height = 240,
  metric = 'requests',
  title,
  className = '',
}) => {
  const gradId = useId();
  const [hoverIdx, setHoverIdx] = useState<number | null>(null);

  if (!points || points.length === 0) {
    return (
      <div
        className={`flex items-center justify-center border rounded-lg p-6 bg-[var(--surface)] text-[var(--ink-muted)] text-sm ${className}`}
        style={{ height }}
      >
        Belum ada data metrik lalu lintas untuk periode ini
      </div>
    );
  }

  const values = points.map((p) => {
    if (metric === 'bytes') return p.bytes;
    if (metric === 'visitors') return p.visitors;
    return p.requests;
  });

  const maxVal = Math.max(...values, 5);
  const minTs = points[0].timestamp;
  const maxTs = points[points.length - 1].timestamp;
  const tsRange = maxTs - minTs || 1;

  const svgWidth = 800;
  const paddingLeft = 55;
  const paddingRight = 20;
  const paddingTop = 25;
  const paddingBottom = 30;

  const plotWidth = svgWidth - paddingLeft - paddingRight;
  const plotHeight = height - paddingTop - paddingBottom;

  const coords = points.map((p, idx) => {
    const x = paddingLeft + (idx / (points.length - 1 || 1)) * plotWidth;
    const val = values[idx];
    const y = paddingTop + plotHeight - (val / maxVal) * plotHeight;
    return { x, y, point: p, val };
  });

  const lineD = coords.length > 1
    ? `M ${coords.map((c) => `${c.x.toFixed(1)},${c.y.toFixed(1)}`).join(' L ')}`
    : `M ${paddingLeft},${paddingTop + plotHeight / 2} L ${svgWidth - paddingRight},${paddingTop + plotHeight / 2}`;

  const areaD = coords.length > 1
    ? `${lineD} L ${coords[coords.length - 1].x.toFixed(1)},${paddingTop + plotHeight} L ${paddingLeft},${paddingTop + plotHeight} Z`
    : '';

  // Deploy markers within timeseries range
  const validDeployments = deployments.filter((d) => {
    const ts = d.created_at || 0;
    return ts >= minTs && ts <= maxTs;
  });

  // Y-axis ticks (4 ticks)
  const yTicks = [0, 0.33, 0.66, 1].map((pct) => {
    const val = Math.round(maxVal * pct);
    const y = paddingTop + plotHeight - pct * plotHeight;
    return { val, y };
  });

  // X-axis ticks (up to 5 labels)
  const xTickIndices = [0, Math.floor(points.length * 0.25), Math.floor(points.length * 0.5), Math.floor(points.length * 0.75), points.length - 1];
  const uniqueXTicks = Array.from(new Set(xTickIndices)).filter((i) => i >= 0 && i < points.length);

  const activePoint = hoverIdx !== null ? coords[hoverIdx] : null;

  return (
    <div className={`relative flex flex-col bg-[var(--surface)] border border-[var(--line)] rounded-xl p-4 shadow-sm ${className}`}>
      {title && (
        <div className="flex items-center justify-between mb-2">
          <h4 className="text-sm font-semibold text-[var(--ink)]">{title}</h4>
          <span className="text-xs text-[var(--ink-muted)]">
            Puncak: {metric === 'bytes' ? formatBytes(maxVal) : `${maxVal.toLocaleString()} ${metric}`}
          </span>
        </div>
      )}

      <div className="relative w-full overflow-hidden">
        <svg
          viewBox={`0 0 ${svgWidth} ${height}`}
          className="w-full h-auto select-none"
          onMouseLeave={() => setHoverIdx(null)}
          role="img"
          aria-label={title || 'Grafik Lalu Lintas'}
        >
          <defs>
            <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="var(--primary)" stopOpacity="0.35" />
              <stop offset="90%" stopColor="var(--primary)" stopOpacity="0.0" />
            </linearGradient>
          </defs>

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
                {metric === 'bytes' ? formatBytes(t.val) : t.val.toLocaleString()}
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

          {/* Area Fill */}
          {areaD && <path d={areaD} fill={`url(#${gradId})`} />}

          {/* Line Path */}
          <path
            d={lineD}
            fill="none"
            stroke="var(--primary)"
            strokeWidth="2.2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />

          {/* Deploy Markers (Vertical dashed line + marker tag) */}
          {validDeployments.map((dep, dIdx) => {
            const depTs = dep.created_at || minTs;
            const depX = paddingLeft + ((depTs - minTs) / tsRange) * plotWidth;
            return (
              <g key={`dep-${dIdx}`} className="group cursor-pointer">
                <line
                  x1={depX}
                  y1={paddingTop - 10}
                  x2={depX}
                  y2={paddingTop + plotHeight}
                  stroke="var(--warning, #f59e0b)"
                  strokeWidth="1.5"
                  strokeDasharray="4 3"
                />
                {/* Deploy flag / circle */}
                <circle
                  cx={depX}
                  cy={paddingTop - 10}
                  r="5"
                  fill="var(--warning, #f59e0b)"
                />
                <text
                  x={depX}
                  y={paddingTop - 18}
                  textAnchor="middle"
                  fontSize="9"
                  fontWeight="bold"
                  fill="var(--warning, #f59e0b)"
                >
                  🚀 v{dep.release_id.slice(0, 6)}
                </text>
              </g>
            );
          })}

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

          {/* Interactive Crosshair & Hover Columns */}
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

          {/* Active Hover Point & Crosshair line */}
          {activePoint && (
            <g>
              <line
                x1={activePoint.x}
                y1={paddingTop}
                x2={activePoint.x}
                y2={paddingTop + plotHeight}
                stroke="var(--primary)"
                strokeWidth="1"
                strokeDasharray="2 2"
              />
              <circle
                cx={activePoint.x}
                cy={activePoint.y}
                r="4.5"
                fill="var(--bg)"
                stroke="var(--primary)"
                strokeWidth="2.5"
              />
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
              <span className="text-[10px] text-blue-400 font-mono">
                {activePoint.point.status_2xx + activePoint.point.status_3xx} sukses
              </span>
            </div>
            <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-[11px]">
              <div>
                <span className="text-gray-400">Permintaan:</span>{' '}
                <span className="font-semibold text-white">{activePoint.point.requests.toLocaleString()}</span>
              </div>
              <div>
                <span className="text-gray-400">Pengunjung:</span>{' '}
                <span className="font-semibold text-white">{activePoint.point.visitors.toLocaleString()}</span>
              </div>
              <div>
                <span className="text-gray-400">Bandwidth:</span>{' '}
                <span className="font-semibold text-white">{formatBytes(activePoint.point.bytes)}</span>
              </div>
              <div>
                <span className="text-gray-400">Cache Hit:</span>{' '}
                <span className="font-semibold text-emerald-400">
                  {activePoint.point.requests > 0
                    ? `${Math.round((activePoint.point.cache_hit / activePoint.point.requests) * 100)}%`
                    : '0%'}
                </span>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};
