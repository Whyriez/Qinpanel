/* UI fixtures only: no provider accounts, panel services, or real plan writes. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5201';
const output = path.join(__dirname, '../artifacts/ai-assistant');
let checks = 0;
const check = (value, label) => { assert.ok(value, label); console.log(`ok ${++checks}: ${label}`); };
const mib = 1048576;
const config = { enabled: false, provider: 'gemini', model: 'retired-gemini-model', revision: 1, daily_request_limit: 20, requests_today: 0, max_output_tokens: 2048, reset_timezone: 'UTC', providers: [
  { id: 'gemini', models: ['gemini-3.8-flash', 'gemini-3.5-flash-lite'], has_key: false },
  { id: 'openai', models: ['gpt-5.4-mini', 'gpt-5-mini'], has_key: false },
] };
function draft(demand) {
  const now = Math.floor(Date.now() / 1000);
  return { id: 'fixture-draft', provider: config.provider, model: config.model, settings_revision: config.revision, snapshot_fingerprint: 'a'.repeat(64), profile: 'balanced', summary: 'Profil seimbang untuk aplikasi pribadi, dengan cadangan untuk sistem dan layanan bersama.', considerations: ['Snapshot sesaat belum menggambarkan puncak beban.'], total_tokens: 120, draft_only: true,
    preset: { rules_version: 1, observed_at: now, expires_at: now + 60, demand, cpu_quota_percent: 180, memory_limit_bytes: 1536 * mib, memory_high_bytes: 1152 * mib, disk_limit_bytes: 4096 * mib, inode_limit: 123000, tasks_max: 128, io_weight: 100, max_sites: demand.projects_per_tenant, max_upload_bytes: 50 * mib, memory_headroom_bytes: 256 * mib, cpu_headroom_percent: 20, disk_headroom_bytes: 512 * mib, notices: [] } };
}
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge', headless: true });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errors = [], missing = [], writes = [], external = [];
  let providerFailure = false;
  page.on('pageerror', e => errors.push(e.message));
  page.on('request', request => { if (!request.url().startsWith(base) && !request.url().startsWith('data:')) external.push(request.url()); });
  await page.addInitScript(() => { if (!localStorage.getItem('nusa_theme')) localStorage.setItem('nusa_theme', 'light'); });
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), url = new URL(request.url()), p = url.pathname, method = request.method();
    let body = {}, status = 200;
    if (method !== 'GET') {
      const data = request.postDataJSON(); writes.push({ path: p, data, csrf: request.headers()['x-csrf-token'] });
      if (p === '/api/v1/system/ai' && method === 'PUT') {
        assert.equal(data.expected_revision, config.revision);
        Object.assign(config, { enabled: data.enabled, provider: data.provider, model: data.model, daily_request_limit: data.daily_request_limit, revision: config.revision + 1 });
        if (data.api_key) config.providers.find(v => v.id === data.provider).has_key = true;
        body = { success: true, revision: config.revision };
      } else if (p === '/api/v1/system/ai/test') { assert.equal(data.consent, true); config.requests_today++; body = { success: true, total_tokens: 20 }; }
      else if (p === '/api/v1/plans/ai/recommend') { config.requests_today++; body = draft(data.demand); if (providerFailure) { status = 503; body = { error: 'Kapasitas terverifikasi belum tersedia.' }; } }
      else { status = 405; missing.push(`${method} ${p}`); }
    } else if (p === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin', status: 'active' }, csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/system/ai') body = config;
    else if (p === '/api/v1/plans') body = { plans: [] };
    else if (p === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 100, total_pages: 0 };
    else if (p.includes('notifications')) body = { notifications: [], unread_count: 0, total: 0 };
    else if (p === '/api/v1/system/metrics') body = { state: 'waiting', error: null, sampling_interval_secs: 2, sample: null };
    else if (p === '/api/v1/system/info') body = { api_version: '0.1.0', agent_version: '0.1.0', agent_connected: true };
    else { status = 404; missing.push(p); body = { error: 'Missing fixture' }; }
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  const modal = () => page.getByTestId('ai-package-assistant');
  async function openAssistant() {
    await page.goto(`${base}/plans`);
    await page.getByRole('button', { name: 'Bantu atur dengan AI', exact: true }).click();
    await modal().getByLabel('Kebutuhan aplikasi', { exact: true }).waitFor();
  }
  async function analyze() {
    await modal().getByLabel('Kebutuhan aplikasi', { exact: true }).fill('Dua aplikasi Laravel pribadi dan satu database.');
    await modal().getByRole('checkbox').check();
    await modal().getByRole('button', { name: 'Analisis kebutuhan', exact: true }).click();
    await modal().getByRole('button', { name: 'Isi formulir paket baru' }).waitFor();
  }
  async function layout(label) {
    check(await page.locator('html').evaluate(el => el.scrollWidth <= el.clientWidth + 1), `${label}: no page overflow`);
    const root = await modal().count() ? modal() : page.getByTestId('ai-settings');
    check(await root.evaluate(el => el.scrollWidth <= el.clientWidth + 1), `${label}: no content overflow`);
    const outside = await root.locator('input:visible,select:visible,textarea:visible,button:visible').evaluateAll(elements => elements.filter(el => { const r = el.getBoundingClientRect(); return r.left < -1 || r.right > innerWidth + 1; }).length);
    check(outside === 0, `${label}: controls fit viewport`);
    const small = await root.locator('button:visible,input:not([type="checkbox"]):visible,select:visible,textarea:visible').evaluateAll(elements => elements.filter(el => el.getBoundingClientRect().height < 44).map(el => el.textContent || el.id));
    check(small.length === 0, `${label}: controls meet 44px touch size (${small.join(', ')})`);
  }
  try {
    await page.goto(`${base}/settings?section=ai`);
    const settings = page.getByTestId('ai-settings');
    await settings.getByLabel('API key', { exact: true }).waitFor();
    check(await settings.getByRole('button', { name: 'Uji koneksi' }).isDisabled(), 'test requires configured key and consent');
    check(await settings.getByLabel('Model', { exact: true }).inputValue() === 'retired-gemini-model', 'a retired stored model is not silently presented as an approved model');
    await settings.getByLabel('Model', { exact: true }).selectOption('gemini-3.8-flash');
    await settings.getByLabel('API key', { exact: true }).fill('  fixture_gemini_key_not_real_123  ');
    await settings.getByRole('button', { name: 'Simpan pengaturan' }).click();
    await settings.getByText('Pengaturan AI tersimpan.', { exact: true }).waitFor();
    check(writes.find(w => w.path === '/api/v1/system/ai')?.data.api_key === 'fixture_gemini_key_not_real_123', 'Gemini key paste whitespace is removed before the authenticated save');
    await settings.getByLabel('Provider', { exact: true }).selectOption('openai');
    check(await settings.getByLabel('Model', { exact: true }).inputValue() === 'gpt-5.4-mini', 'provider switch selects its own supported model');
    await settings.getByLabel('API key', { exact: true }).fill('fixture_key_not_a_real_secret_123');
    await settings.getByRole('checkbox', { name: 'Aktifkan asisten AI untuk administrator' }).check();
    await settings.getByRole('button', { name: 'Simpan pengaturan' }).click();
    await settings.getByText('Pengaturan AI tersimpan.', { exact: true }).waitFor();
    check(await settings.getByLabel('API key', { exact: true }).inputValue() === '', 'key input is cleared after saving');
    check(!await settings.innerText().then(text => text.includes('fixture_key_not_a_real_secret_123')), 'saved secret is never rendered');
    await settings.getByRole('checkbox', { name: /Saya menyetujui satu permintaan uji/ }).check();
    await settings.getByRole('button', { name: 'Uji koneksi' }).click();
    await settings.getByText('Koneksi dan output terstruktur berhasil diverifikasi.', { exact: true }).waitFor();
    check(writes.filter(w => w.path.endsWith('/test')).length === 1, 'connection test is an explicit single request');
    await page.screenshot({ path: path.join(output, 'settings-desktop.png'), fullPage: true });
    await openAssistant();
    check(await modal().getByRole('button', { name: 'Analisis kebutuhan' }).isDisabled(), 'analysis requires workload and consent');
    await modal().getByLabel('Penggunaan server').selectOption('multi_project');
    check(await modal().getByLabel('Project per tenant').inputValue() === '2', 'multi-project mode has canonical counts');
    await analyze();
    const last = writes.filter(w => w.path.endsWith('/recommend')).at(-1);
    check(last.data.demand.projects_per_tenant === 2 && last.data.demand.tenant_slots === 1, 'demand remains explicit in the request');
    check(!('capacity' in last.data) && !('target_id' in last.data), 'browser cannot supply measured capacity or target IDs');
    check(await modal().getByText('1536 MiB', { exact: true }).isVisible(), 'preview displays computed resources');
    await layout('desktop assistant');
    await page.screenshot({ path: path.join(output, 'preview-desktop.png'), fullPage: true });
    await modal().getByRole('button', { name: 'Isi formulir paket baru' }).click();
    await page.getByRole('dialog').getByText(/Draft dari OpenAI/).waitFor();
    check(await page.getByRole('dialog').getByPlaceholder('e.g. starter, business, pro').inputValue() === 'Paket aplikasi', 'AI fills a new editable form');
    check(await page.getByRole('dialog').locator('input[type="number"]').evaluateAll(elements => elements.some(e => e.value === '1536')), 'form resource fields use the verified draft');
    check(!writes.some(w => w.path === '/api/v1/plans'), 'preview/fill does not create or update a plan');
    await page.getByRole('dialog').getByRole('button', { name: 'Batal', exact: true }).click();
    await openAssistant(); await analyze(); config.revision++;
    await modal().getByRole('button', { name: 'Isi formulir paket baru' }).click();
    await modal().getByRole('alert').filter({ hasText: 'Pengaturan atau preview berubah' }).waitFor();
    check(await modal().getByRole('button', { name: 'Isi formulir paket baru' }).count() === 0, 'configuration changes invalidate the draft');
    await openAssistant(); await analyze();
    await page.evaluate(() => { const original = Date.now; Date.now = () => original() + 61000; });
    await modal().getByRole('button', { name: 'Isi formulir paket baru' }).waitFor();
    await page.waitForFunction(() => [...document.querySelectorAll('button')].find(b => b.textContent.includes('Isi formulir paket baru'))?.disabled);
    check(await modal().getByText('Preview kedaluwarsa. Jalankan analisis lagi.').isVisible(), 'expired previews cannot fill the form');
    await openAssistant(); providerFailure = true;
    await modal().getByLabel('Kebutuhan aplikasi', { exact: true }).fill('Aplikasi pribadi'); await modal().getByRole('checkbox').check();
    await modal().getByRole('button', { name: 'Analisis kebutuhan' }).click();
    await modal().getByRole('alert').filter({ hasText: 'Kapasitas terverifikasi belum tersedia.' }).waitFor();
    check(await modal().getByRole('button', { name: 'Isi formulir paket baru' }).count() === 0, 'capacity/provider errors never fabricate a preview'); providerFailure = false;
    for (const width of [320, 375, 768]) {
      await page.setViewportSize({ width, height: 850 }); await openAssistant(); await analyze(); await layout(`assistant ${width}px`);
      await page.screenshot({ path: path.join(output, `preview-${width}.png`), fullPage: true });
      await page.goto(`${base}/settings?section=ai`); await settings.getByLabel('API key', { exact: true }).waitFor(); await layout(`settings ${width}px`);
    }
    await page.setViewportSize({ width: 1280, height: 900 }); await openAssistant();
    await page.evaluate(() => { document.body.style.zoom = '2'; }); await layout('assistant 200% zoom');
    await page.screenshot({ path: path.join(output, 'assistant-zoom.png'), fullPage: true });
    await page.keyboard.press('Escape'); check(await modal().count() === 0, 'Escape closes the assistant');
    await page.evaluate(() => { localStorage.setItem('nusa_theme', 'dark'); document.body.style.zoom = ''; });
    await page.setViewportSize({ width: 375, height: 850 }); await openAssistant(); await analyze();
    check(await page.locator('html').evaluate(el => el.classList.contains('dark')), 'dark theme is active');
    await layout('dark assistant'); await page.screenshot({ path: path.join(output, 'preview-dark-375.png'), fullPage: true });
    await page.goto(`${base}/settings?section=ai`); await settings.getByLabel('API key', { exact: true }).waitFor();
    await layout('dark settings'); await page.screenshot({ path: path.join(output, 'settings-dark-375.png'), fullPage: true });
    check(writes.every(w => w.csrf === 'fixture-csrf'), 'all mutation fixtures include CSRF');
    check(!writes.some(w => w.path === '/api/v1/plans'), 'no real or simulated plan-save requests were made');
    check(missing.length === 0, `all API fixtures resolved (${missing.join(', ')})`);
    check(external.length === 0, 'no provider/external network requests');
    check(errors.length === 0, `no browser exceptions (${errors.join(', ')})`);
    console.log(`${checks} AI UI checks passed. Screenshots: ${output}`);
  } catch (error) { await page.screenshot({ path: path.join(output, 'failure.png'), fullPage: true }); throw error; }
  finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
