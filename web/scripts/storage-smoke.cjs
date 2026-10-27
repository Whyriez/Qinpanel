/* Production bundle + intercepted S3/API fixtures. No provider credentials or calls. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5206';
const output = path.join(__dirname, '../artifacts/storage');
let checks = 0;
const check = (value, label) => { assert.ok(value, label); console.log(`ok ${++checks}: ${label}`); };
const id = '12345678-1234-4234-8234-123456789abc';
const context = { owners: [{ id: 2, username: 'alice' }], sites: [{ id: 1, owner_id: 2, domain: 'alice.example.test' }, { id: 2, owner_id: 3, domain: 'other.example.test' }], upload_limit_bytes: 33554432 };
const site = { id: 1, user_id: 2, primary_domain: 'alice.example.test', kind: 'static', status: 'active', docroot: '/home/p_alice/sites/alice.example.test/public', created_at: 1, updated_at: 1, domains: [], force_https: false };
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge' });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = [], external = [], writes = [], missing = [];
  let connection = null, role = 'admin', fail = false;
  const files = new Map([['notes.txt', 6], ['<img src=x onerror=alert(1)>.txt', 12], ['images/cover +.png', 42], ['page2.txt', 50]]);
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => { if (!request.url().startsWith(base)) external.push(request.url()); });
  await page.addInitScript(() => { if (!localStorage.getItem('nusa_theme')) localStorage.setItem('nusa_theme', 'light'); });
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), url = new URL(request.url()), p = url.pathname, method = request.method();
    let status = 200, body = {};
    if (method !== 'GET') {
      const data = p.endsWith('/upload') ? { key: url.searchParams.get('key'), overwrite: url.searchParams.get('overwrite'), size: request.postDataBuffer()?.length } : request.postDataJSON();
      writes.push({ p, method, data, csrf: request.headers()['x-csrf-token'] });
      if (p === '/api/v1/storage/connections') {
        connection = { id, owner_id: data.owner_id, site_id: data.site_id, label: data.label, provider: data.provider, endpoint: data.endpoint, region: data.region, addressing: data.addressing, bucket: data.bucket, revision: 1, has_credentials: true, verified_at: null, created_at: 1, updated_at: 1 }; body = connection;
      } else if (p.endsWith('/test')) { body = { success: true, revision: connection.revision, verified_at: Date.now() / 1000 }; }
      else if (p.endsWith('/upload')) { if (fail) { status = 502; body = { error: 'Provider belum tersedia. Muat ulang object sebelum mencoba kembali.' }; } else { files.set(data.key, data.size); body = { success: true }; } }
      else if (p.endsWith('/download')) body = { url: `https://s3.us-east-1.amazonaws.com/project-media/${data.key.split('/').map(encodeURIComponent).join('/')}?X-Amz-Signature=fixture`, expires_at: Math.floor(Date.now() / 1000) + 60 };
      else if (p.endsWith('/delete')) { assert.equal(data.confirmation, data.key); files.delete(data.key); body = { success: true }; }
      else if (p.endsWith('/binding')) { connection.site_id = data.site_id; connection.revision++; body = { success: true }; }
      else if (p === `/api/v1/storage/connections/${id}` && method === 'POST') { assert.equal(data.expected_revision, connection.revision); connection = { ...connection, ...data, revision: connection.revision + 1 }; delete connection.credentials; body = connection; }
      else if (p === `/api/v1/storage/connections/${id}` && method === 'DELETE') { assert.equal(data.confirmation, connection.bucket); connection = null; body = { success: true }; }
      else { status = 405; body = { error: 'Missing write fixture' }; missing.push(`${method} ${p}`); }
    } else if (p === '/api/v1/auth/me') body = { user: { id: role === 'admin' ? 1 : 2, username: role === 'admin' ? 'admin' : 'alice', role, status: 'active' }, csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/storage/managed') body = { configured: false, running: false, endpoint: null, engine_version: '4.50', data_directory: '/var/lib/qonpanel-storage', capacity_bytes: null, available_bytes: null, bucket_count: 0, quota_enforcement: 'none' };
    else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/storage/context') body = context;
    else if (p === '/api/v1/storage/connections') body = { connections: connection && (!url.searchParams.has('site_id') || connection.site_id === Number(url.searchParams.get('site_id'))) ? [connection] : [], upload_limit_bytes: 33554432 };
    else if (p.endsWith('/objects')) {
      const prefix = url.searchParams.get('prefix') || '', token = url.searchParams.get('token');
      const matches = [...files].filter(([key]) => key.startsWith(prefix));
      const prefixes = [...new Set(matches.map(([key]) => key.slice(prefix.length)).filter(key => key.includes('/')).map(key => prefix + key.split('/')[0] + '/'))];
      const objects = matches.filter(([key]) => !key.slice(prefix.length).includes('/')).filter(([key]) => token ? key === 'page2.txt' : key !== 'page2.txt').map(([key, size]) => ({ key, size, modified_at: '2026-10-26T00:00:00Z' }));
      body = { listing: { objects, prefixes, next_token: !prefix && !token ? 'page2' : null }, revision: connection.revision, verified_at: Date.now() / 1000 };
    } else if (p === '/api/v1/sites') body = [site];
    else if (p === '/api/v1/sites/1') body = site;
    else if (p === '/api/v1/sites/1/analytics/timeseries') body = { points: [], data: [], series: [], interval_secs: 60 };
    else if (p === '/api/v1/sites/1/ssl') body = { site_id: 1, has_ssl: false, certificate: null };
    else if (p === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 100, total_pages: 0 };
    else if (p.includes('notifications')) body = { notifications: [], unread_count: 0, total: 0 };
    else { status = 404; body = { error: 'No fixture' }; missing.push(p); }
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  try {
    await page.goto(`${base}/storage`);
    const connect = page.getByRole('button', { name: 'Hubungkan S3', exact: true }); await connect.waitFor(); await page.getByRole('heading', { name: 'Belum ada koneksi storage' }).waitFor();
    check(await page.getByRole('link', { name: 'Storage S3', exact: true }).count() > 0, 'Storage is available in panel navigation');
    await connect.click(); let dialog = page.getByRole('dialog'); await dialog.waitFor();
    check(await dialog.getByLabel('Access key ID').getAttribute('type') === 'password' && await dialog.getByLabel('Secret access key', { exact: true }).getAttribute('type') === 'password', 'provider key fields are write-only passwords');
    await dialog.getByLabel('Nama koneksi').fill('Project media'); await dialog.getByLabel('Provider', { exact: true }).selectOption('aws');
    await dialog.getByLabel('Project', { exact: true }).selectOption('1');
    check(!await dialog.locator('option').filter({ hasText: 'other.example.test' }).count(), 'project picker only offers the selected tenant projects');
    await dialog.getByLabel('Endpoint HTTPS').fill('https://s3.us-east-1.amazonaws.com'); await dialog.getByLabel('Bucket', { exact: true }).fill('project-media');
    await dialog.getByLabel('Access key ID').fill('fixture_access_key'); await dialog.getByLabel('Secret access key', { exact: true }).fill('fixture_secret_key_not_real');
    await dialog.getByRole('button', { name: 'Simpan koneksi', exact: true }).click(); await dialog.waitFor({ state: 'detached' }); await page.getByText('notes.txt', { exact: true }).waitFor();
    check(writes[0].data.owner_id === 2 && writes[0].data.site_id === 1 && writes[0].csrf === 'fixture-csrf', 'saving binds the provider, tenant and project with CSRF');
    check(!(await page.content()).includes('fixture_secret_key_not_real'), 'saved credentials are cleared and never returned to visible page state');
    check(await page.getByText('<img src=x onerror=alert(1)>.txt', { exact: true }).count() === 1 && !await page.locator('.storage-object-label img').count(), 'object keys render as escaped plain text');
    await page.getByRole('button', { name: 'Uji koneksi', exact: true }).click(); await page.getByRole('status').filter({ hasText: 'Daftar bucket berhasil diakses' }).waitFor();
    check(writes.at(-1).p.endsWith('/test'), 'connection test is an explicit read probe');
    await page.getByRole('button', { name: 'Berikutnya', exact: true }).click(); await page.getByText('page2.txt', { exact: true }).waitFor(); check(true, 'opaque provider pagination is usable');
    await page.getByRole('button', { name: 'Sebelumnya', exact: true }).click(); await page.getByText('notes.txt', { exact: true }).waitFor();
    await page.getByRole('button', { name: 'images/', exact: true }).click(); await page.getByText('images/cover +.png', { exact: true }).waitFor(); check(true, 'prefix folder navigation keeps the actual Unicode/plus key');
    await page.getByRole('button', { name: 'Folder induk', exact: true }).click(); await page.getByText('notes.txt', { exact: true }).waitFor();
    await page.getByLabel('Pilih file', { exact: true }).setInputFiles({ name: 'new image.txt', mimeType: 'text/plain', buffer: Buffer.from('fixture content') });
    await page.getByRole('button', { name: 'Upload ke bucket', exact: true }).click(); await page.getByText('new image.txt', { exact: true }).waitFor();
    const upload = writes.find(value => value.p.endsWith('/upload')); check(upload.data.key === 'new image.txt' && upload.data.overwrite === 'false' && upload.data.size === 15, 'upload uses raw file bytes, canonical key and no-overwrite default');
    check(!await page.getByLabel('Izinkan menimpa object bernama sama').isChecked(), 'overwrite remains an explicit opt-in after upload');
    await page.getByRole('button', { name: 'Unduh notes.txt', exact: true }).click(); const download = page.getByRole('link', { name: /Unduh file sekarang/ }); await download.waitFor();
    check((await download.getAttribute('href')).startsWith('https://s3.us-east-1.amazonaws.com/project-media/notes.txt?') && await download.getAttribute('rel') === 'noopener noreferrer', 'download uses an expiring bucket URL with isolated navigation');
    await page.getByRole('button', { name: 'Hapus notes.txt', exact: true }).click(); dialog = page.getByRole('dialog');
    check(await dialog.getByRole('button', { name: 'Hapus object', exact: true }).isDisabled(), 'delete requires typed object confirmation');
    await dialog.getByPlaceholder('notes.txt', { exact: true }).fill('notes.txt'); await dialog.getByRole('button', { name: 'Hapus object', exact: true }).click(); await dialog.waitFor({ state: 'detached' });
    await page.getByRole('status').filter({ hasText: 'Provider mengonfirmasi penghapusan' }).waitFor(); check(!files.has('notes.txt'), 'confirmed delete is sent to the selected bucket only');
    await page.getByRole('button', { name: 'Edit koneksi', exact: true }).click(); dialog = page.getByRole('dialog');
    check(await dialog.getByLabel('Access key ID').inputValue() === '' && await dialog.getByLabel('Secret access key', { exact: true }).inputValue() === '', 'editing never repopulates credential fields');
    await page.keyboard.press('Escape'); await dialog.waitFor({ state: 'detached' }); check(await page.getByRole('button', { name: 'Edit koneksi', exact: true }).evaluate(el => el === document.activeElement), 'Escape returns focus to the connection editor trigger');
    await page.screenshot({ path: path.join(output, 'desktop-light.png'), fullPage: true });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => localStorage.setItem('nusa_theme', theme), theme); await page.reload(); await page.getByText('new image.txt', { exact: true }).waitFor();
      check(await page.locator('html').evaluate(el => el.classList.contains('dark')) === (theme === 'dark'), `${theme}: actual theme is active`);
      for (const width of [320, 375, 768, 1440]) {
        await page.setViewportSize({ width, height: 1000 });
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${theme}: storage has no horizontal overflow at ${width}px`);
        const heights = await page.locator('.storage-page button:visible').evaluateAll(nodes => nodes.map(el => el.getBoundingClientRect().height));
        check(heights.every(height => height >= 43.5), `${theme}: storage buttons have 44px touch targets at ${width}px`);
        if (width === 375) await page.screenshot({ path: path.join(output, `mobile-${theme}.png`), fullPage: true });
      }
    }
    await page.setViewportSize({ width: 320, height: 1000 }); await page.evaluate(() => { document.documentElement.style.zoom = '2'; });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'storage fits 320px at 200% zoom'); await page.evaluate(() => { document.documentElement.style.zoom = ''; });
    await page.setViewportSize({ width: 375, height: 900 }); await connect.click(); dialog = page.getByRole('dialog');
    check(await dialog.evaluate(el => el.scrollWidth <= el.clientWidth), 'connection dialog fits mobile width'); await page.screenshot({ path: path.join(output, 'connection-mobile.png') }); await page.keyboard.press('Escape');
    await page.goto(`${base}/sites/alice.example.test/storage`); await page.locator('.storage-page').getByText('new image.txt', { exact: true }).waitFor();
    check(await page.locator('.storage-page').getByText('Koneksi untuk project ini.', { exact: false }).count() > 0, 'project Storage tab uses the same tenant-bound bucket browser');
    await page.goto(`${base}/storage`); await page.getByText('new image.txt', { exact: true }).waitFor(); fail = true;
    await page.getByLabel('Pilih file', { exact: true }).setInputFiles({ name: 'retry.txt', mimeType: 'text/plain', buffer: Buffer.from('fixture') }); await page.getByRole('button', { name: 'Upload ke bucket', exact: true }).click(); await page.getByRole('alert').filter({ hasText: 'Provider belum tersedia' }).waitFor();
    check(writes.filter(value => value.p.endsWith('/upload')).length === 2, 'a failed upload is reported without automatic retry or false success'); fail = false;
    await page.setViewportSize({ width: 1440, height: 1000 }); await page.locator('.language-button').click(); await page.getByRole('button', { name: 'Connect S3', exact: true }).waitFor(); check(true, 'storage supports English copy');
    role = 'user'; await page.reload(); await page.locator('.language-button').click(); await page.getByText('new image.txt', { exact: true }).waitFor(); check(await page.getByRole('button', { name: 'Connect S3', exact: true }).count() === 0 && await page.getByRole('button', { name: 'Edit connection', exact: true }).count() === 0, 'tenant sees own storage without administrator credential controls');
    check(writes.every(value => value.csrf === 'fixture-csrf'), 'all mutations carry the session CSRF token');
    check(external.length === 0, 'fixture run made no external storage/provider requests'); check(errors.length === 0, `no runtime errors: ${errors.join('; ')}`); check(missing.length === 0, `all API calls matched fixtures: ${missing.join('; ')}`);
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors, external, writes: writes.map(({ p, method, csrf }) => ({ p, method, csrf })) }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
