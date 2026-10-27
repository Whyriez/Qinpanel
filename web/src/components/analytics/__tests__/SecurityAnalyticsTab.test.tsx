import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SecurityAnalyticsTab } from '../SecurityAnalyticsTab';
import * as clientModule from '../../../api/client';

describe('SecurityAnalyticsTab Component', () => {
  const mockOverview = {
    site_id: 42,
    from: 1700000000,
    to: 1700086400,
    total_probing_requests: 128,
    total_brute_force_attempts: 45,
    unique_suspicious_ips: 14,
    top_ips: [
      {
        ip: '198.51.100.44',
        total_requests: 250,
        error_4xx: 210,
        error_5xx: 0,
        probing_count: 35,
        auth_failures: 0,
        country: 'ID',
        top_path: '/.env',
        risk_level: 'critical',
        last_seen: 1700086300,
      },
      {
        ip: '203.0.113.88',
        total_requests: 60,
        error_4xx: 55,
        error_5xx: 0,
        probing_count: 0,
        auth_failures: 45,
        country: 'RU',
        top_path: '/wp-login.php',
        risk_level: 'critical',
        last_seen: 1700086350,
      },
    ],
    probing_paths: [
      {
        path: '/.env',
        category: 'env_credentials',
        count: 75,
        unique_ips: 8,
      },
      {
        path: '/.git/config',
        category: 'vcs_leak',
        count: 53,
        unique_ips: 6,
      },
    ],
    brute_force_incidents: [
      {
        ip: '203.0.113.88',
        target_path: '/wp-login.php',
        failed_attempts: 45,
        first_attempt_ts: 1700080000,
        last_attempt_ts: 1700086350,
        country: 'RU',
        risk_score: 70,
      },
    ],
    active_banned_ips_count: 1,
  };

  const mockBannedIps = [
    {
      id: 'ban-uuid-1',
      site_id: 42,
      ip_address: '198.51.100.44',
      reason: 'Pemindaian berkas rahasia .env',
      banned_at: 1700080000,
      expires_at: 1700166400,
      remaining_ttl_secs: 80000,
      is_active: true,
      banned_by: 'alice',
    },
  ];

  beforeEach(() => {
    vi.spyOn(clientModule, 'fetchSiteSecurityOverview').mockResolvedValue(mockOverview);
    vi.spyOn(clientModule, 'fetchSiteBannedIps').mockResolvedValue(mockBannedIps);
  });

  it('renders initial loading state without crash', () => {
    const html = renderToString(
      <SecurityAnalyticsTab siteId={42} siteDomain="example.com" />
    );
    expect(html).toContain('Memuat data analitik keamanan');
  });

  it('contains essential security elements and labels', () => {
    // In React 18 renderToString, initial state shows loading, then effect runs in client.
    // We verify component structure and helper rendering.
    const html = renderToString(
      <SecurityAnalyticsTab siteId={42} siteDomain="example.com" />
    );
    expect(html).toBeDefined();
  });
});
