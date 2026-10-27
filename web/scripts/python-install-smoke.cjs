/* Intercepted API fixtures only. No panel daemon, database server or Python installation. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
(async () => {
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || undefined });
  try {
    const page = await browser.newPage({ viewport: { width: 390, height: 844 } });
    page.setDefaultTimeout(20000);
    const errors = [], installs = [];
    const mode = { catalogReady: false, catalogFail: true, installFail: true, accepted: false, phase: 'queued', installed: false, inventoryFail: false, delay: false, release: null };
    page.on('pageerror', error => errors.push(error.message));
    const job = () => ({ id: 'python-install-fixture', user_id: 1, name: 'python.install', status: mode.phase, progress: mode.phase === 'running' ? 42 : mode.phase === 'succeeded' ? 100 : 0, payload_json: '{"version":"3.15.0","base_dir":null}', created_at: 1, updated_at: 1 });
    const catalog = () => ({ enabled: true, interval_hours: 24, sources: ['php', 'node', 'python'].map(runtime => ({ runtime, checked_at: 1, refreshed_at: runtime === 'python' && !mode.catalogReady ? null : 1, stale: runtime === 'python' && !mode.catalogReady, error: runtime === 'python' && mode.catalogFail ? 'Metadata Python sementara tidak tersedia' : null })) });
    await page.route('**/api/v1/**', async route => {
      const request = route.request(), url = new URL(request.url()), name = url.pathname;
      let body = {}, status = 200;
      if (name === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (name === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (name === '/api/v1/runtimes/catalog') body = catalog();
      else if (name === '/api/v1/runtimes/catalog/refresh') { if (!mode.catalogFail) mode.catalogReady = true; body = catalog(); }
      else if (name === '/api/v1/php/versions') body = { ppa_configured: true, versions: [] };
      else if (name === '/api/v1/runtimes/node/versions') body = { versions: [], installed_count: 0 };
      else if (name === '/api/v1/runtimes/python/versions') {
        if (mode.inventoryFail) { status = 503; body = { error: 'Inventaris Python sementara tidak tersedia' }; }
        else body = { catalog_ready: mode.catalogReady, versions: (mode.catalogReady || mode.installed) ? [{ version: '3.15.0', major_minor: '3.15', is_default: true, is_lts_or_stable: true, description: 'CPython stabil fixture', is_installed: mode.installed }] : [{ version: '3.12.9', major_minor: '3.12', is_default: true, is_lts_or_stable: true, description: 'Fallback fixture', is_installed: false }] };
      } else if (name === '/api/v1/runtimes/python/install') {
        installs.push(request.postDataJSON());
        if (mode.delay) await new Promise(resolve => { mode.release = resolve; });
        if (mode.installFail) { status = 503; body = { error: 'Antrean Python sementara tidak tersedia' }; }
        else { mode.accepted = true; status = 202; body = job(); }
      } else if (name === '/api/v1/jobs') {
        const items = mode.accepted && url.searchParams.get('status') === mode.phase ? [job()] : [];
        body = { items, total: items.length, total_pages: 1, page: 1, per_page: 100 };
      } else if (name === '/api/v1/jobs/python-install-fixture') body = job();
      else if (name.includes('unread-count')) body = { count: 0, unread_count: 0 };
      else if (name.includes('notifications')) body = { notifications: [], total: 0 };
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    await page.goto(base + '/runtime?tab=python', { waitUntil: 'domcontentloaded' });
    const open = page.getByText('Pasang versi Python lainnya', { exact: true });
    await open.waitFor();
    await page.getByRole('heading', { name: 'Daftar versi Python belum tersedia' }).waitFor();
    check(await open.isVisible(), 'empty catalog retains an inline manual installation option');
    check(await page.getByRole('button', { name: 'Python', exact: true }).getAttribute('aria-pressed') === 'true', 'direct runtime URL opens the Python tab');
    check(await page.getByRole('region', { name: 'Jadwal katalog runtime' }).count() === 0, 'Python installation tab leaves catalog scheduling in Settings');
    check(installs.length === 0, 'opening Python does not install a runtime');
    await fs.mkdir(path.join(__dirname, '../artifacts/python-install'), { recursive: true });
    await page.screenshot({ path: path.join(__dirname, '../artifacts/python-install/empty-390.png'), fullPage: true });
    await open.click();
    const form = page.locator('details form');
    const submit = form.locator('button[type="submit"]');
    await form.waitFor();
    check(await submit.isDisabled(), 'empty automatic version choice does not submit an invalid request');
    await page.getByRole('button', { name: 'Perbarui katalog Python', exact: true }).click();
    await page.getByText('Metadata Python sementara tidak tersedia', { exact: false }).first().waitFor();
    check(installs.length === 0, 'catalog errors stay visible without starting installation');
    const version = form.getByLabel('Nomor versi lengkap', { exact: true });
    for (const invalid of ['3.15', '3.15.0;id', '../3.15.0']) {
      await version.fill(invalid); check(await submit.isDisabled(), `rejects invalid manual version ${invalid}`);
    }
    await version.fill('3.15.0');
    check(await submit.isEnabled(), 'full version can be installed when the automatic catalog is unavailable');
    await page.screenshot({ path: path.join(__dirname, '../artifacts/python-install/manual-390.png'), fullPage: true });
    await submit.click();
    await form.getByRole('alert').filter({ hasText: 'Antrean Python sementara tidak tersedia' }).waitFor();
    check(await version.inputValue() === '3.15.0' && await submit.isEnabled(), 'failed enqueue preserves the entered version and allows retry');
    mode.installFail = false; mode.delay = true;
    await submit.click();
    await page.waitForFunction(() => document.querySelector('button[type="submit"]')?.textContent?.includes('Membuat tugas'));
    check(await submit.isDisabled() && await page.getByRole('dialog').count() === 0, 'manual installation blocks duplicate submission without opening a dialog');
    await page.getByRole('button', { name: /^Node\.js/ }).click();
    await page.getByRole('button', { name: 'Python', exact: true }).click();
    while (!mode.release) await new Promise(resolve => setTimeout(resolve, 10));
    mode.delay = false; mode.release();
    const progress = page.getByRole('progressbar', { name: 'Progres tugas Python 3.15.0' });
    await progress.waitFor();
    check(await page.getByRole('dialog').count() === 0, 'navigation during enqueue preserves progress without a dialog');
    check(installs.length === 2 && JSON.stringify(installs[1]) === '{"version":"3.15.0"}', 'manual installation sends a typed version-only request and tracks the job');
    await page.reload({ waitUntil: 'domcontentloaded' }); await progress.waitFor();
    check(await open.isVisible(), 'reload preserves the Python tab and queued installation');
    await open.click();
    await version.fill('3.15.0'); check(await submit.isDisabled() && await form.getByText('Versi ini sedang dipasang. Pantau progres pada daftar runtime.', { exact: true }).isVisible(), 'the same queued version cannot be installed again');
    mode.catalogFail = false;
    await page.getByRole('button', { name: 'Perbarui katalog Python', exact: true }).click();
    await page.getByText('CPython stabil fixture', { exact: false }).waitFor();
    check(await submit.isDisabled(), 'refreshed catalog shows real choices while blocking the active version');
    await page.getByRole('button', { name: 'Pengaturan', exact: true }).click();
    const schedule = page.getByRole('region', { name: 'Jadwal katalog runtime' });
    await schedule.getByLabel('Interval pembaruan katalog').waitFor();
    check(await schedule.getByText('Belum tersinkronisasi', { exact: true }).count() === 0, 'Settings shows the catalog refreshed directly from the Python tab');
    await page.getByRole('button', { name: 'Python', exact: true }).click();
    await progress.waitFor();
    mode.phase = 'running';
    await page.waitForFunction(() => document.querySelector('progress[aria-label="Progres tugas Python 3.15.0"]')?.value === 42);
    check(await progress.getAttribute('value') === '42', 'manual installation displays persisted job progress');
    mode.phase = 'succeeded'; mode.inventoryFail = true;
    await page.getByRole('alert').filter({ hasText: 'Inventaris Python sementara tidak tersedia' }).waitFor();
    check(await progress.count() === 1, 'success remains tracked until inventory can confirm installation');
    mode.inventoryFail = false; mode.installed = true;
    await page.getByRole('button', { name: 'Hapus Python 3.15.0', exact: true }).waitFor(); await progress.waitFor({ state: 'detached' });
    await open.click();
    await version.fill('3.15.0'); check(await submit.isDisabled() && await form.getByText('Versi ini sudah terpasang.', { exact: true }).isVisible(), 'installed versions are protected from duplicate installation');
    await page.getByRole('button', { name: /^Node\.js/ }).click(); await page.goBack(); await open.waitFor();
    check(new URL(page.url()).searchParams.get('tab') === 'python', 'browser history returns to the Python runtime tab');
    check(installs.length === 2 && !await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), 'navigation does not duplicate requests and layout fits 390px');
    check(errors.length === 0, `no browser exceptions (${errors.join(', ')})`);
    console.log(`${checks} Python installation checks passed (API fixtures only).`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
