/* Local production UI with intercepted APIs; no CA, Cloudflare or server changes. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5206';
const output = path.join(__dirname, '../artifacts/storage-connection');
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge', headless: true });
  const page = await browser.newPage();
  const errors = [], missing = [], external = [], mutations = [];
  const tunnelId = '7c03851f-60dd-49d1-a3aa-a2100320e7c9';
  let configured = false, polls = 0, checks = 0;
  const initial = () => ({ revision: 1, request_id: null, hostname: null, email: null, state: 'idle', auto_renew: false, not_after: null, renew_at: null, job_id: null, error: null });
  let tls = initial();
  const check = (value, label) => { assert.ok(value, label); console.log(`ok ${++checks}: ${label}`); };
  const status = () => ({ configured, running: configured, origin_ready: configured, public_access_verified: false,
    connection_mode: configured ? tls.method === 'tunnel' ? 'tunnel' : 'https' : null,
    endpoint: configured ? 'https://storage.example.test' : null, engine_version: '4.50',
    data_directory: '/var/lib/qonpanel-storage', capacity_bytes: null, available_bytes: null, bucket_count: 0, quota_enforcement: 'none' });
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => { if (!request.url().startsWith(base)) external.push(request.url()); });
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), p = new URL(request.url()).pathname, method = request.method();
    let body = {}, code = 200;
    if (method === 'POST') {
      const data = request.postDataJSON();
      assert.equal(request.headers()['x-csrf-token'], 'fixture-csrf');
      if (p === '/api/v1/network/cloudflare') {
        assert.equal(data.action, 'zones');
        body = { items: [{ id: 'a'.repeat(32), name: 'example.test', status: 'active' }], pagination: { page: 1, total_pages: 1 } };
      } else if (p === '/api/v1/storage/managed/tls') {
        mutations.push(data);
        const tunnel = data.method === 'tunnel';
        assert.equal(data.confirmed, !tunnel);
        assert.equal(data.expected_revision, tls.revision);
        assert.equal(data.hostname, 'storage.example.test');
        assert.equal(data.email, tunnel ? '' : 'admin@example.test');
        if (tunnel) { assert.equal(data.zone_id, 'a'.repeat(32)); assert.equal(data.tunnel_id, tunnelId); }
        else { assert.equal(data.method, 'http01'); assert.ok(!('zone_id' in data) && !('tunnel_id' in data)); }
        assert.ok(!('private_key_pem' in data) && !('api_token' in data));
        tls = { ...tls, ...data, state: 'queued', revision: tls.revision + 1, auto_renew: !tunnel, job_id: 'fixture-job' };
        polls = 0; code = 202; body = tls;
      } else { code = 405; missing.push(`${method} ${p}`); }
    } else if (p === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin', status: 'active' }, csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/storage/context') body = { owners: [], sites: [], upload_limit_bytes: 33554432 };
    else if (p === '/api/v1/storage/connections') body = { connections: [], upload_limit_bytes: 33554432 };
    else if (p === '/api/v1/storage/managed') body = status();
    else if (p === '/api/v1/storage/managed/tls') {
      if (tls.state === 'queued' && ++polls >= 3) { configured = true; tls = { ...tls, state: 'active' }; }
      body = { certificate: tls, dns: { available: false, configured: false, zone_name: null } };
    } else if (p === '/api/v1/network/cloudflare') body = { configured: true, connector_installed: true, account_id: 'a'.repeat(32), managed_tunnels: [{ id: tunnelId, local_status: 'active', routes: [] }] };
    else if (p.includes('notifications')) body = { notifications: [], total: 0, unread_count: 0 };
    else if (p === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 100, total_pages: 0 };
    else { code = 404; missing.push(`${method} ${p}`); }
    await route.fulfill({ status: code, contentType: 'application/json', body: JSON.stringify(body) });
  });
  try {
    await page.goto(`${base}/storage`);
    for (const mode of ['tunnel', 'http01']) {
      await page.getByRole('button', { name: 'Aktifkan storage Qinpanel', exact: true }).click();
      const dialog = page.getByRole('dialog');
      await dialog.getByLabel('Jalur akses storage', { exact: true }).selectOption(mode);
      await dialog.getByLabel('Domain storage', { exact: true }).fill('storage.example.test');
      check(await dialog.locator('textarea').count() === 0, `${mode}: no PEM fields`);
      check(await dialog.getByText('Hubungkan token DNS-01 (tab baru)', { exact: true }).count() === 0, `${mode}: no DNS-01 setup required`);
      if (mode === 'tunnel') {
        check(await dialog.locator('input[type=email],input[type=checkbox]').count() === 0, 'Tunnel needs no CA email or consent');
        await dialog.getByRole('button', { name: 'Aktifkan melalui Tunnel', exact: true }).waitFor();
      } else {
        await dialog.getByLabel('Email sertifikat', { exact: true }).fill('admin@example.test');
        check(await dialog.locator('button[type=submit]').isDisabled(), 'HTTP-01 requires explicit CA consent');
        await dialog.getByRole('checkbox').check();
      }
      for (const theme of ['light', 'dark']) {
        await page.evaluate(theme => document.documentElement.classList.toggle('dark', theme === 'dark'), theme);
        for (const [width, height] of [[1440, 1000], [375, 667], [320, 568]]) {
          await page.setViewportSize({ width, height });
          await page.waitForFunction(() => { const overlay = document.querySelector('.storage-modal'); return overlay && getComputedStyle(overlay).opacity === '1'; });
          const bounds = await dialog.locator('.dialog-panel').boundingBox();
          check(bounds.x >= -1 && bounds.y >= -1 && bounds.x + bounds.width <= width + 1 && bounds.y + bounds.height <= height + 1, `${mode} ${theme} ${width}: dialog fits`);
          check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `${mode} ${theme} ${width}: no horizontal overflow`);
          const button = await dialog.locator('button[type=submit]').boundingBox();
          check(button && button.y >= 0 && button.y + button.height <= height + 1, `${mode} ${theme} ${width}: submit reachable`);
          if (width === 375) await page.screenshot({ path: path.join(output, `${mode}-${theme}-375.png`) });
        }
      }
      await dialog.locator('button[type=submit]').click();
      await dialog.getByText(/Aktivasi berjalan di latar belakang/).waitFor();
      await dialog.locator('.dialog-footer').getByRole('button', { name: 'Tutup', exact: true }).click();
      await page.getByRole('button', { name: 'Aktifkan storage Qinpanel', exact: true }).click();
      await page.getByRole('dialog').waitFor({ state: 'hidden' });
      check(tls.state === 'active' && tls.method === mode, `${mode}: activation survives closing and reopening`);
      check(mutations.filter(item => item.method === mode).length === 1, `${mode}: reopening does not replay activation`);
      check(tls.auto_renew === (mode === 'http01'), `${mode}: renewal follows the selected method`);
      configured = false; tls = initial(); await page.reload();
    }
    check(errors.length === 0, `no browser errors: ${errors.join(', ')}`);
    check(missing.length === 0, `all requests intercepted: ${missing.join(', ')}`);
    check(external.length === 0, 'no external provider or CA requests');
    console.log(`${checks} storage connection UI checks passed.`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
