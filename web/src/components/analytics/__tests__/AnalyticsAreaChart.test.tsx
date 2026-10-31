import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { AnalyticsAreaChart } from '../AnalyticsAreaChart';
import { SiteTimeseriesPointDto, SiteDeployment } from '../../../types/api';
import '../../../i18n';

describe('AnalyticsAreaChart Component', () => {
  const mockPoints: SiteTimeseriesPointDto[] = [
    {
      timestamp: 1700000000,
      requests: 120,
      bytes: 204800,
      status_2xx: 110,
      status_3xx: 5,
      status_4xx: 4,
      status_5xx: 1,
      visitors: 85,
      cache_hit: 90,
      cache_miss: 30,
      latency_p50_ms: 25,
      latency_p95_ms: 120,
      latency_p99_ms: 250,
      latency_avg_ms: 45,
    },
    {
      timestamp: 1700003600,
      requests: 250,
      bytes: 512000,
      status_2xx: 240,
      status_3xx: 6,
      status_4xx: 3,
      status_5xx: 1,
      visitors: 180,
      cache_hit: 200,
      cache_miss: 50,
      latency_p50_ms: 30,
      latency_p95_ms: 140,
      latency_p99_ms: 290,
      latency_avg_ms: 50,
    },
  ];

  it('renders SVG area chart and path elements', () => {
    const html = renderToString(
      <AnalyticsAreaChart points={mockPoints} title="Uji Coba Trafik" />
    );
    expect(html).toContain('Uji Coba Trafik');
    expect(html).toContain('<svg');
    expect(html).toContain('role="img"');
    expect(html).toContain('linearGradient');
  });

  it('renders deploy markers when within timestamp range', () => {
    const mockDeploy: SiteDeployment = {
      id: 1,
      site_id: 10,
      release_id: 'rel999abcdef',
      branch: 'main',
      status: 'active',
      trigger_type: 'manual',
      is_current: true,
      created_at: 1700001800,
    };

    const html = renderToString(
      <AnalyticsAreaChart
        points={mockPoints}
        deployments={[mockDeploy]}
      />
    );
    expect(html).toContain('rel999');
    expect(html).toContain('Deploy rel999');
  });

  it('renders empty message when points is empty', () => {
    const html = renderToString(<AnalyticsAreaChart points={[]} />);
    expect(html).toContain('Belum ada data metrik');
  });
});
