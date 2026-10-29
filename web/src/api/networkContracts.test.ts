import { afterEach, describe, expect, it, vi } from 'vitest';
import { setCsrfToken } from './client';
import { cloudflareCommand, checkProjectDns, fetchCloudflareStatus, savePublicAddresses } from './network';

afterEach(() => { vi.unstubAllGlobals(); setCsrfToken(null); });
function capture() {
  const fetcher = vi.fn().mockResolvedValue({ ok: true, status: 200, json: async () => ({}) });
  vi.stubGlobal('fetch', fetcher); setCsrfToken('fixture-csrf'); return fetcher;
}
describe('network request contracts', () => {
  it('reads status with GET and cancellation, without a command body or connector mutation', async () => {
    const value = { configured: true, account_id: 'fixture-account', connector_installed: true, managed_tunnels: [{ id: 'fixture-tunnel', local_status: 'active', routes: [] }] };
    const fetcher = vi.fn().mockResolvedValue(Response.json(value)); vi.stubGlobal('fetch', fetcher);
    const controller = new AbortController();
    expect(await fetchCloudflareStatus(controller.signal)).toEqual(value);
    expect(fetcher).toHaveBeenCalledTimes(1);
    const [url, options] = fetcher.mock.calls[0];
    expect(url).toBe('/api/v1/network/cloudflare');
    expect(options.method).toBeUndefined(); expect(options.body).toBeUndefined(); expect(options.signal).toBe(controller.signal);
  });
  it.each([502, 504])('explains an HTML gateway failure %i without retrying or reinstalling Tunnel', async status => {
    const fetcher = vi.fn().mockResolvedValue(new Response('<html>Cloudflare gateway</html>', { status })); vi.stubGlobal('fetch', fetcher);
    await expect(fetchCloudflareStatus()).rejects.toThrow('Status Cloudflare belum dapat diperbarui');
    expect(fetcher).toHaveBeenCalledTimes(1);
  });
  it('keeps actionable JSON service errors visible and rejects malformed status', async () => {
    const fetcher = vi.fn().mockResolvedValueOnce(Response.json({ error: 'Koneksi panel-api ke panel-agent belum tersedia.' }, { status: 503 }))
      .mockResolvedValueOnce(Response.json({ configured: false }))
      .mockResolvedValueOnce(Response.json({ configured: true, connector_installed: true, account_id: 'fixture', managed_tunnels: [{ id: 'tunnel', local_status: 'active', routes: [null] }] }));
    vi.stubGlobal('fetch', fetcher);
    await expect(fetchCloudflareStatus()).rejects.toThrow('panel-agent');
    await expect(fetchCloudflareStatus()).rejects.toThrow('Respons status Cloudflare tidak valid');
    await expect(fetchCloudflareStatus()).rejects.toThrow('Respons status Cloudflare tidak valid');
  });
  it('reports a disconnected request without converting it to an unconfigured account', async () => {
    const fetcher = vi.fn().mockRejectedValue(new TypeError('Failed to fetch')); vi.stubGlobal('fetch', fetcher);
    await expect(fetchCloudflareStatus()).rejects.toThrow('koneksi panel terputus');
    expect(fetcher).toHaveBeenCalledTimes(1);
  });
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
