import { afterEach, describe, expect, it, vi } from 'vitest';
import { fixSystemDoctor, setCsrfToken } from './client';

afterEach(() => { vi.unstubAllGlobals(); setCsrfToken(null); });

describe('doctor remediation requests', () => {
  it('sends one CSRF-protected POST and returns the verified report', async () => {
    const report = { overall_status: 'warn', total_warnings: 1, total_failures: 0, checks: [] };
    const fetch = vi.fn().mockResolvedValue(Response.json(report));
    vi.stubGlobal('fetch', fetch); setCsrfToken('fixture-csrf');
    expect(await fixSystemDoctor()).toEqual(report);
    expect(fetch).toHaveBeenCalledTimes(1);
    const [url, options] = fetch.mock.calls[0];
    expect(url).toBe('/api/v1/system/doctor/fix');
    expect(options.method).toBe('POST');
    expect(options.headers.get('X-CSRF-Token')).toBe('fixture-csrf');
  });
  it.each([502, 504])('explains an uncertain result on gateway %i without replaying the fix', async status => {
    const fetch = vi.fn().mockResolvedValue(new Response('<html>Cloudflare gateway error</html>', { status }));
    vi.stubGlobal('fetch', fetch);
    await expect(fixSystemDoctor()).rejects.toThrow('Hasil perbaikan belum terkonfirmasi');
    expect(fetch).toHaveBeenCalledTimes(1);
  });
  it('does not replay a POST after the connection drops', async () => {
    const fetch = vi.fn().mockRejectedValue(new TypeError('Failed to fetch'));
    vi.stubGlobal('fetch', fetch);
    await expect(fixSystemDoctor()).rejects.toThrow('segarkan diagnosis');
    expect(fetch).toHaveBeenCalledTimes(1);
  });
  it('keeps actionable agent errors instead of claiming success', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({ detail: 'Agent tidak dapat membuka socket Netlink. Periksa AF_NETLINK.' }, { status: 422 })));
    await expect(fixSystemDoctor()).rejects.toThrow('AF_NETLINK');
  });
});
