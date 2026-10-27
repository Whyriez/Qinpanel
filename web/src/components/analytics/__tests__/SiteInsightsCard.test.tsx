import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SiteInsightsCard } from '../SiteInsightsCard';
import { SiteInsightsResponse } from '../../../types/api';

describe('SiteInsightsCard Component', () => {
  const healthyResponse: SiteInsightsResponse = {
    site_id: 1,
    site_name: 'testsite.id',
    insights: [],
    evaluated_at: 1700000000,
  };

  const anomalyResponse: SiteInsightsResponse = {
    site_id: 1,
    site_name: 'testsite.id',
    evaluated_at: 1700000000,
    insights: [
      {
        id: '1-five_xx_spike-1700000000',
        insight_type: 'five_xx_spike',
        severity: 'critical',
        title: 'Lonjakan Galat 5xx Terdeteksi',
        description: 'Tingkat error 5xx mencapai 12.5% (25 galat dari 200 permintaan).',
        detected_at: 1700000000,
        action: {
          label: 'Periksa Log Error',
          action_type: 'view_logs',
          target_url: '/sites/1/logs',
        },
      },
      {
        id: '1-deploy_latency_regression-1700000000',
        insight_type: 'deploy_latency_regression',
        severity: 'critical',
        title: 'Regresi Latensi p95 Pasca-Deploy',
        description: 'Latensi p95 melonjak dari 100ms menjadi 550ms (+450.0%) pasca deployment.',
        detected_at: 1700000000,
        action: {
          label: 'Rollback Deploy',
          action_type: 'rollback_deploy',
          target_url: '/sites/1?tab=deployments',
        },
      },
      {
        id: '1-bot_spike-1700000000',
        insight_type: 'bot_spike',
        severity: 'warning',
        title: 'Lonjakan Aktivitas Bot Terdeteksi',
        description: 'Trafik bot menyumbang 55.0% dari total lalu lintas.',
        detected_at: 1700000000,
        action: {
          label: 'Tinjau Aturan Akses',
          action_type: 'view_rules',
          target_url: '/sites/1?tab=rules',
        },
      },
    ],
  };

  it('renders operational normal banner when no insights are detected', () => {
    const html = renderToString(
      <SiteInsightsCard siteId={1} siteDomain="testsite.id" initialData={healthyResponse} />
    );
    expect(html).toContain('Kondisi Operasional Normal');
    expect(html).toContain('Tidak ada lonjakan galat 5xx');
    expect(html).toContain('testsite.id');
    expect(html).toContain('Deteksi Anomali &amp; Rekomendasi Kinerja');
  });

  it('renders anomaly cards with critical and warning badges and action buttons', () => {
    const html = renderToString(
      <SiteInsightsCard siteId={1} siteDomain="testsite.id" initialData={anomalyResponse} />
    );
    expect(html).toContain('Lonjakan Galat 5xx Terdeteksi');
    expect(html).toContain('Regresi Latensi p95 Pasca-Deploy');
    expect(html).toContain('Lonjakan Aktivitas Bot Terdeteksi');
    expect(html).toContain('Periksa Log Error');
    expect(html).toContain('Rollback Deploy');
    expect(html).toContain('Tinjau Aturan Akses');
    expect(html).toContain('critical');
    expect(html).toContain('warning');
    expect(html).toContain('3 terdeteksi');
  });

  it('renders skeleton when loading with undefined initialData', () => {
    const html = renderToString(
      <SiteInsightsCard siteId={1} siteDomain="testsite.id" initialData={undefined} />
    );
    expect(html).toContain('animate-pulse');
  });
});
