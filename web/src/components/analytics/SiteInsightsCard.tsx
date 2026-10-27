import React, { useState, useEffect, useCallback } from 'react';
import {
  AlertOctagon,
  AlertTriangle,
  Info,
  CheckCircle2,
  RefreshCw,
  ExternalLink,
  X,
  Sparkles,
} from 'lucide-react';
import { fetchSiteInsights, evaluateSiteInsights } from '../../api/client';
import { AnalyticsInsightDto, SiteInsightsResponse } from '../../types/api';

export interface SiteInsightsCardProps {
  siteId: number;
  siteDomain?: string;
  initialData?: SiteInsightsResponse | null;
  onActionClick?: (actionType: string, targetUrl?: string | null) => void;
  className?: string;
}

export const SiteInsightsCard: React.FC<SiteInsightsCardProps> = ({
  siteId,
  siteDomain,
  initialData,
  onActionClick,
  className = '',
}) => {
  const [data, setData] = useState<SiteInsightsResponse | null>(initialData ?? null);
  const [loading, setLoading] = useState<boolean>(initialData === undefined);
  const [evaluating, setEvaluating] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);
  const [dismissedIds, setDismissedIds] = useState<Set<string>>(new Set());

  const loadInsights = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fetchSiteInsights(siteId);
      setData(res);
    } catch (err: unknown) {
      const e = err as Error;
      setError(e.message || 'Gagal memuat insight analitik');
    } finally {
      setLoading(false);
    }
  }, [siteId]);

  const handleEvaluate = async () => {
    setEvaluating(true);
    setError(null);
    try {
      const res = await evaluateSiteInsights(siteId);
      setData(res);
      // Reset dismiss state after manual re-evaluation
      setDismissedIds(new Set());
    } catch (err: unknown) {
      const e = err as Error;
      setError(e.message || 'Gagal mengevaluasi insight analitik');
    } finally {
      setEvaluating(false);
    }
  };

  useEffect(() => {
    if (initialData === undefined) {
      loadInsights();
    }
  }, [initialData, loadInsights]);

  const handleDismiss = (id: string) => {
    setDismissedIds((prev) => new Set(prev).add(id));
  };

  const handleAction = (actionType: string, targetUrl?: string | null) => {
    if (onActionClick) {
      onActionClick(actionType, targetUrl);
    } else if (targetUrl) {
      window.location.href = targetUrl;
    }
  };

  if (loading && !data) {
    return (
      <div className={`p-4 bg-[var(--surface)] border border-[var(--line)] rounded-xl animate-pulse space-y-3 ${className}`}>
        <div className="h-5 bg-[var(--surface-hover)] rounded w-1/4" />
        <div className="h-14 bg-[var(--surface-hover)] rounded" />
      </div>
    );
  }

  const activeInsights = (data?.insights || []).filter(
    (item: AnalyticsInsightDto) => !dismissedIds.has(item.id)
  );

  return (
    <div className={`space-y-3 ${className}`}>
      {/* Header bar */}
      <div className="flex items-center justify-between gap-3 px-1">
        <div className="flex items-center gap-2">
          <Sparkles className="w-4 h-4 text-[var(--primary)]" />
          <h3 className="text-sm font-semibold text-[var(--ink)]">
            Deteksi Anomali & Rekomendasi Kinerja
          </h3>
          {activeInsights.length > 0 && (
            <span className="px-2 py-0.5 text-[11px] font-mono font-medium rounded-full bg-rose-500/10 text-rose-500 border border-rose-500/20">
              {`${activeInsights.length} terdeteksi`}
            </span>
          )}
        </div>

        <button
          type="button"
          onClick={handleEvaluate}
          disabled={evaluating || loading}
          className="px-2.5 py-1 text-xs font-medium rounded-lg border border-[var(--line)] bg-[var(--surface)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface-hover)] transition-colors flex items-center gap-1.5 disabled:opacity-50"
          title="Jalankan analisis aturan deteksi anomali sekarang"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${evaluating ? 'animate-spin text-[var(--primary)]' : ''}`} />
          <span>{evaluating ? 'Menganalisis...' : 'Evaluasi Ulang'}</span>
        </button>
      </div>

      {error && (
        <div className="p-3 bg-rose-500/10 border border-rose-500/20 rounded-xl text-xs text-rose-500 flex items-center justify-between">
          <span>{error}</span>
          <button
            type="button"
            onClick={loadInsights}
            className="font-semibold underline hover:no-underline"
          >
            Coba lagi
          </button>
        </div>
      )}

      {/* No insights - All good state */}
      {activeInsights.length === 0 && !error && (
        <div className="p-4 bg-[var(--surface)] border border-emerald-500/20 rounded-xl flex items-center gap-3 shadow-xs">
          <div className="p-2 bg-emerald-500/10 rounded-lg text-emerald-500 shrink-0">
            <CheckCircle2 className="w-5 h-5" />
          </div>
          <div className="flex-1 min-w-0">
            <div className="text-xs font-semibold text-[var(--ink)]">
              Kondisi Operasional Normal
            </div>
            <div className="text-xs text-[var(--ink-muted)] mt-0.5">
              Tidak ada lonjakan galat 5xx, regresi latensi p95 pasca-deploy, antrean PHP-FPM, atau anomali lalu lintas yang terdeteksi untuk {siteDomain || 'situs ini'}.
            </div>
          </div>
          {data?.evaluated_at && (
            <div className="text-[11px] text-[var(--ink-muted)] shrink-0 hidden sm:block">
              Dievaluasi: {new Date(data.evaluated_at * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
            </div>
          )}
        </div>
      )}

      {/* List of Insights */}
      {activeInsights.length > 0 && (
        <div className="space-y-2.5">
          {activeInsights.map((insight: AnalyticsInsightDto) => {
            const isCritical = insight.severity === 'critical';
            const isWarning = insight.severity === 'warning';

            const borderClass = isCritical
              ? 'border-rose-500/30 bg-rose-500/5'
              : isWarning
              ? 'border-amber-500/30 bg-amber-500/5'
              : 'border-blue-500/30 bg-blue-500/5';

            const badgeBg = isCritical
              ? 'bg-rose-500/10 text-rose-500 border-rose-500/20'
              : isWarning
              ? 'bg-amber-500/10 text-amber-500 border-amber-500/20'
              : 'bg-blue-500/10 text-blue-500 border-blue-500/20';

            const IconComponent = isCritical
              ? AlertOctagon
              : isWarning
              ? AlertTriangle
              : Info;

            const iconColor = isCritical
              ? 'text-rose-500'
              : isWarning
              ? 'text-amber-500'
              : 'text-blue-500';

            return (
              <div
                key={insight.id}
                className={`p-4 border rounded-xl flex flex-col sm:flex-row sm:items-center justify-between gap-3 shadow-xs ${borderClass}`}
              >
                <div className="flex items-start gap-3">
                  <div className={`p-1.5 rounded-lg shrink-0 mt-0.5 bg-[var(--surface)] ${iconColor}`}>
                    <IconComponent className="w-5 h-5" />
                  </div>
                  <div>
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-xs font-semibold text-[var(--ink)]">
                        {insight.title}
                      </span>
                      <span
                        className={`px-2 py-0.5 text-[10px] uppercase font-semibold font-mono rounded border ${badgeBg}`}
                      >
                        {insight.severity}
                      </span>
                    </div>
                    <p className="text-xs text-[var(--ink-muted)] mt-1 max-w-2xl leading-relaxed">
                      {insight.description}
                    </p>
                  </div>
                </div>

                <div className="flex items-center gap-2 self-end sm:self-center shrink-0">
                  {insight.action && (
                    <button
                      type="button"
                      onClick={() =>
                        handleAction(
                          insight.action!.action_type,
                          insight.action!.target_url
                        )
                      }
                      className={`px-3 py-1.5 text-xs font-semibold rounded-lg border transition-colors flex items-center gap-1.5 ${
                        isCritical
                          ? 'bg-rose-500 text-white hover:bg-rose-600 border-rose-600'
                          : 'bg-[var(--surface)] border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface-hover)]'
                      }`}
                    >
                      <span>{insight.action.label}</span>
                      <ExternalLink className="w-3.5 h-3.5" />
                    </button>
                  )}

                  <button
                    type="button"
                    onClick={() => handleDismiss(insight.id)}
                    className="p-1.5 rounded-lg text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface-hover)] transition-colors"
                    title="Abaikan rekomendasi ini sementara"
                  >
                    <X className="w-4 h-4" />
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
