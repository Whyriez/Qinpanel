import React, { useEffect, useState, useId } from "react";
import { fetchSiteAnalyticsTimeseries } from "../../api/client";
import { SiteTimeseriesPointDto } from "../../types/api";

export interface PulseStripProps {
  siteId?: number;
  data?: number[];
  hasError?: boolean;
  className?: string;
  width?: number;
  height?: number;
  showTooltip?: boolean;
  intervalMs?: number;
  responsive?: boolean;
  label?: string;
}

export const PulseStrip: React.FC<PulseStripProps> = ({
  siteId,
  data: initialData,
  hasError: propHasError,
  className = "",
  width = 120,
  height = 24,
  showTooltip = false,
  intervalMs,
  responsive = false,
  label = "Aktivitas real-time sparkline",
}) => {
  const gradientId = useId();
  const [pointsData, setPointsData] = useState<number[]>(initialData || []);
  const [hasError, setHasError] = useState<boolean>(propHasError ?? false);
  const [loading, setLoading] = useState<boolean>(
    siteId !== undefined && !initialData,
  );
  const [hoverIndex, setHoverIndex] = useState<number | null>(null);

  useEffect(() => {
    if (initialData) {
      setPointsData(initialData);
      if (propHasError !== undefined) {
        setHasError(propHasError);
      }
      return;
    }

    if (siteId === undefined) return;

    let isMounted = true;

    async function loadData() {
      try {
        const res = await fetchSiteAnalyticsTimeseries(siteId!, {
          interval: "1m",
        });
        if (!isMounted) return;
        const pts: SiteTimeseriesPointDto[] = res.points || [];
        // Take last 30-60 data points
        const recent = pts.slice(-30);
        const reqs = recent.map((p) => p.requests);
        const anyErrors = recent.some(
          (p) => (p.status_4xx || 0) + (p.status_5xx || 0) > 0,
        );
        setPointsData(reqs.length > 0 ? reqs : [0, 0, 0, 0, 0]);
        setHasError(propHasError ?? anyErrors);
      } catch {
        if (!isMounted) return;
        // On error, fallback to minimal clean baseline
        if (pointsData.length === 0) {
          setPointsData([0, 0, 0, 0, 0]);
        }
      } finally {
        if (isMounted) setLoading(false);
      }
    }

    loadData();

    if (intervalMs && intervalMs > 0) {
      const timer = setInterval(loadData, intervalMs);
      return () => {
        isMounted = false;
        clearInterval(timer);
      };
    }

    return () => {
      isMounted = false;
    };
  }, [siteId, initialData, propHasError, intervalMs]);

  const data = pointsData.length > 0 ? pointsData : [0, 0, 0];
  const max = Math.max(...data, 1);
  const min = Math.min(...data, 0);
  const range = max - min || 1;

  // Build SVG points
  const coords = data.map((val, idx) => {
    const x = data.length > 1 ? (idx / (data.length - 1)) * width : width / 2;
    const y = height - 3 - ((val - min) / range) * (height - 6);
    return { x, y, val };
  });

  const pathD =
    coords.length > 1
      ? `M ${coords.map((c) => `${c.x.toFixed(1)},${c.y.toFixed(1)}`).join(" L ")}`
      : `M 0,${height / 2} L ${width},${height / 2}`;

  const areaD =
    coords.length > 1 ? `${pathD} L ${width},${height} L 0,${height} Z` : "";

  const strokeColor = hasError
    ? "var(--danger, #ef4444)"
    : "var(--primary, #3b82f6)";

  if (loading) {
    return (
      <div
        className={`inline-flex items-center opacity-40 animate-pulse ${className}`}
        style={{ width: responsive ? "100%" : width, height }}
        title="Memuat metrik aktivitas..."
      >
        <svg
          width={responsive ? "100%" : width}
          height={height}
          viewBox={`0 0 ${width} ${height}`}
          className="overflow-hidden"
        >
          <line
            x1="0"
            y1={height / 2}
            x2={width}
            y2={height / 2}
            stroke="var(--line, #cbd5e1)"
            strokeWidth="1.5"
            strokeDasharray="4 2"
          />
        </svg>
      </div>
    );
  }

  const lastCoord = coords[coords.length - 1];

  return (
    <div
      className={`relative inline-flex items-center group ${className}`}
      onMouseLeave={() => setHoverIndex(null)}
      style={{ width: responsive ? "100%" : width, height }}
    >
      <svg
        width={responsive ? "100%" : width}
        viewBox={`0 0 ${width} ${height}`}
        height={height}
        className="overflow-visible"
        aria-label={label}
        role="img"
      >
        <defs>
          <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={strokeColor} stopOpacity="0.25" />
            <stop offset="100%" stopColor={strokeColor} stopOpacity="0.0" />
          </linearGradient>
        </defs>

        {/* Gradient fill beneath line */}
        {areaD && (
          <path
            d={areaD}
            fill={`url(#${gradientId})`}
            className="transition-opacity duration-300 pointer-events-none"
          />
        )}

        {/* Main Line with entrance animation */}
        <path
          d={pathD}
          fill="none"
          stroke={strokeColor}
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
          className="transition-all duration-300"
          style={{
            animation: "pulseStripEntrance 600ms ease-out forwards",
          }}
        />

        {/* Pulsing latest point */}
        {lastCoord && (
          <g
            transform={`translate(${lastCoord.x.toFixed(1)}, ${lastCoord.y.toFixed(1)})`}
          >
            <circle r="4.5" fill={strokeColor} opacity="0.3" />
            <circle r="2.5" fill={strokeColor} />
          </g>
        )}

        {/* Hover interactive markers */}
        {showTooltip &&
          coords.map((c, i) => (
            <rect
              key={i}
              x={c.x - width / data.length / 2}
              y={0}
              width={width / data.length}
              height={height}
              fill="transparent"
              className="cursor-crosshair"
              onMouseEnter={() => setHoverIndex(i)}
            />
          ))}

        {hoverIndex !== null && coords[hoverIndex] && (
          <circle
            cx={coords[hoverIndex].x}
            cy={coords[hoverIndex].y}
            r="3"
            fill="var(--bg, #ffffff)"
            stroke={strokeColor}
            strokeWidth="1.5"
          />
        )}
      </svg>

      {/* Tooltip */}
      {showTooltip && hoverIndex !== null && coords[hoverIndex] && (
        <div
          className="absolute z-10 px-2 py-1 text-xs rounded shadow-lg pointer-events-none whitespace-nowrap"
          style={{
            left: `${Math.min(Math.max(coords[hoverIndex].x - 20, 0), width - 40)}px`,
            bottom: `${height + 4}px`,
            backgroundColor: "var(--surface-overlay, #1e293b)",
            color: "var(--ink, #f8fafc)",
            border: "1px solid var(--line, #334155)",
          }}
        >
          {coords[hoverIndex].val} req
        </div>
      )}

      <style>{`
        @keyframes pulseStripEntrance {
          0% {
            opacity: 0.2;
            transform: scaleY(0.6);
          }
          100% {
            opacity: 1;
            transform: scaleY(1);
          }
        }
      `}</style>
    </div>
  );
};
