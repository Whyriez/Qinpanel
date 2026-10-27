import { afterEach, describe, expect, it, vi } from 'vitest';
import { aiDraftFields, fetchAiSettings, recommendAiPackage, saveAiSettings, testAiConnection, validAiDraft, type AiRecommendation } from './ai';
import { setCsrfToken } from './client';

const mib = 1048576;
function draft(): AiRecommendation {
  const observed = Math.floor(Date.now() / 1000);
  return { id: 'fixture-draft', provider: 'gemini', model: 'gemini-3.8-flash', settings_revision: 2, snapshot_fingerprint: 'a'.repeat(64), profile: 'balanced', summary: 'Profil seimbang.', considerations: [], draft_only: true, total_tokens: 120,
    preset: { rules_version: 1, observed_at: observed, expires_at: observed + 60, demand: { usage: 'personal', projects_per_tenant: 1, tenant_slots: 1 }, cpu_quota_percent: 90, memory_limit_bytes: 512 * mib, memory_high_bytes: 384 * mib, disk_limit_bytes: 2048 * mib, inode_limit: 100000, tasks_max: 96, io_weight: 100, max_sites: 1, max_upload_bytes: 50 * mib, memory_headroom_bytes: 256 * mib, cpu_headroom_percent: 10, disk_headroom_bytes: 512 * mib, notices: [] } };
}
afterEach(() => { vi.unstubAllGlobals(); setCsrfToken(null); });

describe('AI integration client and draft boundary', () => {
  it('sends keys only in the CSRF-protected save body and never in the URL', async () => {
    const fetch = vi.fn().mockResolvedValue(Response.json({ success: true, revision: 2 })); vi.stubGlobal('fetch', fetch); setCsrfToken('fixture-csrf');
    await saveAiSettings({ expected_revision: 1, enabled: false, provider: 'openai', model: 'gpt-5-mini', daily_request_limit: 20, api_key: 'fixture_key_not_real_123' });
    const [url, request] = fetch.mock.calls[0];
    expect(url).toBe('/api/v1/system/ai'); expect(request.method).toBe('PUT'); expect(request.headers.get('X-CSRF-Token')).toBe('fixture-csrf');
    expect(JSON.parse(request.body).api_key).toBe('fixture_key_not_real_123'); expect(url).not.toContain('fixture_key');
  });
  it('keeps credentials unchanged when a key is omitted', async () => {
    const fetch = vi.fn().mockResolvedValue(Response.json({ success: true })); vi.stubGlobal('fetch', fetch);
    await saveAiSettings({ expected_revision: 2, enabled: true, provider: 'gemini', model: 'gemini-3.8-flash', daily_request_limit: 15 });
    expect(JSON.parse(fetch.mock.calls[0][1].body)).not.toHaveProperty('api_key');
  });
  it('requires a revision and explicit consent for connection tests', async () => {
    const fetch = vi.fn().mockResolvedValue(Response.json({ success: true })); vi.stubGlobal('fetch', fetch);
    await testAiConnection(7);
    expect(JSON.parse(fetch.mock.calls[0][1].body)).toEqual({ expected_revision: 7, consent: true });
  });
  it('rejects malformed settings from a mismatched API', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({ enabled: true, revision: 1, provider: 'unknown' })));
    await expect(fetchAiSettings()).rejects.toThrow('paket Qinpanel lengkap');
  });
  it('sends only explicit needs/counts/consent and forwards cancellation', async () => {
    const result = draft(); const fetch = vi.fn().mockResolvedValue(Response.json(result)); vi.stubGlobal('fetch', fetch);
    const controller = new AbortController();
    expect(await recommendAiPackage('Aplikasi pribadi', result.preset.demand, 2, false, controller.signal)).toEqual(result);
    const [url, request] = fetch.mock.calls[0]; expect(url).toBe('/api/v1/plans/ai/recommend'); expect(request.signal).toBe(controller.signal);
    expect(JSON.parse(request.body)).toEqual({ description: 'Aplikasi pribadi', demand: result.preset.demand, expected_revision: 2, consent: true, english: false });
  });
  it('does not turn malformed, expired or inconsistent output into a form', () => {
    const base = draft();
    expect(validAiDraft(base)).toBe(true);
    const cases = [
      { ...base, draft_only: false }, { ...base, profile: 'unlimited' },
      { ...base, preset: { ...base.preset, expires_at: Math.floor(Date.now() / 1000) - 1 } },
      { ...base, preset: { ...base.preset, memory_high_bytes: base.preset.memory_limit_bytes } },
      { ...base, preset: { ...base.preset, tasks_max: Number.NaN } },
      { ...base, preset: { ...base.preset, demand: undefined } },
    ];
    for (const value of cases) expect(validAiDraft(value as AiRecommendation)).toBe(false);
  });
  it('maps only resource fields and never grants shell or external upstream permissions', () => {
    const result = draft(); Object.assign(result.preset, { shell_policy: 'full', allow_external_upstream: true, command: 'untrusted' });
    const fields = aiDraftFields(result);
    expect(fields.memory_limit_mb).toBe(512); expect(fields.memory_high_mb).toBe(384);
    expect(fields.max_sites).toBe(1); expect(fields.max_databases).toBe(1);
    expect(fields).not.toHaveProperty('shell_policy'); expect(fields).not.toHaveProperty('allow_external_upstream'); expect(fields).not.toHaveProperty('command');
  });
  it('preserves provider/unavailable errors instead of fabricating a successful draft', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({ error: 'Provider belum tersedia' }, { status: 502 })));
    await expect(recommendAiPackage('app', draft().preset.demand, 2, false)).rejects.toMatchObject({ status: 502, message: 'Provider belum tersedia' });
  });
});
