import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { ConnectivitySummary } from '../ConnectivitySettings';
import { availableTransports, endpointAuthority, endpointConfig, isCurrentEvidence, parseConnectivity, presetConfig, type ConnectivityOverview, type Endpoint, type Evidence } from '../../../api/connectivity';
import '../../../i18n';

const now = 2000000000000;
const endpoint: Endpoint = { id: 'endpoint-one', revision: 3, service: 'ssh', scope: { kind: 'project', site_id: 1 }, transport: 'cloudflare_access', host: '2001:db8::7', port: 2222, tls_mode: 'ssh', certificate_id: null, enabled: true, primary: true, client_requirement: 'cloudflared', created_at: 1, updated_at: 2 };
const report: Evidence = { id: 1, endpoint_id: endpoint.id, service: 'ssh', endpoint_revision: 3, kind: 'service', observation_point: 'local_server', observer_id: 'server', address_family: null, observed_at: now / 1000 - 10, expires_at: now / 1000 + 100, outcome: 'pass', reason: 'verified', current: true, expired: false, revision_matches: true, suggested_action: 'Tidak ada tindakan.' };
function overview(evidence: Evidence[] = []): ConnectivityOverview { return { can_manage: false, page: 1, per_page: 25, total: 1, total_pages: 1, service_counts: { ssh: 1 }, endpoints: [{ endpoint, evidence }], projects: [{ id: 1, name: 'app.example.test' }], certificates: [] }; }

describe('Connectivity summary and contract', () => {
  it('renders IPv6, custom port, project and client requirements without admin actions', () => {
    const html = renderToStaticMarkup(<ConnectivitySummary data={overview()} now={now} />);
    expect(html).toContain('[2001:db8::7]:2222'); expect(html).toContain('app.example.test');
    expect(html).toContain('Memerlukan cloudflared'); expect(html).toContain('Belum diuji dari luar');
    expect(html).not.toContain('>Ubah'); expect(html).not.toContain('>Hapus');
    expect(html).not.toContain('>Periksa'); expect(html).toContain('Administrator dapat menjalankan pemeriksaan');
  });
  it('enables registered admin checks and disables pending or inactive endpoints', () => {
    const data = overview(); data.can_manage = true;
    const html = renderToStaticMarkup(<ConnectivitySummary data={data} manage onCheck={() => {}} now={now} />);
    expect(html).toContain('>Periksa'); expect(html).toContain('Pemeriksaan dari server ini tidak membuktikan akses internet');
    const pending = renderToStaticMarkup(<ConnectivitySummary data={data} manage disabled checkingId={endpoint.id} onCheck={() => {}} now={now} />);
    expect(pending).toContain('Memeriksa'); expect(pending).toContain('aria-busy="true"');
    data.endpoints[0].endpoint = { ...endpoint, enabled: false };
    const inactive = renderToStaticMarkup(<ConnectivitySummary data={data} manage onCheck={() => {}} now={now} />);
    expect(inactive).toContain('disabled=""');
  });
  it('shows listener firewall DNS and TLS independently without a ready path claim', () => {
    const data = overview([
      { ...report, kind: 'listener', address_family: 'ipv4' },
      { ...report, id: 2, kind: 'firewall', outcome: 'unknown', reason: 'local_guard_only' },
      { ...report, id: 3, kind: 'tls', outcome: 'fail', reason: 'tls_failure' },
    ]);
    const html = renderToStaticMarkup(<ConnectivitySummary data={data} now={now} />);
    expect(html).toContain('Listener lokal (termasuk loopback): Terdeteksi');
    expect(html).toContain('Firewall milik panel: Belum diketahui'); expect(html).toContain('TLS: Perlu tindakan');
    expect(html).toContain('Belum diuji dari luar'); expect(html).not.toContain('Jalur siap');
  });
  it('does not turn a local service pass into an external connection claim', () => {
    const html = renderToStaticMarkup(<ConnectivitySummary data={overview([report])} now={now} />);
    expect(html).toContain('Layanan lokal: Berjalan'); expect(html).toContain('Belum diuji dari luar');
    expect(html).not.toContain('Terjangkau dari pengamat ini');
  });
  it('keeps IPv4 success and IPv6 failure visible as different evidence', () => {
    const ipv4: Evidence = { ...report, kind: 'external_reachability', observation_point: 'external_probe', address_family: 'ipv4' };
    const ipv6: Evidence = { ...ipv4, id: 2, address_family: 'ipv6', outcome: 'fail', reason: 'port_blocked' };
    const html = renderToStaticMarkup(<ConnectivitySummary data={overview([ipv4, ipv6])} now={now} />);
    expect(html).toContain('IPV4'); expect(html).toContain('IPV6'); expect(html).toContain('Terjangkau dari pengamat ini'); expect(html).toContain('Perlu tindakan');
  });
  it('marks expired and old revisions while excluding them from readiness', () => {
    const expired = { ...report, expires_at: now / 1000 };
    const old = { ...report, id: 2, endpoint_revision: 2 };
    const html = renderToStaticMarkup(<ConnectivitySummary data={overview([expired, old])} now={now} />);
    expect(html).toContain('Kedaluwarsa'); expect(html).toContain('Konfigurasi telah berubah');
    expect(html).not.toContain('Layanan lokal: Berjalan');
    expect(isCurrentEvidence({ ...endpoint, enabled: false }, report, now)).toBe(false);
    expect(isCurrentEvidence(endpoint, report, now)).toBe(true);
  });
  it('uses draft-only presets with service-appropriate routes and no inferred host', () => {
    expect(presetConfig('website', 'mixed').transport).toBe('cloudflare_http');
    expect(presetConfig('ssh', 'mixed').transport).toBe('direct');
    expect(presetConfig('ssh', 'tunnel').transport).toBe('cloudflare_access');
    expect(presetConfig('imap', 'tunnel').transport).toBe('provider');
    expect(presetConfig('submission', 'tunnel').tls_mode).toBe('starttls');
    expect(presetConfig('smtp_inbound', 'direct').tls_mode).toBe('starttls');
    expect(presetConfig('database', 'mixed').transport).toBe('private_network');
    expect(presetConfig('website', 'direct')).not.toHaveProperty('host');
    expect(availableTransports('ssh')).not.toContain('cloudflare_http');
    expect(endpointAuthority(endpoint)).toBe('[2001:db8::7]:2222');
  });
  it('omits read-only metadata from replacements and rejects malformed responses', () => {
    const config = endpointConfig(endpoint);
    expect(config).not.toHaveProperty('id'); expect(config).not.toHaveProperty('revision'); expect(config).not.toHaveProperty('client_requirement');
    expect(parseConnectivity(overview([report]))).toEqual(overview([report]));
    expect(() => parseConnectivity({ endpoints: [] })).toThrow('Data konektivitas tidak valid');
    const invalid = overview(); invalid.endpoints[0].endpoint = { ...endpoint, port: 0 };
    expect(() => parseConnectivity(invalid)).toThrow();
  });
  it('does not label off-page endpoints as unconfigured', () => {
    const data = overview(); data.endpoints = [];
    const html = renderToStaticMarkup(<ConnectivitySummary data={data} now={now} filter="ssh" />);
    expect(html).toContain('1 alamat terdaftar pada halaman lain');
    expect(html).not.toContain('Belum diatur');
  });
});
