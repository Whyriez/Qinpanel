import { afterEach, describe, expect, it, vi } from 'vitest';
import { setCsrfToken } from './client';
import { cloudflareCommand, checkProjectDns, savePublicAddresses } from './network';

afterEach(() => { vi.unstubAllGlobals(); setCsrfToken(null); });
function capture() {
  const fetcher = vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => ({}) });
  vi.stubGlobal('fetch', fetcher); setCsrfToken('fixture-csrf'); return fetcher;
}
describe('network request contracts', () => {
  it('sends DNS records with the typed kind, priority and explicit identity', async () => {
    const fetcher = capture();
    await cloudflareCommand({ action: 'save_record', zone_id: 'zone', record_id: null, record: { kind:'MX', name:'@', content:'mail.example.com', ttl:300, proxied:false, priority:0 } });
    const [url, options] = fetcher.mock.calls[0];
    expect(url).toBe('/api/v1/network/cloudflare');
    expect(new Headers(options.headers).get('x-csrf-token')).toBe('fixture-csrf');
    expect(JSON.parse(options.body).record).toEqual({ kind:'MX', name:'@', content:'mail.example.com', ttl:300, proxied:false, priority:0 });
  });
  it('publishes a project hostname without an arbitrary origin URL or command', async () => {
    const fetcher = capture();
    await cloudflareCommand({ action:'publish', zone_id:'zone', tunnel_id:'tunnel', hostname:'app.example.com', tls:false });
    expect(JSON.parse(fetcher.mock.calls[0][1].body)).toEqual({ action:'publish', zone_id:'zone', tunnel_id:'tunnel', hostname:'app.example.com', tls:false });
  });
  it('checks an existing project domain by ID', async () => {
    const fetcher = capture(); await checkProjectDns(42);
    expect(fetcher.mock.calls[0][0]).toBe('/api/v1/network/dns-check');
    expect(JSON.parse(fetcher.mock.calls[0][1].body)).toEqual({ domain_id:42 });
  });
  it('preserves empty address values so settings can be cleared', async () => {
    const fetcher = capture(); await savePublicAddresses('', '');
    expect(fetcher.mock.calls[0][1].method).toBe('PUT');
    expect(JSON.parse(fetcher.mock.calls[0][1].body)).toEqual({ ipv4:'', ipv6:'' });
  });
});
