/* Static frontend + API fixtures only. No panel-api/agent or database service. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5190';
const output = path.join(__dirname, '../artifacts/database-tab');
const domain = 'testing.incogmsg.my.id';
const site = { id: 1, user_id: 2, primary_domain: domain, kind: 'php', version: '8.3', preset: 'generic', docroot: `/home/p_alice/sites/${domain}/public`, status: 'active', spa_fallback: false, force_https: false, created_at: 1770000000, updated_at: 1770000000, domains: [] };
const database = { id: 7, user_id: 2, site_id: 1, engine: 'mariadb', name: 'p_alice_shop', db_user: 'p_alice_shop', remote_access: false, allowed_hosts: 'localhost', collation: 'utf8mb4_unicode_ci', character_set: 'utf8mb4', size_bytes: 0, created_at: 1770000000, updated_at: 1770000000 };
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || 'msedge' });
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce' });
    page.setDefaultTimeout(10000);
    const errors = [], requests = [], rotations = [];
    let mode = 'empty', releaseList;
    let engines = [{ engine: 'mariadb', label: 'MariaDB', port: 3306 }], engineFailure = false;
    const creates = [];
    const listGate = new Promise(resolve => { releaseList = resolve; });
    page.on('pageerror', error => errors.push(error.message));
    await page.addInitScript(() => { localStorage.setItem('nusa_theme', 'light'); });
    await page.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url());
      requests.push({ path: url.pathname, query: url.search });
      let status = 200, body = {};
      if (url.pathname === '/api/v1/auth/me') body = { user: { id: 2, username: 'alice', role: 'user' }, csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/sites') body = [site];
      else if (url.pathname === '/api/v1/sites/1') body = site;
      else if (url.pathname === '/api/v1/sites/1/ssl') body = { site_id: 1, has_ssl: false, certificate: null };
      else if (url.pathname === '/api/v1/databases/engines') {
        if (engineFailure) { status = 503; body = { error: 'Pemeriksaan mesin fixture gagal.' }; }
        else body = engines;
      }
      else if (url.pathname === '/api/v1/databases' && req.method() === 'POST') {
        creates.push(req.postDataJSON()); status = 422; body = { error: 'Mesin fixture tidak tersedia lagi.' };
      }
      else if (url.pathname === '/api/v1/databases') {
        await listGate;
        if (mode === 'malformed') body = { databases: [], total: 0 };
        else if (mode === 'invalid-row') body = { databases: [{ ...database, db_user: null }], total: 1, plan_max_databases: 2, used_databases: 1 };
        else if (mode === 'failure') { status = 500; body = { error: 'Gagal mengambil daftar basis data: fixture unavailable' }; }
        else body = { databases: mode === 'empty' ? [] : [database], total: mode === 'empty' ? 0 : 1, plan_max_databases: mode === 'unlimited' ? 0 : 2, used_databases: mode === 'empty' ? 0 : 2 };
      }
      else if (url.pathname === '/api/v1/databases/7/rotate-password') {
        const payload = JSON.parse(req.postData()); rotations.push(payload);
        body = { success: true, new_password: payload.new_password || 'fixture-generated-password', message: 'Updated' };
      }
      else if (url.pathname.includes('timeseries')) body = { points: [] };
      else if (url.pathname === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 1 };
      else if (url.pathname.includes('notifications')) body = { notifications: [], count: 0, unread_count: 0, total: 0 };
      else { status = 404; body = { error: 'No fixture' }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    await page.goto(`${base}/sites/${domain}/database`, { waitUntil: 'domcontentloaded' });
    await page.getByText('Memuat daftar basis data...', { exact: true }).waitFor();
    check(await page.getByRole('button', { name: 'Buat Basis Data', exact: true }).isDisabled(), 'create stays disabled until quota is known');
    check(await page.getByText('Memuat kuota...', { exact: true }).isVisible(), 'loading does not invent a quota of five');
    releaseList();
    await page.getByRole('heading', { name: 'Belum ada basis data', exact: true }).waitFor();
    check(await page.getByRole('button', { name: 'Buat Basis Data', exact: true }).isEnabled(), 'real empty Rust response renders and permits create');
    check((await page.locator('[role="progressbar"]').getAttribute('aria-valuemax')) === '2', 'flat plan_max_databases is mapped to quota');
    check(requests.some(req => req.path === '/api/v1/databases' && req.query === '?site_id=1'), 'database list remains scoped to the project');
    await page.screenshot({ path: path.join(output, 'empty-light.png') });
    mode = 'malformed';
    await page.getByTitle('Muat Ulang', { exact: true }).click();
    await page.getByRole('heading', { name: 'Daftar basis data belum tersedia' }).waitFor();
    check(await page.getByText('Format respons basis data tidak valid. Perbarui panel lalu coba lagi.', { exact: true }).isVisible(), 'incomplete HTTP 200 becomes a useful error instead of a blank page');
    check(await page.getByRole('button', { name: 'Buat Basis Data', exact: true }).isDisabled(), 'unknown quota never enables create');
    check(await page.getByRole('link', { name: 'Ringkasan', exact: true }).isVisible(), 'site navigation survives a malformed response');
    await page.screenshot({ path: path.join(output, 'contract-error-light.png') });
    mode = 'full';
    await page.getByRole('button', { name: 'Coba lagi', exact: true }).click();
    await page.getByText(database.name, { exact: true }).first().waitFor();
    check(await page.getByRole('button', { name: 'Buat Basis Data', exact: true }).isDisabled(), 'tenant-wide used count enforces quota even with one visible row');
    const quota = page.getByRole('progressbar');
    check(await quota.getAttribute('aria-valuenow') === '2', 'quota usage is not inferred from filtered rows');
    await page.getByPlaceholder('Cari nama basis data...').fill('not-found');
    await page.getByText('Tidak ada basis data yang cocok dengan kueri pencarian.', { exact: true }).waitFor();
    await page.getByPlaceholder('Cari nama basis data...').fill('');
    await page.getByText(database.name, { exact: true }).first().waitFor();
    check(true, 'database search works after normalization');
    await page.getByRole('button', { name: 'Kata Sandi', exact: true }).click();
    await page.getByRole('checkbox', { name: 'Acak kata sandi kuat secara otomatis (Rekomendasi)' }).uncheck();
    await page.getByPlaceholder('Minimal 12 karakter aman...').fill('fixture-selected-password');
    await page.getByRole('button', { name: 'Terapkan Kata Sandi', exact: true }).click();
    await page.getByText('Kata sandi baru berhasil dibuat:', { exact: true }).waitFor();
    check(rotations.length === 1 && rotations[0].new_password === 'fixture-selected-password' && !('password' in rotations[0]), 'manual rotation sends new_password rather than silently generating a password');
    await page.getByRole('button', { name: 'Selesai', exact: true }).click();
    mode = 'unlimited';
    await page.getByTitle('Muat Ulang', { exact: true }).click();
    await page.getByText(/2 \/ Tanpa batas Basis Data/).waitFor();
    check(await page.getByRole('button', { name: 'Buat Basis Data', exact: true }).isEnabled(), 'nonpositive quota limit is shown as unlimited');
    await page.getByRole('button', { name: 'Ganti tema', exact: true }).click();
    await page.waitForFunction(() => document.documentElement.classList.contains('dark'));
    await page.screenshot({ path: path.join(output, 'loaded-dark.png') });
    mode = 'invalid-row';
    await page.getByTitle('Muat Ulang', { exact: true }).click();
    await page.getByRole('heading', { name: 'Daftar basis data belum tersedia' }).waitFor();
    check(await page.getByRole('button', { name: 'Buat Basis Data', exact: true }).isDisabled(), 'malformed database rows are rejected before search/render');
    mode = 'failure';
    await page.getByRole('button', { name: 'Coba lagi', exact: true }).click();
    await page.getByText('Gagal mengambil daftar basis data: fixture unavailable', { exact: true }).waitFor();
    check(await page.getByRole('button', { name: 'Coba lagi', exact: true }).isVisible(), 'HTTP 500 preserves diagnostics and retry');
    mode = 'empty';
    await page.getByRole('button', { name: 'Coba lagi', exact: true }).click();
    await page.getByRole('heading', { name: 'Belum ada basis data', exact: true }).waitFor();
    check(await page.getByRole('button', { name: 'Buat Basis Data', exact: true }).isEnabled(), 'retry restores the database tab');

    const create = page.getByRole('button', { name: 'Buat Basis Data', exact: true }).first();
    await create.click();
    const dialog = page.getByRole('dialog');
    await dialog.getByRole('radio', { name: 'MariaDB Port 3306' }).waitFor();
    check(await dialog.getByRole('radio').count() === 1, 'only the live installed and running engine is offered');
    check(await dialog.getByRole('radio', { name: /PostgreSQL/ }).count() === 0, 'uninstalled PostgreSQL is hidden');
    engines = [{ engine: 'postgresql', label: 'PostgreSQL', port: 5432 }];
    await dialog.getByRole('button', { name: 'Muat ulang pilihan mesin' }).click();
    await dialog.getByRole('radio', { name: 'PostgreSQL Port 5432' }).waitFor();
    check(await dialog.getByRole('radio', { name: /MariaDB/ }).count() === 0, 'refresh removes the previous engine selection');
    check(await dialog.getByRole('radio').isChecked(), 'one newly available engine becomes the current choice');
    await dialog.getByPlaceholder('contoh: wordpress').fill('shop');
    await dialog.getByRole('button', { name: 'Buat Basis Data', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: 'Mesin fixture tidak tersedia lagi.' }).waitFor();
    check(creates.length === 1 && creates[0].engine === 'postgresql' && creates[0].site_id === 1, 'create submits only the current live engine and project ID');
    engines = [];
    await dialog.getByRole('button', { name: 'Muat ulang pilihan mesin' }).click();
    await dialog.getByText(/Belum ada mesin basis data yang siap digunakan/).waitFor();
    check(await dialog.getByRole('radio').count() === 0 && await dialog.getByRole('button', { name: 'Buat Basis Data', exact: true }).isDisabled(), 'engine removal blocks an already-open form');
    engineFailure = true;
    await dialog.getByRole('button', { name: 'Muat ulang pilihan mesin' }).click();
    await dialog.getByRole('alert').filter({ hasText: 'Pemeriksaan mesin fixture gagal.' }).waitFor();
    check(await dialog.getByRole('radio').count() === 0, 'failed verification never falls back to hardcoded choices');
    engineFailure = false;
    engines = [{ engine: 'mariadb', label: 'MariaDB', port: 3306 }, { engine: 'postgresql', label: 'PostgreSQL', port: 5432 }];
    await dialog.getByRole('button', { name: 'Muat ulang pilihan mesin' }).click();
    await dialog.getByRole('radio', { name: 'MariaDB Port 3306' }).waitFor();
    check(await dialog.getByRole('radio').count() === 2, 'administrator installation appears after metadata refresh without a frontend engine branch');
    await page.setViewportSize({ width: 390, height: 740 });
    check(await dialog.evaluate(el => el.scrollWidth <= el.clientWidth + 1), 'live engine choices fit the mobile dialog');
    await page.screenshot({ path: path.join(output, 'live-engine-choices-mobile.png') });
    check(await dialog.evaluate(el => el.contains(document.activeElement)), 'engine refresh preserves keyboard focus inside the modal');
    await page.keyboard.press('Escape'); await dialog.waitFor({ state: 'detached' });
    engines = [];
    await page.getByTitle('Muat Ulang', { exact: true }).click();
    await page.getByText(/Belum ada mesin basis data yang siap digunakan/).waitFor();
    check(await create.isDisabled(), 'no installed engine blocks creation while retaining database management');
    engines = [{ engine: 'mariadb', label: 'MariaDB', port: 3306 }];
    await page.goto(`${base}/sites/1/database`, { waitUntil: 'domcontentloaded' });
    await page.waitForURL(url => url.pathname === `/sites/${domain}/database`);
    await page.getByRole('heading', { name: 'Belum ada basis data', exact: true }).waitFor();
    check(true, 'old numeric project link opens the repaired database tab');
    check(errors.length === 0, `no uncaught browser errors: ${errors.join('; ')}`);
    console.log(`PASS: ${checks} browser checks`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
