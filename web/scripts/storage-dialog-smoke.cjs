/* Production UI with API fixtures. Never connects to storage or uses real keys. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5206';
const output = path.join(__dirname, '../artifacts/storage-dialog');
let count = 0;
const check = (value, label) => { assert.ok(value, label); console.log(`ok ${++count}: ${label}`); };

(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge', headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = [], external = [], missing = [];
  let saves = 0;
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => { if (!request.url().startsWith(base)) external.push(request.url()); });
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), p = new URL(request.url()).pathname;
    let status = 200, body = {};
    if (p === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin', status: 'active' }, csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/storage/managed') body = { configured: false, running: false, endpoint: null, engine_version: '4.50', data_directory: '/var/lib/qonpanel-storage', capacity_bytes: null, available_bytes: null, bucket_count: 0, quota_enforcement: 'none' };
    else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/storage/context') body = { owners: [{ id: 2, username: 'tenant-with-a-long-name' }], sites: [{ id: 1, owner_id: 2, domain: 'project-with-a-long-name.example.test' }], upload_limit_bytes: 33554432 };
    else if (p === '/api/v1/storage/connections' && request.method() === 'POST') {
      saves++;
      assert.equal(request.headers()['x-csrf-token'], 'fixture-csrf');
      status = 422; body = { error: 'Endpoint belum dapat diverifikasi. Periksa konfigurasi provider sebelum mencoba kembali.' };
    } else if (p === '/api/v1/storage/connections') body = { connections: [], upload_limit_bytes: 33554432 };
    else if (p.includes('notifications')) body = { notifications: [], unread_count: 0, total: 0 };
    else if (p === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 100, total_pages: 0 };
    else { status = 404; missing.push(p); }
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  try {
    await page.goto(`${base}/storage`);
    const connect = page.getByRole('button', { name: 'Hubungkan S3', exact: true });
    await connect.waitFor();
    for (const theme of ['light', 'dark']) {
      await page.evaluate(value => localStorage.setItem('nusa_theme', value), theme);
      await page.reload(); await connect.click();
      const dialog = page.getByRole('dialog'), panel = dialog.locator('.storage-dialog'), body = dialog.locator('.storage-dialog-body');
      await page.waitForFunction(() => getComputedStyle(document.querySelector('.storage-modal')).opacity === '1');
      check(await page.locator('#root').evaluate(el => el.inert), `${theme}: background is inert while the dialog is open`);
      check(await panel.evaluate(el => getComputedStyle(el).backgroundColor !== 'rgba(0, 0, 0, 0)'), `${theme}: dialog has an opaque surface`);
      for (const [width, height] of [[1440, 1000], [1024, 768], [768, 1024], [560, 800], [390, 844], [375, 667], [320, 568], [844, 390], [1440, 600]]) {
        await page.setViewportSize({ width, height });
        await page.waitForFunction(() => Math.abs(parseFloat(document.querySelector('.storage-modal').style.getPropertyValue('--dialog-viewport-height')) - visualViewport.height) < 1);
        const box = await panel.boundingBox();
        check(box.x >= 12 && box.y >= 12 && box.x + box.width <= width - 12 && box.y + box.height <= height - 12, `${theme} ${width}x${height}: dialog fits the viewport`);
        check(await panel.evaluate(el => el.scrollWidth <= el.clientWidth), `${theme} ${width}x${height}: no horizontal overflow`);
        const header = dialog.locator('.storage-dialog-header'), footer = dialog.locator('.storage-dialog-footer');
        const before = [await header.boundingBox(), await footer.boundingBox()];
        await body.evaluate(el => { el.scrollTop = el.scrollHeight; });
        const after = [await header.boundingBox(), await footer.boundingBox()];
        check(after.every((box, i) => Math.abs(box.y - before[i].y) < 1), `${theme} ${width}x${height}: header and actions remain visible while scrolling`);
        const save = await dialog.getByRole('button', { name: 'Simpan koneksi', exact: true }).boundingBox();
        check(save.y >= 0 && save.y + save.height <= height && save.height >= 44, `${theme} ${width}x${height}: save action stays reachable`);
        const controls = await dialog.locator('input:visible,select:visible,button:visible,summary:visible').evaluateAll(nodes => nodes.map(el => el.getBoundingClientRect().height));
        check(controls.every(height => height >= 43.5), `${theme} ${width}x${height}: controls have 44px touch targets`);
        await body.evaluate(el => { el.scrollTop = 0; });
        if (width === 1440 && height === 1000 || width === 375 && height === 667 || width === 844) {
          await page.screenshot({ path: path.join(output, `${theme}-${width}x${height}.png`), animations: 'disabled' });
        }
      }
      await page.setViewportSize({ width: 1440, height: 1000 });
      const access = await dialog.getByLabel('Access key ID').boundingBox(), secret = await dialog.getByLabel('Secret access key', { exact: true }).boundingBox();
      check(Math.abs(access.y - secret.y) < 1 && secret.x > access.x, `${theme}: credential fields are balanced on desktop`);
      await dialog.locator('.storage-session summary').click();
      await dialog.getByLabel('Session token', { exact: true }).fill('fixture_session');
      check(await dialog.getByLabel('Session token', { exact: true }).inputValue() === 'fixture_session', `${theme}: optional session token can be expanded and entered`);
      await dialog.getByLabel('Provider', { exact: true }).selectOption('compatible');
      check(await dialog.getByText('Gunakan endpoint S3 yang sudah berjalan.', { exact: false }).count() === 1, `${theme}: compatible storage explains the existing endpoint requirement`);
      check(await dialog.getByLabel('Session token', { exact: true }).inputValue() === '', `${theme}: provider change clears the session token`);
      await dialog.getByLabel('Provider', { exact: true }).selectOption('supabase');
      await page.setViewportSize({ width: 320, height: 568 });
      check(await dialog.getByText('Access key S3 Supabase standar', { exact: false }).count() === 1, `${theme}: Supabase permission guidance remains available`);
      await page.screenshot({ path: path.join(output, `${theme}-supabase-320.png`), animations: 'disabled' });
      await dialog.getByRole('button', { name: 'Simpan koneksi', exact: true }).focus(); await page.keyboard.press('Tab');
      check(await dialog.getByRole('button', { name: 'Tutup', exact: true }).evaluate(el => el === document.activeElement), `${theme}: keyboard focus remains trapped`);
      await page.keyboard.press('Escape'); await dialog.waitFor({ state: 'detached' });
      check(await connect.evaluate(el => el === document.activeElement), `${theme}: Escape restores the opener focus`);
    }
    await page.setViewportSize({ width: 375, height: 667 }); await connect.click();
    const dialog = page.getByRole('dialog');
    await dialog.getByLabel('Nama koneksi').fill('Fixture media');
    await dialog.getByLabel('Endpoint HTTPS').fill('https://s3.us-east-1.amazonaws.com');
    await dialog.getByLabel('Bucket', { exact: true }).fill('fixture-media');
    await dialog.getByLabel('Access key ID').fill('fixture_access');
    await dialog.getByLabel('Secret access key', { exact: true }).fill('fixture_secret_not_real');
    await dialog.getByRole('button', { name: 'Simpan koneksi', exact: true }).click();
    const alert = dialog.getByRole('alert'); await alert.waitFor();
    check(await alert.evaluate(el => el === document.activeElement), 'save failure is focused and announced inside the scroll body');
    check(saves === 1 && await dialog.getByRole('button', { name: 'Simpan koneksi', exact: true }).isEnabled(), 'failed save has no automatic retry and keeps actions available');
    await page.screenshot({ path: path.join(output, 'mobile-error.png'), animations: 'disabled' });
    await page.keyboard.press('Escape');
    check(!await page.locator('#root').evaluate(el => el.inert), 'closing the dialog restores page access');
    check(errors.length === 0 && external.length === 0 && missing.length === 0, `no browser exceptions, provider calls or unmatched API requests: ${JSON.stringify({ errors, external, missing })}`);
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks: count, errors, external, missing }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
