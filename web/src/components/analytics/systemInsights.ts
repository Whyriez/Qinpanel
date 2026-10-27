import type { AnalyticsInsightDto, Site, SystemInsightsResponse } from '../../types/api';

export function summarizeSystemInsights(response: SystemInsightsResponse, sites: Site[]) {
  const groups = new Map<number | null, { site_id: number | null; site_name: string; can_review: boolean; insights: AnalyticsInsightDto[] }>();
  for (const insight of response.insights) {
    const siteId = insight.site_id ?? null;
    const site = sites.find(item => item.id === siteId);
    let group = groups.get(siteId);
    if (!group) {
      group = { site_id: siteId, site_name: site?.primary_domain || (siteId === null ? 'Server host' : `Situs #${siteId}`), can_review: Boolean(site), insights: [] };
      groups.set(siteId, group);
    }
    group.insights.push(insight);
  }
  return {
    total_critical: response.insights.filter(item => item.severity === 'critical').length,
    total_warning: response.insights.filter(item => item.severity === 'warning').length,
    site_insights: [...groups.values()],
  };
}
