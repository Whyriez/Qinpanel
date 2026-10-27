import { afterEach, describe, expect, it, vi } from 'vitest';
import { fetchOptimizerStatus } from './client';
import type { PackageDemand } from '../types/api';

const demand: PackageDemand = { usage: 'multi_project', projects_per_tenant: 3, tenant_slots: 1 };
afterEach(() => { vi.unstubAllGlobals(); });

describe('read-only package capacity client', () => {
  it('keeps legacy status requests working without fabricating a capacity preview', async () => {
    const legacy = { hardware: { vcpu_count: 2, ram_total_mb: 2048 } };
    const fetch = vi.fn().mockResolvedValue(Response.json(legacy));
    vi.stubGlobal('fetch', fetch);
    const status = await fetchOptimizerStatus();
    expect(status).toEqual(legacy);
    expect(status.capacity).toBeUndefined();
    expect(status.package_preset).toBeUndefined();
    expect(fetch.mock.calls[0][0]).toBe('/api/v1/system/optimizer');
  });

  it('sends an explicit demand through a GET without a mutation or request body', async () => {
    const fetch = vi.fn().mockResolvedValue(Response.json({ package_preset: { demand, rules_version: 1 } }));
    vi.stubGlobal('fetch', fetch);
    const status = await fetchOptimizerStatus(demand);
    expect(status.package_preset?.demand).toEqual(demand);
    const [url, options] = fetch.mock.calls[0];
    expect(url).toBe('/api/v1/system/optimizer?capacity_usage=multi_project&projects_per_tenant=3&tenant_slots=1');
    expect(options.method ?? 'GET').toBe('GET');
    expect(options.body).toBeUndefined();
    expect(options.credentials).toBe('include');
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it('rejects an old API response that silently ignores the preview query', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({ hardware: { vcpu_count: 2 } })));
    await expect(fetchOptimizerStatus(demand)).rejects.toThrow('Perbarui paket Qinpanel lengkap');
  });

  it('preserves unavailable/conflict responses and does not fall back to guessed hardware', async () => {
    for (const status of [409, 503]) {
      const fetch = vi.fn().mockResolvedValue(Response.json({ code: 'capacity_unavailable', detail: 'Data kapasitas belum tersedia.' }, { status }));
      vi.stubGlobal('fetch', fetch);
      await expect(fetchOptimizerStatus(demand)).rejects.toMatchObject({ message: 'Data kapasitas belum tersedia.', status });
      expect(fetch).toHaveBeenCalledTimes(1);
    }
  });
});
