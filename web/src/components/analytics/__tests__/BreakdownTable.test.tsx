import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { BreakdownTable } from '../BreakdownTable';
import { SiteBreakdownsResponse } from '../../../types/api';
import '../../../i18n';

describe('BreakdownTable Component', () => {
  const mockBreakdowns: SiteBreakdownsResponse = {
    site_id: 1,
    from_ts: 1700000000,
    to_ts: 1700086400,
    paths: [
      { name: '/api/v1/health', count: 1200, percentage: 60.0 },
      { name: '/index.html', count: 800, percentage: 40.0 },
    ],
    referrers: [
      { name: 'google.com', count: 500, percentage: 100.0 },
    ],
    countries: [
      { name: 'ID (Indonesia)', count: 1800, percentage: 90.0 },
      { name: 'SG (Singapore)', count: 200, percentage: 10.0 },
    ],
    browsers: [
      { name: 'Chrome', count: 1500, percentage: 75.0 },
    ],
    os: [
      { name: 'Linux', count: 1000, percentage: 50.0 },
    ],
    devices: [
      { name: 'Desktop', count: 1600, percentage: 80.0 },
    ],
    slow_urls: [
      { name: '/wp-admin/admin-ajax.php', count: 50, percentage: 2.5 },
    ],
    error_urls: [
      { name: '/not-found', count: 15, percentage: 0.75 },
    ],
  };

  it('renders breakdown table header and paths data', () => {
    const html = renderToString(<BreakdownTable breakdowns={mockBreakdowns} />);
    expect(html).toContain('Kenali sumber trafik Anda');
    expect(html).toContain('/api/v1/health');
    expect(html).toContain(mockBreakdowns.paths[0].count.toLocaleString());
    expect(html).toContain('60.0');
  });

  it('renders dimension pills', () => {
    const html = renderToString(<BreakdownTable breakdowns={mockBreakdowns} />);
    expect(html).toContain('Halaman');
    expect(html).toContain('Negara');
    expect(html).toContain('Browser');
    expect(html).toContain('URL lambat');
  });

  it('renders empty fallback when breakdowns is null', () => {
    const html = renderToString(<BreakdownTable breakdowns={null} />);
    expect(html).toContain('Belum ada rincian untuk periode ini');
  });
});
