/* Production UI with intercepted fixtures only; no panel daemon, SSH or real credentials. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5183';
const output = path.resolve(__dirname, '../artifacts/account-access');
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
const tenant = { id: 42, username: 'pelanggan', status: 'active', linux_username: 'p_pelanggan', linux_uid: 10042, plan_id: null, created_at: 1 };

async function fixture(browser, width, theme, role = 'admin') {
  const page = await browser.newPage({ viewport: { width, height: 900 }, reducedMotion: 'reduce' });
  page.setDefaultTimeout(12000);
  await page.addInitScript(value => localStorage.setItem('nusa_theme', value), theme);
  const errors = [], calls = [];
  const mode = { tenants: [], tenantFail: false, statusFail: false, setupFail: false, enabled: false, policy: 'none' };
  let user = { id: role === 'admin' ? 1 : 42, username: role === 'admin' ? 'admin' : 'pelanggan', role, email: 'akun@example.test', totp_enabled: false };
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), name = new URL(request.url()).pathname;
    calls.push({ name, method: request.method(), payload: request.postDataJSON() });
    let status = 200, body = {};
    if (name === '/api/v1/auth/me') body = { user, csrf_token: 'fixture' };
    else if (name === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (name === '/api/v1/tenants') {
      if (mode.tenantFail) { status = 503; body = { error: 'Layanan tenant fixture tidak tersedia.' }; }
      else body = { tenants: mode.tenants };
    } else if (name.endsWith('/shell-policy')) body = { user_id: 42, username: 'pelanggan', effective_policy: mode.policy, inherited_from_plan: true, plan_policy: mode.policy };
    else if (name === '/api/v1/terminal/sessions') { status = 400; body = { error: 'Sesi fixture tidak membuka PTY.' }; }
    else if (name === '/api/v1/auth/2fa/status') {
      if (mode.statusFail) { status = 503; body = { error: 'Status fixture tidak tersedia.' }; }
      else body = { enabled: mode.enabled, remaining_recovery_codes: mode.enabled ? 8 : 0 };
    } else if (name === '/api/v1/auth/2fa/setup') {
      if (mode.setupFail) { status = 503; body = { error: 'Persiapan autentikasi fixture gagal.' }; }
      else body = { secret: 'FIXTUREKEYABCDEFGHIJKLMNOPQRSTUVWXYZ', qr_code_base64: 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jM1sAAAAASUVORK5CYII=' };
    } else if (name === '/api/v1/auth/2fa/enable') {
      mode.enabled = true; user.totp_enabled = true;
      body = { recovery_codes: Array.from({ length: 8 }, (_, index) => `fixture-${index}-code`) };
    } else if (name === '/api/v1/auth/2fa/disable') { mode.enabled = false; user.totp_enabled = false; body = { success: true }; }
    else if (name === '/api/v1/auth/password') {
      const payload = request.postDataJSON();
      if (payload.current_password !== 'FixtureCurrent123!') { status = 400; body = { error: 'Kata sandi fixture tidak cocok.' }; }
      else { user = { ...user, username: payload.username || user.username }; body = { user, csrf_token: 'fixture', requires_2fa: false }; }
    } else if (name === '/api/v1/ssh-keys' || name === '/api/v1/sites') body = [];
    else if (name.includes('unread-count')) body = { count: 0, unread_count: 0 };
    else if (name.includes('notifications')) body = { notifications: [], total: 0 };
    else if (name === '/api/v1/jobs') body = { items: [], total: 0 };
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  return { page, mode, calls, errors };
}

async function fits(page, label) {
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${label}: no horizontal overflow`);
  check(await page.locator('#panel-main').evaluate(el => el.scrollWidth <= el.clientWidth + 1), `${label}: content fits its container`);
}

(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || 'msedge' });
  try {
    for (const [width, theme] of [[1440, 'light'], [1280, 'dark'], [390, 'light'], [320, 'dark']]) {
      const { page, mode, calls, errors } = await fixture(browser, width, theme);
      await page.goto(`${base}/terminal`, { waitUntil: 'domcontentloaded' });
      await page.getByRole('heading', { name: 'Belum ada tenant yang dapat dikelola' }).waitFor();
      check(await page.getByRole('heading', { name: 'Terminal', exact: true }).count() === 1, `${width}: terminal keeps a page title in the empty state`);
      check(await page.locator('.tenant-target-notice').getByRole('link', { name: 'Kelola tenant' }).getAttribute('href') === '/tenants', `${width}: empty state has a tenant management action`);
      check(!calls.some(call => call.name.includes('shell-policy') || call.name.includes('terminal/sessions')), `${width}: no target means no policy or session request`);
      await fits(page, `${width} terminal empty`);
      await page.screenshot({ path: path.join(output, `terminal-empty-${width}.png`), fullPage: true });

      mode.tenantFail = true;
      await page.reload({ waitUntil: 'domcontentloaded' });
      await page.getByRole('heading', { name: 'Daftar tenant belum dapat dimuat' }).waitFor();
      check(await page.getByRole('alert').filter({ hasText: 'Layanan tenant fixture' }).count() === 1, `${width}: tenant failure is distinct from empty data`);
      mode.tenantFail = false;
      mode.tenants = [tenant, { ...tenant, id: 43, username: 'ditangguhkan', status: 'suspended' }, { ...tenant, id: 44, username: 'belum_siap', linux_uid: null }];
      await page.getByRole('button', { name: 'Coba lagi', exact: true }).click();
      await page.getByRole('heading', { name: 'Pilih tenant untuk melanjutkan' }).waitFor();
      const selection = page.getByLabel('Akun tenant', { exact: true });
      check(await selection.locator('option').count() === 2, `${width}: only eligible tenants are offered`);
      await selection.selectOption('42');
      await page.getByRole('heading', { name: 'Terminal belum diaktifkan' }).waitFor();
      check(new URL(page.url()).searchParams.get('tenant') === '42', `${width}: explicit target persists in URL`);
      check(!calls.some(call => call.name.includes('terminal/sessions')), `${width}: blocked shell never requests a session`);
      await fits(page, `${width} selected terminal`);

      mode.statusFail = true;
      await page.goto(`${base}/profile`, { waitUntil: 'domcontentloaded' });
      await page.getByText('Status autentikasi dua faktor belum dapat dimuat.').waitFor();
      check(await page.getByRole('button', { name: 'Aktifkan 2FA', exact: true }).count() === 0, `${width}: unavailable 2FA status is not displayed as disabled`);
      mode.statusFail = false;
      await page.getByRole('button', { name: 'Coba lagi', exact: true }).click();
      await page.getByRole('button', { name: 'Aktifkan 2FA', exact: true }).waitFor();
      const form = page.getByRole('region', { name: 'Username dan kata sandi' });
      check(await form.getByLabel('Username login').inputValue() === 'admin', `${width}: administrator identity remains editable`);
      await fits(page, `${width} profile`);
      await page.locator('#panel-main').evaluate(element => { element.scrollTop = 0; });
      await page.screenshot({ path: path.join(output, `profile-${width}.png`), fullPage: true });

      mode.setupFail = true;
      await page.getByRole('button', { name: 'Aktifkan 2FA', exact: true }).click();
      await page.getByRole('alert').filter({ hasText: 'Persiapan autentikasi fixture gagal.' }).waitFor();
      check(true, `${width}: initial 2FA setup failure is visible and retryable`);
      mode.setupFail = false;
      await page.getByRole('button', { name: 'Aktifkan 2FA', exact: true }).click();
      await page.getByLabel('Masukkan 6 digit kode dari aplikasi:', { exact: true }).fill('123456');
      await fits(page, `${width} 2FA setup`);
      await page.getByRole('button', { name: 'Verifikasi & Aktifkan 2FA', exact: true }).click();
      await page.getByRole('button', { name: 'Saya sudah menyimpan kode ini' }).waitFor();
      check(await page.locator('.profile-recovery-codes > div').count() === 8, `${width}: recovery codes remain available`);
      await fits(page, `${width} recovery codes`);
      await page.screenshot({ path: path.join(output, `profile-recovery-${width}.png`), fullPage: true });
      await page.getByRole('button', { name: 'Saya sudah menyimpan kode ini' }).click();
      await page.getByRole('button', { name: 'Nonaktifkan 2FA', exact: true }).click();
      await page.getByLabel('Kode autentikasi atau pemulihan').fill('fixture-recovery');
      await page.getByRole('button', { name: 'Konfirmasi Nonaktifkan', exact: true }).click();
      await page.getByRole('button', { name: 'Aktifkan 2FA', exact: true }).waitFor();
      check(calls.some(call => call.name.endsWith('/2fa/disable') && call.payload.code === 'fixture-recovery'), `${width}: recovery-based 2FA disable keeps its request contract`);
      check(errors.length === 0, `${width}: no browser errors ${errors.join('; ')}`);
      await page.close();
    }
    const customer = await fixture(browser, 390, 'light', 'user');
    await customer.page.goto(`${base}/profile`, { waitUntil: 'domcontentloaded' });
    await customer.page.getByRole('button', { name: 'Aktifkan 2FA', exact: true }).waitFor();
    check(await customer.page.getByLabel('Username login').count() === 0, 'regular tenant has no administrator username control');
    check(await customer.page.getByText('Gunakan kata sandi baru dengan minimal 8 karakter.').count() === 1, 'tenant receives relevant password guidance');
    await fits(customer.page, 'tenant profile mobile');
    await customer.page.close();
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, note: 'Intercepted fixtures only; no live account, 2FA, SSH or PTY changes.' }, null, 2));
    console.log(`${checks} browser assertions passed`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
