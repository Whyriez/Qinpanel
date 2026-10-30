/* Local production UI; intercepted APIs plus a loopback-only download fixture. */
const { chromium } = require('playwright');
const http = require('node:http');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5206';
const output = path.join(__dirname, '../artifacts/storage-workspace');
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aTmcAAAAASUVORK5CYII=', 'base64');
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge', headless: true });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, permissions: ['clipboard-read', 'clipboard-write'] });
  const page = await context.newPage();
  const errors = [], missing = [], mutations = [];
  let checks = 0, failAccess = false, role = 'user', impersonated = false;
  const check = (value, label) => { assert.ok(value, label); console.log(`ok ${++checks}: ${label}`); };
  const connection = { id: '7c03851f-60dd-49d1-a3aa-a2100320e7c9', owner_id: 2, site_id: 1, label: 'Media project', provider: 'compatible', endpoint: 'https://storage.example.test', region: 'us-east-1', addressing: 'path', bucket: 'alice-media', revision: 1, verified_at: 1, has_credentials: true, managed: true, managed_state: 'ready', application_key_generation: 1, application_key_state: 'ready', created_at: 1, updated_at: 1 };
  let access = { revision: 1, visibility: 'private', desired_visibility: 'private', state: 'ready', request_id: null, can_change: true };
  const objects = [{ key: 'photo + café.png', size: png.length, modified_at: '2026-10-30T00:00:00Z' }, { key: 'archive.zip', size: 4200, modified_at: '2026-10-30T00:00:00Z' }, { key: 'report.pdf', size: 8200, modified_at: '2026-10-30T00:00:00Z' }];
  // Chromium downloads can be reissued outside Playwright interception. Serve
  // only the exact fixture object through Vite's existing loopback API proxy.
  const downloadFixture = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://127.0.0.1');
    if (req.method !== 'GET' || url.pathname !== `/api/v1/storage/connections/${connection.id}/download` || url.searchParams.get('key') !== objects[0].key || url.searchParams.get('expected_revision') !== '1') {
      missing.push(`download fixture: ${req.method} ${url.pathname}`); res.writeHead(404).end(); return;
    }
    res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Length': png.length, 'Content-Disposition': 'attachment; filename="photo.png"', 'Cache-Control': 'no-store' }); res.end(png);
  });
  page.on('pageerror', error => errors.push(error.message));
  await context.route('https://storage.example.test/**', route => route.fulfill({ status: 200, contentType: 'image/png', body: png }));
  await context.route('**/api/v1/**', async route => {
    const req = route.request(), url = new URL(req.url()), p = url.pathname, method = req.method();
    let body = {}, code = 200;
    if (method === 'POST') {
      assert.equal(req.headers()['x-csrf-token'], 'fixture-csrf');
      if (p.endsWith('/upload')) {
        const key = url.searchParams.get('key');
        objects.push({ key, size: req.postDataBuffer().length, modified_at: '2026-10-30T01:00:00Z' });
        mutations.push({ upload: key });
      } else if (p.endsWith('/access')) {
        const data = req.postDataJSON(); mutations.push(data);
        if (access.state === 'unknown') assert.equal(data.request_id, access.request_id);
        else { assert.equal(data.expected_revision, access.revision); access.revision++; }
        access = { ...access, request_id: data.request_id, desired_visibility: data.visibility, state: failAccess ? 'unknown' : 'ready' };
        if (!failAccess) access.visibility = data.visibility;
        body = failAccess ? { error: 'Hasil perubahan akses belum dapat dipastikan.' } : access; code = failAccess ? 503 : 200;
      } else if (p.endsWith('/link')) {
        const data = req.postDataJSON(); mutations.push(data);
        body = { kind: 'signed', url: `https://storage.example.test/alice-media/${data.key.split('/').map(encodeURIComponent).join('/')}?X-Amz-Signature=fixture`, expires_at: Math.floor(Date.now()/1000)+data.expires_in };
      } else if (p.endsWith('/download')) {
        const data = req.postDataJSON(), expires_at = Math.floor(Date.now()/1000)+60;
        body = { url: `${p}?expected_revision=1&key=${encodeURIComponent(data.key)}&expires_at=${expires_at}`, expires_at };
      } else { missing.push(`${method} ${p}`); code = 405; }
    } else if (p === '/api/v1/auth/me') body = { user: { id: 2, username: 'alice', role, status: 'active', impersonated_by: impersonated ? 'admin' : null }, csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/storage/context') body = { owners: [{ id: 2, username: 'alice' }], sites: [{ id: 1, owner_id: 2, domain: 'project.example.test' }], upload_limit_bytes: 33554432 };
    else if (p === '/api/v1/storage/connections') body = { connections: [connection], upload_limit_bytes: 33554432 };
    else if (p.endsWith('/access')) body = access;
    else if (p.endsWith('/objects')) body = { listing: { objects, prefixes: [], next_token: null } };
    else if (p.endsWith('/policy')) body = { revision: 1, quota_bytes: 1073741824, used_bytes: 12400, object_count: objects.length, measured_at: 1, rules: { read: true, write: true, delete: true, max_file_bytes: 33554432, extensions: [] }, enforcement: 'panel', visibility: 'owner' };
    else if (p.endsWith('/download')) {
      if (!url.searchParams.has('preview')) { await route.continue(); return; }
      await route.fulfill({ contentType: 'image/png', headers: { 'Cache-Control': 'no-store' }, body: png }); return;
    }
    else if (p === '/api/v1/storage/managed') body = { configured: true, running: true, endpoint: connection.endpoint, engine_version: '4.50', data_directory: '/var/lib/qonpanel-storage', bucket_count: 1, quota_enforcement: 'none' };
    else if (p === '/api/v1/storage/managed/tls') body = { certificate: { revision: 1, state: 'active', auto_renew: false, method: 'tunnel' }, dns: { available: false, configured: false, zone_name: null } };
    else if (p.includes('notifications')) body = { notifications: [], total: 0, unread_count: 0 };
    else if (p === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 100, total_pages: 0 };
    else { missing.push(`${method} ${p}`); code = 404; }
    await route.fulfill({ status: code, contentType: 'application/json', body: JSON.stringify(body) });
  });
  try {
    await new Promise((resolve, reject) => { downloadFixture.once('error', reject); downloadFixture.listen(7700, '127.0.0.1', resolve); });
    await page.goto(`${base}/storage`);
    await page.getByRole('heading', { name: 'File browser', exact: true }).waitFor();
    await page.getByRole('button', { name: /photo \+ café.png/ }).click();
    const detail = page.getByRole('complementary', { name: 'Detail file', exact: true });
    await detail.locator('.storage-preview img').waitFor();
    check(await detail.getByText('Private', { exact: true }).count() === 1, 'private file has image preview and explicit access');
    await detail.getByLabel(/^Masa berlaku/).selectOption('86400');
    await detail.getByRole('button', { name: 'Dapatkan URL', exact: true }).click();
    await detail.getByRole('link', { name: 'Buka URL', exact: true }).waitFor();
    check((await detail.getByLabel('URL akses sementara', { exact: true }).inputValue()).includes('X-Amz-Signature='), 'private URL is shareable S3 signed URL');
    check(mutations.at(-1).expires_in === 86400, 'selected signed URL duration is submitted');
    await detail.getByRole('button', { name: 'Salin URL', exact: true }).click();
    check((await page.evaluate(() => navigator.clipboard.readText())).includes('X-Amz-Signature='), 'copy URL writes the actual access URL');
    const download = page.waitForEvent('download'); await detail.getByRole('button', { name: 'Unduh file', exact: true }).click();
    const downloaded = await download;
    check(await downloaded.failure() === null, 'download icon completes without a browser download error');
    assert.deepEqual(await fs.readFile(await downloaded.path()), png, 'Downloaded bytes must match the storage object');
    await detail.getByRole('button', { name: 'Tutup detail file', exact: true }).click();
    await page.getByRole('tab', { name: 'Pengaturan bucket', exact: true }).click();
    const editor = page.getByRole('region', { name: 'Akses bucket', exact: true });
    await editor.scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(output, 'bucket-settings.png') });
    await editor.getByRole('radio', { name: /^Public/ }).check();
    check(access.visibility === 'private', 'choosing Public does not publish before saving');
    await editor.getByRole('button', { name: 'Simpan akses bucket', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.storage-workspace-heading .storage-access-badge')?.textContent === 'Public');
    await page.getByRole('tab', { name: 'File', exact: true }).click();
    await page.getByRole('button', { name: 'Tampilan grid', exact: true }).click();
    await page.getByRole('button', { name: /photo \+ café.png/ }).click();
    check(await detail.getByLabel('URL publik', { exact: true }).inputValue() === 'https://storage.example.test/alice-media/photo%20%2B%20caf%C3%A9.png', 'Public URL is canonical, permanent, and has no signature');
    check(await detail.getByRole('button', { name: 'Dapatkan URL', exact: true }).count() === 0, 'public detail exposes URL without minting a download');
    check(await page.locator('.storage-file-thumbnail img').count() > 0, 'grid shows image thumbnails');
    await page.getByRole('heading', { name: 'File browser', exact: true }).scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(output, 'file-workspace.png') });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.classList.toggle('dark', theme === 'dark'), theme);
      for (const [width, height] of [[1440, 1000], [375, 812], [320, 700]]) {
        await page.setViewportSize({ width, height });
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `${theme} ${width}: no page overflow`);
        await detail.scrollIntoViewIfNeeded();
        check((await detail.boundingBox()).width <= width, `${theme} ${width}: file details fit`);
        if (width !== 375) await page.screenshot({ path: path.join(output, `${theme}-${width}.png`), fullPage: true });
      }
    }
    await page.setViewportSize({ width: 1440, height: 1000 });
    await detail.getByRole('button', { name: 'Tutup detail file', exact: true }).click();
    await page.getByRole('button', { name: 'Upload file', exact: true }).click();
    check(await page.locator('input[type=file]').getAttribute('accept') === null, 'upload allows arbitrary file formats');
    await page.locator('input[type=file]').setInputFiles({ name: 'video.mp4', mimeType: 'video/mp4', buffer: Buffer.from('fixture video') });
    await page.getByRole('checkbox', { name: /Izinkan menimpa/ }).check();
    await page.getByRole('button', { name: 'Mulai upload', exact: true }).click();
    await page.getByRole('button', { name: /video.mp4/ }).waitFor();
    check(mutations.some(item => item.upload === 'video.mp4'), 'non-image files use the same storage upload flow');
    await page.getByRole('tab', { name: 'Pengaturan bucket', exact: true }).click();
    await editor.getByRole('radio', { name: /^Private/ }).check(); failAccess = true;
    await editor.getByRole('button', { name: 'Simpan akses bucket', exact: true }).click();
    await editor.getByRole('button', { name: 'Sinkronkan perubahan akses', exact: true }).waitFor();
    check(access.state === 'unknown' && access.visibility === 'public', 'uncertain revoke is not displayed as confirmed Private');
    const requestId = access.request_id; failAccess = false;
    await editor.getByRole('button', { name: 'Sinkronkan perubahan akses', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.storage-workspace-heading .storage-access-badge')?.textContent === 'Private');
    check(access.request_id === requestId && access.visibility === 'private', 'reconciliation resumes the same request');
    impersonated = true; await page.reload();
    await page.getByRole('heading', { name: 'Workspace file milik tenant', exact: true }).waitFor();
    check(await page.getByRole('heading', { name: 'File browser', exact: true }).count() === 0, 'impersonated session has no file workspace');
    impersonated = false; role = 'admin'; await page.reload();
    await page.getByRole('heading', { name: 'Workspace file milik tenant', exact: true }).waitFor();
    check(await page.getByRole('heading', { name: 'File browser', exact: true }).count() === 0, 'admin retains only allocation and metadata controls');
    check(errors.length === 0, `no browser errors: ${errors.join(', ')}`);
    check(missing.length === 0, `all requests handled by fixtures: ${missing.join(', ')}`);
    console.log(`${checks} storage workspace UI checks passed.`);
  } catch (error) { await page.screenshot({ path: path.join(output, 'failure.png'), fullPage: true }); throw error; }
  finally { await browser.close(); if (downloadFixture.listening) await new Promise(resolve => downloadFixture.close(resolve)); }
})().catch(error => { console.error(error); process.exitCode = 1; });
