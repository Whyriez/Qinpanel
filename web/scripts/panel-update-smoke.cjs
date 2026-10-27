/* Production UI with intercepted fixtures only: never calls an agent or installer. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5184';
let count = 0;
const check = (ok, message) => { assert.ok(ok, message); console.log(`PASS ${++count}: ${message}`); };
(async () => {
  const browser = await chromium.launch({ headless: true, ...(process.env.NUSA_BROWSER_CHANNEL ? { channel: process.env.NUSA_BROWSER_CHANNEL } : {}) });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  page.setDefaultTimeout(20000);
  const errors = [], applies = [];
  const output = path.resolve(__dirname, '../artifacts/panel-update'); fs.mkdirSync(output, { recursive: true });
  let outage = false, delayedStatus = false;
  const data = { diagnostics: null, current_version: 'v0.1.0', installed_digest: null, supported: true, checked_at: null, available: null, check_error: null, update: null };
  page.on('pageerror', e => errors.push(e.message));
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), url = new URL(request.url()); let body = {}, status = 200;
    if (url.pathname === '/api/v1/auth/me') body = { user: { id: 1, role: 'admin', username: 'admin' } };
    else if (url.pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture-csrf' };
    else if (url.pathname === '/api/v1/system/update') { if (delayedStatus) { delayedStatus = false; await new Promise(resolve => setTimeout(resolve, 1400)); return route.abort('failed'); } if (outage) return route.abort('failed'); body = data; }
    else if (url.pathname.endsWith('/update/check')) { data.checked_at = Math.floor(Date.now()/1000); data.available = { version: 'v0.2.0', digest: 'a'.repeat(64), notes: '<script>window.fixtureUnsafe=true</script>\nCatatan rilis dengan perbaikan website.' }; body = data; }
    else if (url.pathname.endsWith('/update/apply')) {
      const payload = request.postDataJSON(); applies.push(payload); check(request.headers()['x-csrf-token'] === 'fixture-csrf', 'apply uses session CSRF');
      data.update = { ...payload, phase: 'queued', started_at: Math.floor(Date.now()/1000), finished_at: null }; body = data; status = 202;
      if (applies.length === 1) { outage = true; return route.abort('failed'); }
    } else if (url.pathname.endsWith('/update/logs')) body = { ...data, diagnostics: Array.from({length:100}, (_,i) => `Log fixture ${i+1}: pemeriksaan paket`).join('\n') };
    else if (url.pathname.endsWith('/jobs')) body = { items: [], total: 0, page: 1, per_page: 1, total_pages: 0 };
    else if (url.pathname.includes('unread-count')) body = { count: 0, unread_count: 0 };
    else if (url.pathname.includes('notifications')) body = { notifications: [], total: 0 };
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  await page.goto(base + '/settings?section=updates');
  await page.getByRole('heading', { name: 'Pembaruan panel', exact: true }).waitFor();
  await page.getByRole('button', { name: 'Periksa pembaruan', exact: true }).waitFor({ state: 'visible' });
  await page.waitForTimeout(300);
  check(await page.getByRole('button', { name: 'Terapkan pembaruan', exact: true }).isDisabled(), 'no update is offered before a release check');
  delayedStatus = true;
  await Promise.all([page.waitForRequest(request => new URL(request.url()).pathname === '/api/v1/system/update'), page.evaluate(() => window.dispatchEvent(new Event('focus')))]);
  await page.getByRole('button', { name: 'Periksa pembaruan', exact: true }).click();
  await page.getByRole('button', { name: 'Terapkan pembaruan', exact: true }).waitFor();
  await page.waitForTimeout(1600);
  check(await page.getByRole('alert').count() === 0, 'a stale status failure cannot overwrite a newer successful release check');
  check(await page.getByRole('button', { name: 'Terapkan pembaruan', exact: true }).isEnabled(), 'fresh pinned release is applicable');
  await page.getByText('Catatan rilis', { exact: true }).click();
  check(!(await page.evaluate(() => window.fixtureUnsafe)), 'release notes remain text, never executable markup');
  await page.getByRole('button', { name: 'Terapkan pembaruan', exact: true }).click();
  await page.getByRole('dialog').waitFor(); await page.keyboard.press('Escape');
  check(applies.length === 0, 'Escape dismisses confirmation without submitting');
  await page.getByRole('button', { name: 'Terapkan pembaruan', exact: true }).click();
  await page.getByRole('button', { name: 'Terapkan versi ini', exact: true }).click();
  await page.getByText('Menunggu konfirmasi server. Menyambungkan kembali ke panel…', { exact: true }).waitFor();
  check(await page.evaluate(() => !!sessionStorage.getItem('qpanel.update.pending')), 'lost apply acknowledgement retains the same request identity');
  await page.reload();
  await page.getByText('Menyambungkan kembali ke panel…', { exact: true }).waitFor();
  check(applies.length === 1, 'reload during an ambiguous apply never submits a second request');
  outage = false;
  await page.getByText('Update dijadwalkan · v0.2.0', { exact: true }).waitFor();
  check(applies.length === 1 && applies[0].digest === 'a'.repeat(64) && Object.keys(applies[0]).sort().join(',') === 'digest,id,version', 'one typed immutable request is submitted');
  check(await page.getByRole('button', { name: 'Terapkan pembaruan', exact: true }).isDisabled(), 'queued update prevents duplicate submissions');
  outage = true;
  await page.waitForTimeout(5500);
  await page.getByText('Menunggu konfirmasi server. Menyambungkan kembali ke panel…', { exact: true }).waitFor();
  check(applies.length === 1, 'API restart is treated as reconnection and never resubmits automatically');
  outage = false; data.update.phase = 'running';
  await page.reload(); await page.getByText('Menerapkan pembaruan · v0.2.0', { exact: true }).waitFor();
  check(applies.length === 1, 'reload restores update progress from server state');
  data.update.phase = 'failed'; data.update.finished_at = Math.floor(Date.now()/1000);
  await page.waitForTimeout(5500); await page.getByText('Pembaruan gagal · v0.2.0', { exact: true }).waitFor();
  await page.getByRole('button', { name: 'Lihat diagnostik', exact: true }).click();
  const logs = page.getByLabel('Diagnostik pembaruan', { exact: true }); await logs.waitFor();
  check(await logs.evaluate(e => e.scrollHeight > e.clientHeight), 'diagnostics preserve complete lines with their own scroll area');
  await logs.focus(); await page.keyboard.press('End'); await page.waitForTimeout(100);
  check(await logs.evaluate(e => e.scrollTop > 0), 'diagnostics can be scrolled with the keyboard');
  await page.keyboard.press('Home');
  data.update.phase = 'succeeded'; data.current_version = 'v0.2.0'; data.installed_digest = 'a'.repeat(64);
  await page.waitForTimeout(5500); await page.getByRole('button', { name: 'Muat ulang panel', exact: true }).waitFor();
  check(await page.getByRole('button', { name: 'Terapkan pembaruan', exact: true }).isDisabled(), 'installed package is not offered repeatedly');
  await page.screenshot({ path: path.join(output, 'desktop.png'), fullPage: true });
  for (const width of [390,320]) {
    await page.setViewportSize({ width, height: 840 });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `no horizontal overflow at ${width}px`);
  }
  await page.screenshot({ path: path.join(output, 'mobile.png'), fullPage: true });
  const light = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--bg'));
  await page.evaluate(() => localStorage.setItem('nusa_theme', 'dark'));
  await page.setViewportSize({ width: 1280, height: 900 }); await page.reload();
  await page.getByRole('button', { name: 'Muat ulang panel', exact: true }).waitFor();
  check(await page.evaluate(value => getComputedStyle(document.documentElement).getPropertyValue('--bg') !== value, light), 'dark theme keeps the update state with the shared color tokens');
  await page.screenshot({ path: path.join(output, 'dark.png'), fullPage: true });
  check(errors.length === 0, `no browser exceptions: ${errors.join('; ')}`);
  await browser.close(); console.log(`${count} panel update browser checks passed; all API traffic intercepted.`);
})().catch(error => { console.error(error); process.exitCode = 1; });
