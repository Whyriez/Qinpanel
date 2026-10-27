import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { UnifiedProcessCard } from '../UnifiedProcessCard';
import '../../../i18n';

describe('UnifiedProcessCard Component', () => {
  it('renders active Node.js process card with telemetry and actions', () => {
    const html = renderToString(
      <UnifiedProcessCard
        siteId={1}
        runtime="nodejs"
        domain="node-app.example.com"
        initialProcess={{
          site_id: 1,
          primary_domain: 'node-app.example.com',
          runtime: 'nodejs',
          version: '20.18.0',
          service_name: 'site-site_1.service',
          active: true,
          sub_state: 'running',
          pid: 4321,
          memory_bytes: 52428800, // 50 MB
          cpu_usage_usec: 2500000, // 2.5s
          uptime_secs: 3665, // 1h 1m 5s
          healthy: true,
          port_or_socket: 'Port 20050',
          probe_message: 'HTTP 200 OK',
        }}
      />
    );

    // Header & Domain
    expect(html).toContain('node-app.example.com');
    expect(html).toContain('nodejs');
    expect(html).toContain('site-site_1.service');

    // Status pill
    expect(html).toContain('Aktif');
    expect(html).toContain('running');
    expect(html).toContain('Sehat');

    // Telemetry
    expect(html).toContain('4321'); // PID
    expect(html).toContain('50.0 MB'); // Memory
    expect(html).toContain('2.50 s'); // CPU
    expect(html).toContain('Port 20050'); // Endpoint
    expect(html).toContain('1j 1m 5d'); // Uptime

    // Actions & Log viewer toggle
    expect(html).toContain('Berhenti');
    expect(html).toContain('Mulai Ulang');
    expect(html).toContain('Deploy Ulang');
    expect(html).toContain('Live Log Aplikasi (Stdout/Stderr)');
  });

  it('renders inactive Python process card with start action', () => {
    const html = renderToString(
      <UnifiedProcessCard
        siteId={2}
        runtime="python"
        domain="py-app.example.com"
        initialProcess={{
          site_id: 2,
          primary_domain: 'py-app.example.com',
          runtime: 'python',
          version: '3.12.9',
          service_name: 'site-site_2.service',
          active: false,
          sub_state: 'inactive',
          pid: null,
          memory_bytes: null,
          cpu_usage_usec: null,
          uptime_secs: null,
          healthy: false,
          port_or_socket: '/run/panel/python/site_2.sock',
          probe_message: 'Socket file not found',
        }}
      />
    );

    expect(html).toContain('py-app.example.com');
    expect(html).toContain('python');
    expect(html).toContain('Nonaktif');
    expect(html).toContain('Belum Siap');
    expect(html).toContain('Mulai');
    expect(html).toContain('/run/panel/python/site_2.sock');
  });
});
