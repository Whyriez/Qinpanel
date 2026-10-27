import { afterEach, describe, expect, it, vi } from 'vitest';
import { fetchDatabases, fetchAvailableDatabaseEngines, fetchSiteAppMetrics, rotateDatabasePassword, setCsrfToken } from './client';
import { parseDatabaseList } from './databaseContracts';

const database = { id: 7, user_id: 2, site_id: 1, engine: 'mariadb', name: 'p_alice_shop', db_user: 'p_alice_shop', remote_access: false, allowed_hosts: 'localhost', collation: 'utf8mb4_unicode_ci', character_set: 'utf8mb4', size_bytes: 0, created_at: 1770000000, updated_at: 1770000000 };
// Wire shape from panel_api::databases::ListDatabasesResponse, not the UI model.
const response = { databases: [database], total: 1, plan_max_databases: 2, used_databases: 2 };
afterEach(() => { vi.unstubAllGlobals(); setCsrfToken(null); });

describe('Database API contracts', () => {
  it('loads tenant-safe capabilities from the live catalog endpoint', async () => {
    const data = [{ engine: 'postgresql', label: 'PostgreSQL', port: 5432 }];
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(data)));
    vi.stubGlobal('fetch', fetchMock);
    expect(await fetchAvailableDatabaseEngines()).toEqual(data);
    expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/databases/engines');
  });
  it.each([{}, null, [{ engine: 'mariadb' }], [{ engine: 'mariadb', label: 'MariaDB', port: 0 }], [{ engine: 'x', label: 'X', port: 1 }, { engine: 'x', label: 'X', port: 1 }]])('rejects malformed engine capabilities %#', async value => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify(value))));
    await expect(fetchAvailableDatabaseEngines()).rejects.toThrow('Format pilihan mesin');
  });
  it('maps the real flat quota fields before updating UI state', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(response)));
    vi.stubGlobal('fetch', fetchMock);
    const data = await fetchDatabases({ site_id: 1 });
    expect(fetchMock.mock.calls[0][0]).toBe('/api/v1/databases?site_id=1');
    expect(data.databases).toEqual([database]);
    expect(data.quota).toEqual({ used: 2, limit: 2, remaining: 0 });
    expect(data.total).toBe(1);
  });
  it('keeps tenant-wide usage distinct from an empty site list', () => {
    expect(parseDatabaseList({ databases: [], total: 0, plan_max_databases: 5, used_databases: 3 }).quota).toEqual({ used: 3, limit: 5, remaining: 2 });
  });
  it.each([0, -1])('represents unlimited quota %s without a fake remaining count', limit => {
    expect(parseDatabaseList({ ...response, plan_max_databases: limit }).quota).toEqual({ used: 2, limit, remaining: null });
  });
  it('clamps the remaining count after a plan downgrade', () => {
    expect(parseDatabaseList({ ...response, plan_max_databases: 1 }).quota.remaining).toBe(0);
  });
  it.each([null, {}, { ...response, databases: null }, { ...response, used_databases: '2' }, { ...response, used_databases: -1 }, { ...response, plan_max_databases: null }, { databases: [], total: 0, quota: { used: 0, limit: 5 } }, { ...response, databases: [null] }, { ...response, databases: [{ ...database, name: null }] }, { ...response, databases: [{ ...database, engine: 'unknown' }] }])('rejects malformed success payload %# without guessing quota', value => {
    expect(() => parseDatabaseList(value)).toThrow('Format respons basis data tidak valid');
  });
  it('rejects malformed HTTP 200 before it can reach a component', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify({ databases: [], total: 0 }))));
    await expect(fetchDatabases({ site_id: 1 })).rejects.toThrow('Format respons basis data tidak valid');
  });
  it('preserves the backend failure message', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: 'Gagal membaca daftar basis data.' }), { status: 500 })));
    await expect(fetchDatabases({ site_id: 1 })).rejects.toThrow('Gagal membaca daftar basis data.');
  });
  it('sends the chosen password under the field Rust reads', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ success: true, new_password: 'fixture-password', message: 'Updated' })));
    vi.stubGlobal('fetch', fetchMock); setCsrfToken('fixture');
    await rotateDatabasePassword(7, { new_password: 'fixture-password' });
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ new_password: 'fixture-password' });
    expect(fetchMock.mock.calls[0][1].headers.get('X-CSRF-Token')).toBe('fixture');
  });
  it.each([{}, { from: 1792065441, to: 1792151841 }])('uses the registered application-metrics route with range %j', async range => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify({ site_id: 1, runtime: 'php', php_fpm: null, app_process: null })));
    vi.stubGlobal('fetch', fetchMock);
    await fetchSiteAppMetrics(1, range);
    const url = new URL(fetchMock.mock.calls[0][0], 'http://panel.local');
    expect(url.pathname).toBe('/api/v1/sites/1/analytics/app');
    expect(Object.fromEntries(url.searchParams)).toEqual(Object.fromEntries(Object.entries(range).map(([key, value]) => [key, String(value)])));
  });
});
