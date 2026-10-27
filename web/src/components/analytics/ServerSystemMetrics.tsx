import React, { useState } from 'react';
import { ServerTimeseriesPointDto } from '../../types/api';

export interface ServerSystemMetricsProps {
  points: ServerTimeseriesPointDto[];
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

export const ServerSystemMetrics: React.FC<ServerSystemMetricsProps> = ({
  points,
  className = '',
}) => {
  const [metricTab, setMetricTab] = useState<'cpu_ram' | 'load_io' | 'network'>('cpu_ram');

  if (!points || points.length === 0) {
    return (
      <div className={`p-8 text-center text-sm text-[var(--ink-muted)] border border-[var(--line)] rounded-xl bg-[var(--surface)] ${className}`}>
        Belum ada data metrik server sistem yang tercatat.
      </div>
    );
  }

  const svgWidth = 800;
  const height = 220;
  const paddingLeft = 55;
  const paddingRight = 20;
  const paddingTop = 20;
  const paddingBottom = 30;

  const plotWidth = svgWidth - paddingLeft - paddingRight;
  const plotHeight = height - paddingTop - paddingBottom;

  // Render chart based on selected tab
  let maxVal = 100;
  let unit = '%';

  if (metricTab === 'cpu_ram') {
    maxVal = 100; // Percentage for CPU
  } else if (metricTab === 'load_io') {
    maxVal = Math.max(...points.map((p) => p.load1_max || p.load1_avg || 1), 4);
    unit = '';
  } else if (metricTab === 'network') {
    maxVal = Math.max(...points.map((p) => Math.max(p.net_rx_avg_bytes_sec || 0, p.net_tx_avg_bytes_sec || 0)), 1024);
    unit = 'B/s';
  }

  const coords = points.map((p, idx) => {
    const x = paddingLeft + (idx / (points.length - 1 || 1)) * plotWidth;
    let y1 = 0;
    let y2 = 0;

    if (metricTab === 'cpu_ram') {
      y1 = paddingTop + plotHeight - ((p.cpu_avg || 0) / 100) * plotHeight;
      y2 = paddingTop + plotHeight - ((p.cpu_max || 0) / 100) * plotHeight;
    } else if (metricTab === 'load_io') {
      y1 = paddingTop + plotHeight - ((p.load1_avg || 0) / maxVal) * plotHeight;
      y2 = paddingTop + plotHeight - ((p.load1_max || 0) / maxVal) * plotHeight;
    } else {
      y1 = paddingTop + plotHeight - ((p.net_rx_avg_bytes_sec || 0) / maxVal) * plotHeight;
      y2 = paddingTop + plotHeight - ((p.net_tx_avg_bytes_sec || 0) / maxVal) * plotHeight;
    }

    return { x, y1, y2, point: p };
  });

  const line1D = `M ${coords.map((c) => `${c.x.toFixed(1)},${c.y1.toFixed(1)}`).join(' L ')}`;
  const line2D = `M ${coords.map((c) => `${c.x.toFixed(1)},${c.y2.toFixed(1)}`).join(' L ')}`;

  const xTickIndices = [0, Math.floor(points.length * 0.5), points.length - 1];
  const uniqueXTicks = Array.from(new Set(xTickIndices)).filter((i) => i >= 0 && i < points.length);

  return (
    <div className={`bg-[var(--surface)] border border-[var(--line)] rounded-xl p-5 shadow-sm ${className}`}>
      <div className="flex flex-wrap items-center justify-between gap-3 mb-4">
        <div>
          <h3 className="text-base font-semibold text-[var(--ink)]">
            Metrik Sistem Server (Admin)
          </h3>
          <p className="text-xs text-[var(--ink-muted)]">
            Observabilitas beban host, penggunaan memori, load average, dan bandwidth
          </p>
        </div>

        {/* Tab selector */}
        <div className="flex items-center gap-1 bg-[var(--bg)] p-1 rounded-lg border border-[var(--line)] text-xs">
          <button
            type="button"
            onClick={() => setMetricTab('cpu_ram')}
            className={`px-3 py-1 rounded-md font-medium transition-colors ${
              metricTab === 'cpu_ram'
                ? 'bg-[var(--primary)] text-white shadow-xs'
                : 'text-[var(--ink-muted)] hover:text-[var(--ink)]'
            }`}
          >
            CPU (Avg & Peak)
          </button>
          <button
            type="button"
            onClick={() => setMetricTab('load_io')}
            className={`px-3 py-1 rounded-md font-medium transition-colors ${
              metricTab === 'load_io'
                ? 'bg-[var(--primary)] text-white shadow-xs'
                : 'text-[var(--ink-muted)] hover:text-[var(--ink)]'
            }`}
          >
            Load Average
          </button>
          <button
            type="button"
            onClick={() => setMetricTab('network')}
            className={`px-3 py-1 rounded-md font-medium transition-colors ${
              metricTab === 'network'
                ? 'bg-[var(--primary)] text-white shadow-xs'
                : 'text-[var(--ink-muted)] hover:text-[var(--ink)]'
            }`}
          >
            Jaringan (RX / TX)
          </button>
        </div>
      </div>

      {/* SVG Chart */}
      <div className="relative w-full overflow-hidden">
        <svg
          viewBox={`0 0 ${svgWidth} ${height}`}
          className="w-full h-auto select-none"
          role="img"
          aria-label="Metrik server timeseries"
        >
          {/* Grid lines */}
          {[0, 0.5, 1].map((pct, idx) => {
            const y = paddingTop + plotHeight - pct * plotHeight;
            const val = pct * maxVal;
            return (
              <g key={idx}>
                <line
                  x1={paddingLeft}
                  y1={y}
                  x2={svgWidth - paddingRight}
                  y2={y}
                  stroke="var(--line)"
                  strokeDasharray="3 3"
                  strokeOpacity="0.6"
                />
                <text
                  x={paddingLeft - 8}
                  y={y + 4}
                  textAnchor="end"
                  fontSize="10"
                  fill="var(--ink-muted)"
                >
                  {metricTab === 'network' ? formatBytes(val) : `${val.toFixed(1)}${unit}`}
                </text>
              </g>
            );
          })}

          {/* Line 2 (Peak / TX) */}
          <path
            d={line2D}
            fill="none"
            stroke={metricTab === 'network' ? '#f59e0b' : '#ef4444'}
            strokeWidth="1.75"
            strokeDasharray="3 2"
            strokeLinecap="round"
          />

          {/* Line 1 (Avg / RX) */}
          <path
            d={line1D}
            fill="none"
            stroke="#3b82f6"
            strokeWidth="2.2"
            strokeLinecap="round"
          />

          {/* X Axis */}
          <line
            x1={paddingLeft}
            y1={paddingTop + plotHeight}
            x2={svgWidth - paddingRight}
            y2={paddingTop + plotHeight}
            stroke="var(--line)"
          />

          {/* X Labels */}
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

        {/* Legend */}
        <div className="flex items-center justify-end gap-5 mt-2 text-xs">
          <div className="flex items-center gap-1.5">
            <span className="w-3 h-0.5 bg-blue-500 rounded-full" />
            <span className="text-[var(--ink-muted)]">
              {metricTab === 'cpu_ram' ? 'Rata-rata CPU' : metricTab === 'load_io' ? 'Load 1m Avg' : 'Net RX (Unduh)'}
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-3 h-0.5 bg-rose-500 rounded-full border-dashed" />
            <span className="text-[var(--ink-muted)]">
              {metricTab === 'cpu_ram' ? 'Puncak CPU' : metricTab === 'load_io' ? 'Load 1m Max' : 'Net TX (Unggah)'}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
};
