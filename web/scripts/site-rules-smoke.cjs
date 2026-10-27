/* Production frontend and API fixtures only; no panel-api/agent or Nginx service. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const fixture = require('./fixtures/site-rules.json');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5190';
const output = path.join(__dirname, '../artifacts/site-rules');
const domain = 'testing.incogmsg.my.id';
const site = { id: 1, user_id: 2, primary_domain: domain, kind: 'static', docroot: `/home/p_alice/sites/${domain}/public`, status: 'active', spa_fallback: false, force_https: false, created_at: 1770000000, updated_at: 1770000000, domains: [] };
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || 'msedge' });
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce' });
    page.setDefaultTimeout(10000);
    const errors = [], puts = [];
    let stored = structuredClone(fixture), mode = 'ok', release;
    const initialGate = new Promise(resolve => { release = resolve; });
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => { localStorage.setItem('nusa_theme', 'light'); });
    await page.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url());
      let status = 200, body = {};
      if (url.pathname === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/sites') body = [site];
      else if (url.pathname === '/api/v1/sites/1') {
        if (mode === 'metadata-failure') { status = 500; body = { error: 'fixture metadata unavailable' }; }
        else body = site;
      }
      else if (url.pathname === '/api/v1/sites/1/ssl') body = { site_id: 1, has_ssl: false, certificate: null };
      else if (url.pathname === '/api/v1/sites/1/rules') {
        await initialGate;
        if (req.method() === 'PUT') {
          const payload = req.postDataJSON(); puts.push(payload);
          if (mode === 'reject') { status = 400; body = { error: 'Validasi aturan situs gagal: fixture rejected' }; }
          else { stored = structuredClone(payload); body = stored; }
        } else if (mode === 'load-failure') { status = 500; body = { error: 'fixture unavailable' }; }
        else body = stored;
      }
      else if (url.pathname.includes('timeseries')) body = { points: [] };
      else if (url.pathname === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 1 };
      else if (url.pathname.includes('notifications')) body = { notifications: [], count: 0, unread_count: 0, total: 0 };
      else { status = 404; body = { error: 'No fixture' }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const save = async (success = true) => {
      await page.getByRole('button', { name: 'Simpan Perubahan Aturan', exact: true }).click();
      try {
        await page.getByText(success ? /Aturan situs berhasil diterapkan/ : /Gagal menyimpan aturan situs:/).waitFor();
      } catch (error) {
        console.error(JSON.stringify({ feedback: await page.getByText(/Gagal|berhasil|Failed|success/).allTextContents(), puts, errors }));
        await page.screenshot({ path: path.join(output, 'failure.png') });
        throw error;
      }
    };
    const addRedirect = async (source, target, match = 'exact', code = '301') => {
      await page.getByRole('button', { name: 'Tambah Redirect', exact: true }).click();
      const modal = page.locator('.dialog-panel');
      await modal.getByPlaceholder('/blog/old-post', { exact: true }).fill(source);
      await modal.getByPlaceholder('https://example.com/new-post atau /new-post', { exact: true }).fill(target);
      await modal.locator('select').nth(0).selectOption(match);
      await modal.locator('select').nth(1).selectOption(code);
      await modal.getByRole('button', { name: 'Tambahkan', exact: true }).click();
      await page.getByRole('cell', { name: source, exact: true }).waitFor();
    };
    await page.goto(`${base}/sites/${domain}/rules`, { waitUntil: 'domcontentloaded' });
    await page.getByText('Memuat aturan situs...', { exact: true }).waitFor();
    check(await page.getByRole('button', { name: 'Simpan Perubahan Aturan', exact: true }).count() === 0, 'save unavailable before existing rules load');
    release();
    await page.getByRole('cell', { name: '/old-existing', exact: true }).waitFor();
    check(!await page.getByRole('checkbox', { name: 'Aktifkan redirect /old-existing', exact: true }).isChecked(), 'existing disabled redirect is displayed with canonical source/target');
    await addRedirect('/old-blog', '/new-blog');
    await save();
    const first = structuredClone(stored), created = first.redirects[1];
    check(/^redirect_[0-9a-f]{32}$/.test(created.id), 'new redirect includes an ID without requiring crypto.randomUUID');
    assert.deepEqual(created, { id: created.id, source: '/old-blog', target: '/new-blog', status_code: 301, match_type: 'exact', enabled: true });
    check(true, 'new redirect sends the exact Rust input fields');
    assert.deepEqual({ ...first, redirects: fixture.redirects }, fixture);
    check(true, 'redirect save preserves every existing rule, nullable field, ID, disabled flag and zero timeout');
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.getByRole('cell', { name: '/new-blog', exact: true }).waitFor();
    await save();
    assert.deepEqual(stored, first);
    check(true, 'reload and repeated save keep IDs and all settings stable');
    await addRedirect('/legacy', '/replacement', 'prefix', '302');
    await save();
    check(stored.redirects[2].status_code === 302 && stored.redirects[2].match_type === 'prefix' && stored.redirects[2].id !== created.id, '302 prefix redirect receives a distinct stable ID');
    await page.getByRole('checkbox', { name: 'Aktifkan redirect /old-blog', exact: true }).uncheck();
    await save();
    check(stored.redirects[1].id === created.id && stored.redirects[1].enabled === false, 'changing enabled preserves redirect identity');
    await page.getByRole('row').filter({ has: page.getByRole('cell', { name: '/legacy', exact: true }) }).getByTitle('Hapus redirect').click();
    await save();
    check(stored.redirects.length === 2 && stored.redirects[1].id === created.id, 'deleting a redirect preserves the remaining identities');
    mode = 'reject';
    const accepted = structuredClone(stored);
    await addRedirect('/draft', '/draft-target');
    await save(false);
    assert.deepEqual(stored, accepted);
    check(await page.getByRole('cell', { name: '/draft', exact: true }).isVisible(), 'failed save retains the draft and does not report success');
    mode = 'ok';
    await save();
    check(stored.redirects.some(rule => rule.source === '/draft'), 'retry submits the retained draft successfully');
    await page.getByRole('button', { name: /^Header/ }).click();
    check(await page.getByRole('combobox').nth(0).inputValue() === 'DENY' && await page.getByRole('combobox').nth(1).inputValue() === '', 'security header names and nullable values render from the Rust response');
    check(await page.getByPlaceholder("default-src 'self'; script-src 'self' 'unsafe-inline'").inputValue() === fixture.security_headers.csp, 'saved CSP renders with the canonical field');
    check(!await page.getByRole('checkbox', { name: 'Aktifkan header X-Fixture', exact: true }).isChecked(), 'existing disabled custom header is preserved');
    await page.getByRole('button', { name: 'Tambah Header', exact: true }).click();
    await page.getByPlaceholder('X-Custom-Header', { exact: true }).fill('X-New');
    await page.getByPlaceholder('custom-value', { exact: true }).fill('new-value');
    await page.locator('.dialog-panel').getByRole('button', { name: 'Tambahkan', exact: true }).click();
    await page.getByRole('button', { name: 'Akses IP', exact: true }).click();
    check(!await page.getByRole('checkbox', { name: 'Aktifkan kontrol akses IP', exact: true }).isChecked(), 'disabled IP access state survives fetching and redirect saves');
    await page.getByRole('button', { name: 'Tambah Aturan IP', exact: true }).click();
    await page.getByPlaceholder('192.168.1.100 atau 10.0.0.0/24', { exact: true }).fill('198.51.100.0/24');
    await page.locator('.dialog-panel').getByRole('button', { name: 'Tambahkan', exact: true }).click();
    await save();
    check(/^header_[0-9a-f]{32}$/.test(stored.custom_headers[1].id) && stored.custom_headers[1].enabled, 'custom headers include required ID and enabled fields');
    check(/^ip_[0-9a-f]{32}$/.test(stored.ip_access.rules[1].id) && stored.ip_access.rules[1].cidr_or_ip === '198.51.100.0/24', 'IP rules include required ID and canonical CIDR field');
    await page.getByRole('checkbox', { name: 'Aktifkan kontrol akses IP', exact: true }).check();
    await page.getByPlaceholder('misal: /wp-admin atau /api/secret', { exact: true }).fill('');
    await page.getByRole('button', { name: 'CORS', exact: true }).click();
    await page.locator('div').filter({ has: page.getByText('Izinkan Semua (*)', { exact: true }) }).filter({ has: page.locator('input[name="cors_preset"]') }).last().click();
    await save();
    check(stored.ip_access.enabled && stored.ip_access.scope === 'global' && stored.ip_access.path === null, 'IP enable/scope updates match the Rust config');
    check(stored.cors.enabled && stored.cors.preset === 'allow_all', 'selecting an active CORS preset enables the Rust CORS config');
    await page.getByRole('button', { name: 'Redirect', exact: true }).click();
    await page.screenshot({ path: path.join(output, 'redirects-light.png') });
    await page.getByRole('button', { name: 'Ganti tema', exact: true }).click();
    await page.waitForFunction(() => document.documentElement.classList.contains('dark'));
    await page.screenshot({ path: path.join(output, 'redirects-dark.png') });
    await page.setViewportSize({ width: 390, height: 844 });
    const description = await page.getByText('Arahkan path lama atau URL pola regex ke lokasi target baru dengan kode status yang tepat.', { exact: true }).boundingBox();
    const addButton = await page.getByRole('button', { name: 'Tambah Redirect', exact: true }).boundingBox();
    check(addButton.y >= description.y + description.height, 'mobile add button sits below the redirect description without overlap');
    await addRedirect('/mobile', '/mobile-target');
    await save();
    check(stored.redirects.some(rule => rule.source === '/mobile'), 'redirect create/save works on mobile');
    await page.screenshot({ path: path.join(output, 'redirects-mobile.png') });
    await page.getByRole('button', { name: 'Halaman Error', exact: true }).click();
    check(await page.getByRole('cell', { name: '/errors/404.html', exact: true }).isVisible(), 'saved error-page target renders from the Rust response');
    mode = 'metadata-failure';
    await save();
    await page.getByText('Aturan tersimpan, tetapi detail situs gagal diperbarui. Segarkan halaman.', { exact: true }).waitFor();
    check(await page.getByRole('cell', { name: '/errors/404.html', exact: true }).isVisible(), 'failed background metadata refresh retains the active section and saved rules');
    mode = 'load-failure';
    await page.getByRole('button', { name: 'Segarkan', exact: true }).click();
    await page.getByText(/Gagal memuat aturan situs:/).waitFor();
    check(await page.getByRole('button', { name: 'Simpan Perubahan Aturan', exact: true }).isDisabled(), 'load failure prevents overwriting existing rules with defaults');
    mode = 'ok';
    await page.getByRole('button', { name: 'Segarkan', exact: true }).click();
    await page.getByRole('button', { name: 'Redirect', exact: true }).click();
    await page.getByRole('cell', { name: '/mobile-target', exact: true }).waitFor();
    check(await page.getByRole('button', { name: 'Simpan Perubahan Aturan', exact: true }).isEnabled(), 'reload retry restores saving with existing rules');
    check(errors.length === 0, 'no browser runtime errors');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors, puts }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
