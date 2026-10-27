/* Production bundle, local intercepted API only. Every key/certificate is a fixture. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5206';
const output = path.join(__dirname, '../artifacts/managed-storage');
let checks = 0;
const check = (value, name) => { assert.ok(value, name); console.log(`ok ${++checks}: ${name}`); };
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge', headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = [], external = [], missing = [], writes = [];
  let configured = false, connection = null, role = 'admin', failAllocation = true, failCertificate = true;
  let tls = { revision: 1, request_id: null, hostname: null, email: null, state: 'idle', auto_renew: false, not_after: null, renew_at: null, job_id: null, error: null };
  let tlsPolls = 0;
  const context = { owners: [{ id: 2, username: 'alice' }, { id: 3, username: 'bob' }], sites: [{ id: 1, owner_id: 2, domain: 'alice.example.test' }, { id: 2, owner_id: 3, domain: 'bob.example.test' }], upload_limit_bytes: 33554432 };
  const status = () => ({ configured, running: configured, endpoint: configured ? 'https://storage.example.test' : null, engine_version: '4.50', data_directory: '/var/lib/qonpanel-storage', capacity_bytes: configured ? 107374182400 : null, available_bytes: configured ? 85899345920 : null, bucket_count: connection ? 1 : 0, quota_enforcement: 'none' });
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => { if (!request.url().startsWith(base)) external.push(request.url()); });
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), url = new URL(request.url()), p = url.pathname, method = request.method();
    let code = 200, body = {};
    if (method !== 'GET') {
      const data = request.postDataJSON(); writes.push({ p, method, data, csrf: request.headers()['x-csrf-token'] });
      assert.equal(request.headers()['x-csrf-token'], 'fixture-csrf');
      if (p === '/api/v1/storage/managed/tls') {
        assert.equal(data.confirmed, true); assert.equal(data.expected_revision, tls.revision);
        assert.equal(data.hostname, 'storage.example.test'); assert.equal(data.email, 'admin@example.test');
        assert.ok(!('private_key_pem' in data)); assert.ok(!('api_token' in data));
        tls = { ...tls, ...data, revision: tls.revision + 1, state: 'queued', auto_renew: true, job_id: 'fixture-tls-job' }; tlsPolls = 0;
        code = 202; body = tls;
      } else if (p === '/api/v1/storage/managed') {
        assert.equal(data.hostname, 'storage.example.test'); assert.equal(data.private_key_pem, 'fixture-private-key');
        if (failCertificate) { failCertificate = false; code = 503; body = { error: 'Sertifikat fixture belum valid.' }; }
        else { configured = true; body = status(); }
      } else if (p === '/api/v1/storage/managed/buckets') {
        assert.equal(data.owner_id, 2); assert.equal(data.site_id, 1);
        connection = { id: data.request_id, owner_id: data.owner_id, site_id: data.site_id, label: data.label, provider: 'compatible', endpoint: status().endpoint, region: 'us-east-1', addressing: 'path', bucket: data.bucket, revision: 1, managed: true, managed_state: failAllocation ? 'unknown' : 'ready', application_key_generation: 0, application_key_state: null, has_credentials: true, verified_at: null, created_at: 1, updated_at: 1 };
        if (failAllocation) { failAllocation = false; code = 504; body = { error: 'Alokasi terputus. Lanjutkan permintaan yang sama.' }; }
        else body = connection;
      } else if (p.endsWith('/application-key')) {
        assert.equal(data.expected_revision, 1); assert.equal(data.expected_generation, connection.application_key_generation); assert.equal(data.confirmation, connection.bucket);
        connection.application_key_generation++; connection.application_key_state = 'ready';
        body = { credentials: { access_key: 'fixture-application-access', secret_key: 'fixture-application-secret-not-real', session_token: null }, generation: connection.application_key_generation };
      } else if (p.endsWith('/decommission')) { assert.equal(data.confirmation, connection.bucket); connection = null; body = { removed: true }; }
      else { code = 405; missing.push(`${method} ${p}`); }
    } else if (p === '/api/v1/auth/me') body = { user: { id: role === 'admin' ? 1 : 2, username: role === 'admin' ? 'admin' : 'alice', role, status: 'active' }, csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/storage/managed/tls') {
      if (tls.state === 'queued' && ++tlsPolls >= 3) tls = { ...tls, state: 'active', not_after: Math.floor(Date.now() / 1000) + 60 * 86400, renew_at: Math.floor(Date.now() / 1000) + 30 * 86400 };
      body = { certificate: tls, dns: { configured: true, zone_name: 'example.test' } };
    }
    else if (p === '/api/v1/storage/context') body = context;
    else if (p === '/api/v1/storage/managed') body = status();
    else if (p === '/api/v1/storage/connections') body = { connections: connection ? [connection] : [], upload_limit_bytes: 33554432 };
    else if (p.endsWith('/objects')) body = { listing: { objects: [], prefixes: [], next_token: null }, revision: 1, verified_at: Math.floor(Date.now() / 1000) };
    else if (p.includes('notifications')) body = { notifications: [], total: 0, unread_count: 0 };
    else if (p === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 100, total_pages: 0 };
    else { code = 404; missing.push(`${method} ${p}`); }
    await route.fulfill({ status: code, contentType: 'application/json', body: JSON.stringify(body) });
  });
  try {
    await page.goto(`${base}/storage`);
    await page.getByRole('button', { name: 'Aktifkan storage Qinpanel', exact: true }).waitFor({ state: 'visible' });
    check(await page.getByRole('button', { name: 'Hubungkan S3', exact: true }).count() === 1, 'external connection remains available beside managed storage');
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => { document.documentElement.classList.toggle('dark', theme === 'dark'); localStorage.setItem('nusa_theme', theme); }, theme);
      for (const [width, height] of [[1440, 1000], [375, 667], [320, 568], [844, 390]]) {
        await page.setViewportSize({ width, height });
        await page.getByRole('button', { name: 'Aktifkan storage Qinpanel', exact: true }).click();
        const dialog = page.getByRole('dialog'); await dialog.waitFor();
        await page.waitForFunction(() => { const overlay = document.querySelector('.storage-modal'), panel = overlay?.querySelector('.dialog-panel'); return overlay && panel && getComputedStyle(overlay).opacity === '1' && getComputedStyle(panel).opacity === '1'; });
        const bounds = await dialog.locator('.dialog-panel').boundingBox();
        check(bounds.x >= -1 && bounds.y >= -1 && bounds.x + bounds.width <= width + 1 && bounds.y + bounds.height <= height + 1, `${theme} ${width}x${height}: setup dialog fits viewport`);
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `${theme} ${width}x${height}: no page overflow`);
        const button = await dialog.getByRole('button', { name: 'Generate & aktifkan', exact: true }).boundingBox();
        check(button && button.y >= 0 && button.y + button.height <= height + 1, `${theme} ${width}x${height}: action stays reachable`);
        check(await dialog.locator('input').first().inputValue() === '', `${theme} ${width}x${height}: a fresh setup does not retain secrets`);
        if (width === 375 || width === 1440) await page.screenshot({ path: path.join(output, `setup-${theme}-${width}.png`), fullPage: true });
        await dialog.getByRole('button', { name: 'Batal', exact: true }).click();
      }
    }
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.getByRole('button', { name: 'Aktifkan storage Qinpanel', exact: true }).click();
    let dialog = page.getByRole('dialog');
    await dialog.getByLabel('Sertifikat HTTPS', { exact: true }).selectOption('manual');
    await dialog.getByLabel('Domain storage', { exact: true }).fill('storage.example.test');
    await dialog.getByLabel('Sertifikat lengkap (PEM)', { exact: true }).fill('fixture-certificate');
    await dialog.getByLabel('Private key (PEM)', { exact: true }).fill('fixture-private-key');
    await dialog.getByRole('button', { name: 'Aktifkan layanan', exact: true }).click();
    await dialog.getByRole('alert').waitFor();
    check(await dialog.getByRole('alert').evaluate(element => element === document.activeElement), 'certificate failure receives focus without closing the dialog');
    await dialog.getByRole('button', { name: 'Aktifkan layanan', exact: true }).click();
    await dialog.waitFor({ state: 'hidden' });
    check(await page.getByText('https://storage.example.test', { exact: true }).count() === 1, 'activated endpoint and capacity appear from service status');
    await page.getByRole('button', { name: 'Buat bucket Qinpanel', exact: true }).click();
    dialog = page.getByRole('dialog');
    await dialog.getByLabel('Nama tampilan', { exact: true }).fill('Project media');
    await dialog.getByLabel('Nama bucket', { exact: true }).fill('alice-media');
    const projects = await dialog.getByLabel('Project', { exact: true }).locator('option').allTextContents();
    check(projects.length === 2 && projects.includes('alice.example.test') && !projects.includes('bob.example.test'), `project choices are restricted to the selected tenant: ${JSON.stringify(projects)}`);
    await dialog.getByLabel('Project', { exact: true }).selectOption('1');
    await dialog.getByRole('button', { name: 'Buat bucket', exact: true }).click();
    await dialog.getByRole('alert').waitFor();
    check(await dialog.getByLabel('Nama bucket', { exact: true }).isDisabled(), 'an unknown outcome keeps its original allocation identity');
    await dialog.getByRole('button', { name: 'Lanjutkan alokasi', exact: true }).click();
    await dialog.waitFor({ state: 'hidden' });
    const allocations = writes.filter(value => value.p.endsWith('/managed/buckets'));
    check(allocations.length === 2 && JSON.stringify(allocations[0].data) === JSON.stringify(allocations[1].data), 'retry reuses the same allocation UUID, owner, bucket and project');
    await page.getByRole('button', { name: 'Access key aplikasi', exact: true }).waitFor();
    check(await page.getByRole('button', { name: 'Edit koneksi', exact: true }).count() === 0 && await page.getByRole('button', { name: 'Lepas koneksi', exact: true }).count() === 0, 'managed buckets do not expose external edit/disconnect actions');
    await page.getByRole('button', { name: 'Access key aplikasi', exact: true }).click();
    dialog = page.getByRole('dialog');
    const issue = dialog.getByRole('button', { name: 'Terbitkan key baru', exact: true });
    check(await issue.isDisabled(), 'key rotation requires typed bucket confirmation');
    await dialog.getByLabel('Ketik nama bucket untuk melanjutkan', { exact: true }).fill('alice-media');
    await issue.click();
    await dialog.getByText(/Simpan sekarang/).waitFor();
    check(await dialog.getByLabel('Secret access key', { exact: true }).getAttribute('type') === 'password', 'application secret is masked in its issuance result');
    check(await dialog.getByLabel('Access key ID', { exact: true }).inputValue() === 'fixture-application-access', 'issued bucket key is available for the application');
    await page.evaluate(() => Object.defineProperty(navigator, 'clipboard', { configurable: true, value: undefined }));
    await dialog.getByRole('button', { name: 'Salin kedua key', exact: true }).click();
    await dialog.getByRole('alert').waitFor();
    await dialog.getByRole('button', { name: 'Tampilkan secret', exact: true }).click();
    check(await dialog.getByLabel('Secret access key', { exact: true }).getAttribute('type') === 'text', 'HTTP clipboard fallback lets the admin reveal and copy manually without a runtime error');
    await page.screenshot({ path: path.join(output, 'key-desktop.png'), fullPage: true });
    await dialog.getByRole('button', { name: 'Sudah disimpan, tutup', exact: true }).click();
    check(await page.locator('input[value="fixture-application-secret-not-real"]').count() === 0, 'closing the result clears the application secret from the DOM');
    await page.getByRole('button', { name: 'Sertifikat & pengaturan', exact: true }).click();
    dialog = page.getByRole('dialog');
    check(await dialog.locator('textarea').count() === 0, 'automatic TLS does not ask the operator to copy PEM or private keys');
    check(await dialog.getByRole('button', { name: 'Generate & aktifkan', exact: true }).isDisabled(), 'automatic issuance requires explicit consent');
    await dialog.getByLabel('Email sertifikat', { exact: true }).fill('admin@example.test');
    await dialog.getByRole('checkbox').check();
    await dialog.getByRole('button', { name: 'Generate & aktifkan', exact: true }).click();
    await dialog.getByText(/Penerbitan dan aktivasi berjalan di latar belakang/).waitFor();
    await dialog.locator('.dialog-footer').getByRole('button', { name: 'Tutup', exact: true }).click();
    await page.getByRole('button', { name: 'Sertifikat & pengaturan', exact: true }).click();
    await page.getByRole('dialog').waitFor({ state: 'hidden' });
    check(tls.state === 'active' && tls.auto_renew, 'background issuance survives closing and reopening the dialog');
    check(writes.filter(value => value.p === '/api/v1/storage/managed/tls').length === 1, 'reopening never duplicates a CA issuance request');
    role = 'user'; await page.reload();
    await page.getByRole('heading', { name: 'Project media', exact: true }).waitFor();
    check(await page.getByRole('button', { name: 'Access key aplikasi', exact: true }).count() === 0 && await page.getByRole('button', { name: 'Buat bucket Qinpanel', exact: true }).count() === 0, 'tenant view exposes own objects without service/key administration');
    role = 'admin'; await page.reload();
    await page.getByRole('button', { name: 'Hapus bucket kosong', exact: true }).click();
    dialog = page.getByRole('dialog');
    const input = dialog.locator('input'); await input.fill('alice-media');
    await dialog.getByRole('button', { name: 'Hapus bucket kosong', exact: true }).click();
    await dialog.waitFor({ state: 'hidden' });
    check(connection === null, 'empty-bucket removal carries typed confirmation to its dedicated endpoint');
    check(writes.every(value => value.csrf === 'fixture-csrf'), 'all managed mutations carry CSRF');
    check(missing.length === 0, `all API requests have fixtures: ${missing.join(', ')}`);
    check(errors.length === 0, `no browser runtime errors: ${errors.join(', ')}`);
    check(external.length === 0, 'the browser smoke never contacts an external storage provider');
    console.log(`${checks} managed storage UI fixture checks passed.`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
