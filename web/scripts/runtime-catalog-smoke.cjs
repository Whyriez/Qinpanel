/* API fixtures only. Never starts a panel daemon or installs a runtime. */
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
    const calls = [], errors = [];
    const mode = { enabled: true, interval_hours: 24, installFail: true, accepted: false, phase: 'queued', installed: false, inventoryFail: false, conflict: true, refreshed: 1 };
    const job = () => ({ id: 'python-fixture', user_id: 1, name: 'python.install', status: mode.phase, progress: mode.phase === 'running' ? 42 : mode.phase === 'succeeded' ? 100 : 0, payload_json: '{"version":"3.15.0","base_dir":null}', result_json: null, error_message: null, retry_count: 0, max_retries: 1, created_at: 1, updated_at: 1 });
    const catalog = () => ({ enabled: mode.enabled, interval_hours: mode.interval_hours, sources: ['php', 'node', 'python'].map(runtime => ({ runtime, checked_at: mode.refreshed, refreshed_at: mode.refreshed, error: null, stale: false })) });
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/api/v1/**', async route => {
      const request = route.request(), url = new URL(request.url()), name = url.pathname, method = request.method();
      calls.push({ name, method, payload: request.postDataJSON() });
      let body = {}, status = 200;
      if (name === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (name === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (name === '/api/v1/runtimes/catalog') {
        if (method === 'PUT') Object.assign(mode, request.postDataJSON());
        body = catalog();
      } else if (name === '/api/v1/runtimes/catalog/refresh') { mode.refreshed = Math.floor(Date.now() / 1000); body = catalog(); }
      else if (name === '/api/v1/php/versions') body = { ppa_configured: true, versions: [{ version: '8.5', latest_release: '8.5.11', installed: false }] };
      else if (name === '/api/v1/runtimes/python/versions') {
        if (mode.inventoryFail) { status = 503; body = { error: 'Inventaris Python sementara tidak tersedia' }; }
        else body = { catalog_ready: true, versions: [{ version: '3.15.0', major_minor: '3.15', release_date: null, is_default: true, is_lts_or_stable: true, description: 'CPython stabil, masih didukung', is_installed: mode.installed, install_path: mode.installed ? '/opt/panel/runtimes/python/3.15.0' : null, bin_path: null }] };
      } else if (name === '/api/v1/runtimes/python/install') {
        if (mode.installFail) { status = 503; body = { error: 'Antrean Python sementara tidak tersedia' }; }
        else { mode.accepted = true; status = 202; body = job(); }
      } else if (name === '/api/v1/runtimes/python/3.15.0' && method === 'DELETE') {
        if (mode.conflict) { status = 409; body = { error: 'Python 3.15.0 masih digunakan situs' }; }
        else { mode.installed = false; body = { success: true, message: 'Python dihapus', version: '3.15.0' }; }
      } else if (name === '/api/v1/jobs') {
        const filter = url.searchParams.get('status');
        const items = mode.accepted && (!filter || filter === mode.phase) ? [job()] : [];
        body = { items, total: items.length, total_pages: 1, page: 1, per_page: 100 };
      } else if (name === '/api/v1/jobs/python-fixture') body = job();
      else if (name === '/api/v1/runtimes/node/versions') body = { mirror_url: 'https://nodejs.org/dist', installed_count: 1, is_offline_fallback: false, versions: [
        { version: '24.10.0', raw_version: 'v24.10.0', is_installed: false, is_lts: true, lts_codename: 'Krypton', support_status: 'lts', used_by_sites_count: 0, used_by_domains: [], can_delete: false, corepack_enabled: false },
        { version: '18.20.0', raw_version: 'v18.20.0', is_installed: true, installed_path: '/opt/panel/runtimes/node/18.20.0', is_lts: true, lts_codename: 'Hydrogen', support_status: 'eol', used_by_sites_count: 1, used_by_domains: ['existing.example.com'], can_delete: false, corepack_enabled: false }
      ] };
      else if (name.includes('unread-count')) body = { count: 0, unread_count: 0 };
      else if (name.includes('notifications')) body = { notifications: [], total: 0 };
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    await page.goto(base + '/runtime', { waitUntil: 'domcontentloaded' });
    const navigation = page.getByRole('navigation', { name: 'Navigasi runtime' });
    const buttons = navigation.getByRole('button');
    await buttons.nth(5).waitFor();
    check(JSON.stringify(await navigation.locator('button > span:first-of-type').allTextContents()) === JSON.stringify(['PHP', 'Node.js', 'Python', 'Pengaturan', 'Monitoring Proses', 'Server Basis Data']), 'PHP, Node.js, Python and Settings are consecutive navigation choices');
    const tops = await navigation.evaluate(nav => [...nav.querySelectorAll('button')].slice(0, 4).map(button => button.getBoundingClientRect().top));
    check(tops.every(top => top === tops[0]), 'the four primary tabs stay adjacent at 390px');
    check(await page.getByRole('region', { name: 'Jadwal katalog runtime' }).count() === 0, 'PHP tab is dedicated to PHP, with scheduling in Settings');
    await page.getByRole('button', { name: 'Python', exact: true }).click();
    await page.getByRole('button', { name: 'Pasang Python 3.15.0', exact: true }).waitFor();
    check(await page.getByRole('heading', { name: 'Manajer runtime Python' }).isVisible(), 'Python has a dedicated installation tab');
    await page.getByRole('button', { name: 'Pengaturan', exact: true }).click();
    await page.getByRole('heading', { name: 'Pengaturan runtime', exact: true }).waitFor();
    check(new URL(page.url()).searchParams.get('tab') === 'settings', 'Settings has a direct runtime URL');
    await page.getByLabel('Interval pembaruan katalog').selectOption('12');
    await page.waitForFunction(() => document.querySelector('select[aria-label="Interval pembaruan katalog"]')?.value === '12' && !document.querySelector('select[aria-label="Interval pembaruan katalog"]')?.disabled);
    check(mode.interval_hours === 12, 'scheduler interval is persisted through the API');
    await page.getByLabel('Perbarui otomatis', { exact: true }).uncheck();
    await page.waitForTimeout(250);
    check(!mode.enabled, 'catalog scheduler can be disabled');
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.getByLabel('Interval pembaruan katalog').waitFor();
    check(await page.getByLabel('Interval pembaruan katalog').inputValue() === '12' && !await page.getByLabel('Perbarui otomatis', { exact: true }).isChecked(), 'reload preserves Settings and saved schedule values');
    const catalogReads = calls.filter(c => c.name === '/api/v1/runtimes/catalog' && c.method === 'GET').length;
    await page.getByRole('button', { name: 'Segarkan', exact: true }).click();
    await page.getByLabel('Interval pembaruan katalog').waitFor();
    check(calls.filter(c => c.name === '/api/v1/runtimes/catalog' && c.method === 'GET').length > catalogReads, 'Settings refresh reloads catalog status');
    check(!calls.some(c => c.name.includes('/processes')), 'Settings refresh does not request process monitoring');
    await fs.mkdir(path.join(__dirname, '../artifacts/runtime-catalog'), { recursive: true });
    await page.screenshot({ path: path.join(__dirname, '../artifacts/runtime-catalog/settings-390.png'), fullPage: true });
    await page.getByRole('button', { name: 'Perbarui katalog sekarang' }).click();
    await page.getByRole('button', { name: 'Perbarui katalog sekarang' }).waitFor();
    check(mode.refreshed > 1, 'manual catalog refresh updates persisted source timestamps');
    check(!calls.some(c => c.name.endsWith('/install')), 'catalog refresh does not install runtimes');
    await page.getByRole('button', { name: 'Python', exact: true }).click();
    await page.getByRole('button', { name: 'Pasang Python 3.15.0', exact: true }).waitFor();
    const install = page.getByRole('button', { name: 'Pasang Python 3.15.0', exact: true });
    await install.click();
    await page.getByText('Antrean Python sementara tidak tersedia', { exact: false }).first().waitFor();
    check(await install.isEnabled(), 'failed Python enqueue allows retry');
    mode.installFail = false;
    await install.click();
    const progress = page.getByRole('progressbar', { name: 'Progres tugas Python 3.15.0' });
    await progress.waitFor();
    check(mode.accepted, 'Python installation is queued in the background');
    check(await page.getByRole('button', { name: 'Sedang dipasang', exact: true }).isDisabled(), 'duplicate installation is blocked');
    await page.getByRole('button', { name: /^Node\.js/ }).click();
    check(await page.getByRole('region', { name: 'Jadwal katalog runtime' }).count() === 0, 'Node.js tab leaves scheduling in Settings');
    await page.getByText('Dukungan berakhir', { exact: true }).waitFor();
    check(await page.getByText('existing.example.com', { exact: true }).isVisible(), 'installed EOL Node runtime and existing sites remain visible');
    check(await page.getByRole('button', { name: 'Pasang Node.js 24.10.0', exact: true }).isEnabled(), 'supported Node version has a direct installation action');
    check(await page.getByRole('button', { name: 'Pasang Node.js 18.20.0', exact: true }).count() === 0, 'installed EOL runtime is not offered for installation');
    await page.getByText('Pengaturan pemasangan Node.js', { exact: true }).click();
    check(await page.getByRole('combobox').count() === 0 && await page.getByRole('dialog').count() === 0, 'Node options stay inline without a second version picker or dialog');
    await page.getByRole('button', { name: 'Python', exact: true }).click();
    await progress.waitFor();
    check(await page.getByRole('button', { name: 'Sedang dipasang', exact: true }).isDisabled(), 'Python progress is recovered after changing tabs');
    mode.phase = 'running';
    await page.waitForFunction(() => document.querySelector('progress[aria-label="Progres tugas Python 3.15.0"]')?.value === 42);
    check(await progress.getAttribute('value') === '42', 'Python shows actual server job progress');
    mode.phase = 'succeeded'; mode.inventoryFail = true;
    await page.getByRole('alert').filter({ hasText: 'Inventaris Python' }).waitFor();
    check(await progress.count() === 1, 'successful install stays tracked while inventory refresh fails');
    mode.inventoryFail = false; mode.installed = true;
    await page.getByRole('button', { name: 'Hapus Python 3.15.0', exact: true }).waitFor();
    await page.waitForFunction(() => !document.querySelector('progress[aria-label="Progres tugas Python 3.15.0"]'));
    check(await page.getByRole('button', { name: 'Pasang Python 3.15.0', exact: true }).count() === 0, 'successful inventory confirmation removes Install action');
    await page.getByRole('button', { name: 'Hapus Python 3.15.0', exact: true }).click();
    const remove = page.getByRole('dialog', { name: 'Hapus Python 3.15.0?' });
    await remove.waitFor();
    await remove.getByRole('textbox').fill('3.15.0');
    await remove.getByRole('button', { name: 'Hapus runtime', exact: true }).click();
    await page.getByText('Python 3.15.0 masih digunakan situs', { exact: false }).first().waitFor();
    check(mode.installed, '409 deletion conflict preserves the installed Python runtime');
    check(calls.filter(c => c.name === '/api/v1/runtimes/python/install').length === 2, 'navigation and refresh never duplicate install requests');
    check(!await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), 'runtime page fits a 390px viewport');
    check(errors.length === 0, errors.join('\n') || 'no browser exceptions');
    await fs.mkdir(path.join(__dirname, '../artifacts/runtime-catalog'), { recursive: true });
    await page.screenshot({ path: path.join(__dirname, '../artifacts/runtime-catalog/python-390.png'), fullPage: true });
    console.log(`${checks} runtime catalog checks passed`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
