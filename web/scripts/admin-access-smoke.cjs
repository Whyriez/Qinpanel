/* Static built UI and mocked API only. No panel daemon, server changes or real credentials. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const http = require('node:http');
const root = path.resolve(__dirname, '../dist');
const output = path.resolve(__dirname, '../artifacts/admin-access');
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };

(async () => {
  await fs.mkdir(output, { recursive: true });
  const server = http.createServer(async (req, res) => {
    try {
      const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
      const file = path.resolve(root, `.${pathname}`);
      if (!file.startsWith(root + path.sep) && file !== root) { res.writeHead(403); return res.end(); }
      const extension = path.extname(file);
      const target = extension ? file : path.join(root, 'index.html');
      const content = await fs.readFile(target);
      const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.woff2': 'font/woff2', '.svg': 'image/svg+xml' };
      res.writeHead(200, { 'content-type': types[path.extname(target)] || 'application/octet-stream' });
      res.end(content);
    } catch { res.writeHead(404); res.end(); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  let browser;
  try {
    browser = await chromium.launch({ channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge', headless: true });
    const page = await browser.newPage({ viewport: { width: 1366, height: 1000 }, reducedMotion: 'reduce' });
    page.setDefaultTimeout(10000);
    const errors = [], requests = [];
    page.on('pageerror', error => errors.push(error.message));
    let authenticated = false, csrf = 'fixture-initial';
    let user = { id: 1, username: 'admin_generated', role: 'admin', email: null, totp_enabled: false };
    await page.route('**/api/v1/**', async route => {
      const request = route.request(), pathname = new URL(request.url()).pathname;
      let status = 200, body = {};
      if (pathname === '/api/v1/auth/me') {
        status = authenticated ? 200 : 401;
        body = authenticated ? { user, csrf_token: csrf } : { error: 'Fixture session required' };
      } else if (pathname === '/api/v1/auth/password') {
        const payload = request.postDataJSON();
        requests.push({ payload, csrf: request.headers()['x-csrf-token'] });
        assert.equal(request.headers()['x-csrf-token'], csrf);
        if (payload.current_password !== 'FixtureOriginal123!') { status = 400; body = { error: 'Kata sandi saat ini tidak benar.' }; }
        else if (payload.username === 'taken') { status = 409; body = { error: 'Username sudah digunakan.' }; }
        else {
          user = { ...user, username: payload.username || user.username };
          csrf = `fixture-rotated-${requests.length}`;
          body = { user, csrf_token: csrf, requires_2fa: false };
        }
      } else if (pathname === '/api/v1/auth/2fa/status') body = { enabled: false, remaining_recovery_codes: 0 };
      else if (pathname === '/api/v1/auth/csrf') body = { csrf_token: csrf };
      else if (pathname === '/api/v1/ssh-keys' || pathname === '/api/v1/sites' || pathname === '/api/v1/users') body = [];
      else if (pathname.includes('notifications')) body = { notifications: [], count: 0, unread_count: 0, total: 0 };
      else { status = 404; body = { error: 'No fixture' }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    await page.goto(`${base}/login`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('button', { name: 'Bantuan akses admin' }).click();
    await page.getByRole('heading', { name: 'Akses administrator', exact: true }).waitFor();
    check(await page.locator('form').count() === 0, 'setup has SSH recovery instructions and no public token form');
    check((await page.locator('body').innerText()).includes('sudo panelctl admin reset-password --username NAMA'), 'recovery shows executable CLI commands');
    await page.setViewportSize({ width: 320, height: 740 });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'recovery page fits 320px');
    await page.screenshot({ path: path.join(output, 'recovery-mobile.png') });
    await page.getByRole('button', { name: 'Kembali ke login' }).click();
    await page.getByLabel('Nama pengguna', { exact: true }).waitFor();
    check(new URL(page.url()).pathname === '/login', 'recovery returns to login');

    authenticated = true;
    await page.setViewportSize({ width: 1366, height: 1000 });
    await page.goto(`${base}/profile`, { waitUntil: 'domcontentloaded' });
    const username = page.getByLabel('Username login', { exact: true });
    await username.waitFor();
    check(await username.inputValue() === 'admin_generated', 'profile loads the generated username');
    const current = page.locator('#account-current-password');
    const password = page.locator('#account-new-password');
    const confirmation = page.locator('#account-confirm-password');
    const save = page.locator('form').filter({ has: page.locator('#account-current-password') }).locator('button[type="submit"]');
    check(await current.inputValue() === '' && await password.inputValue() === '', 'password fields start empty');
    await username.fill('admin_renamed');
    await current.fill('wrong-password');
    await save.click();
    await page.getByRole('alert').filter({ hasText: 'Kata sandi saat ini tidak benar.' }).waitFor();
    check(await username.inputValue() === 'admin_renamed' && await current.inputValue() === 'wrong-password', 'failed reauthentication preserves form input');
    await current.fill('FixtureOriginal123!');
    await username.fill('taken');
    await save.click();
    await page.getByRole('alert').filter({ hasText: 'Username sudah digunakan.' }).waitFor();
    check(user.username === 'admin_generated', 'username conflict preserves the account');
    await username.fill('admin_renamed');
    await save.click();
    await page.getByRole('status').filter({ hasText: 'Kredensial diperbarui.' }).waitFor();
    check(requests.at(-1).payload.username === 'admin_renamed' && !('new_password' in requests.at(-1).payload), 'username-only change omits a new password');
    check(await current.inputValue() === '' && user.username === 'admin_renamed', 'successful save clears secrets and refreshes the username');
    await current.fill('FixtureOriginal123!');
    await password.fill('FixtureChanged456!');
    await confirmation.fill('Mismatch789!');
    const before = requests.length;
    await save.click();
    await page.getByRole('alert').filter({ hasText: 'Konfirmasi kata sandi tidak cocok.' }).waitFor();
    check(requests.length === before, 'password confirmation mismatch makes no API request');
    await confirmation.fill('FixtureChanged456!');
    await save.click();
    await page.getByRole('status').filter({ hasText: 'Kredensial diperbarui.' }).waitFor();
    check(requests.at(-1).payload.current_password === 'FixtureOriginal123!' && requests.at(-1).payload.new_password === 'FixtureChanged456!', 'combined save sends the API password contract');
    check(requests.at(-1).csrf.startsWith('fixture-rotated-'), 'next save adopts the rotated CSRF token');
    check(await password.inputValue() === '' && await confirmation.inputValue() === '', 'password and confirmation are cleared after success');
    await page.screenshot({ path: path.join(output, 'profile-desktop.png') });
    await page.setViewportSize({ width: 320, height: 740 });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'profile fits 320px');
    await page.screenshot({ path: path.join(output, 'profile-mobile.png') });
    user = { ...user, id: 2, role: 'user', username: 'tenant' };
    await page.reload({ waitUntil: 'domcontentloaded' });
    await current.waitFor();
    check(await page.locator('#account-username').count() === 0, 'tenant retains password changes without login or Linux identity rename');
    check(errors.length === 0, 'no browser runtime errors');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors, requests }, null, 2));
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
