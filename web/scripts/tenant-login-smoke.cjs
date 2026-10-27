/* Static frontend with API fixtures. Never starts panel-api or panel-agent. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
const output = path.join(__dirname, '../artifacts/tenant-login');
let checks = 0;
function check(value, message) { assert.ok(value, message); checks++; console.log(`ok ${checks}: ${message}`); }
async function fixture(browser, { authenticated = false, role = 'user', twoFactor = false } = {}) {
  const page = await browser.newPage();
  const calls = [], errors = [], deniedFrames = [];
  const mode = { authenticated, role, twoFactor };
  page.on('pageerror', error => errors.push(error.message));
  await page.exposeFunction('recordDenied', text => deniedFrames.push(text));
  await page.addInitScript(() => {
    new MutationObserver(() => {
      const alert = [...document.querySelectorAll('[role=alert]')].find(el => el.textContent.includes('Anda tidak memiliki akses'));
      if (alert) window.recordDenied(alert.textContent);
    }).observe(document, { childList: true, subtree: true });
  });
  await page.route('**/api/v1/**', async route => {
    const req = route.request(), url = new URL(req.url());
    calls.push({ pathname: url.pathname, method: req.method(), role: mode.role });
    const user = { id: mode.role === 'admin' ? 1 : 42, username: mode.role === 'admin' ? 'admin' : 'customer', role: mode.role };
    let body = {}, status = 200;
    if (url.pathname === '/api/v1/auth/me') {
      status = mode.authenticated ? 200 : 401;
      body = mode.authenticated ? { user, csrf_token: 'fixture' } : { error: 'Unauthenticated' };
    } else if (url.pathname === '/api/v1/auth/login') {
      const payload = req.postDataJSON();
      mode.role = payload.username === 'admin' ? 'admin' : 'user';
      if (mode.twoFactor && !payload.code) body = { requires_2fa: true };
      else {
        mode.authenticated = true;
        body = { user: { id: mode.role === 'admin' ? 1 : 42, username: mode.role === 'admin' ? 'admin' : 'customer', role: mode.role }, csrf_token: 'fixture' };
      }
    } else if (url.pathname === '/api/v1/auth/logout') { mode.authenticated = false; body = { message: 'Logged out' }; }
    else if (url.pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (url.pathname === '/api/v1/system/info') body = { hostname: 'fixture-host', os_name: 'Debian', os_version: '12', uptime_secs: 60, agent_connected: true, panel_version: '0.1.1' };
    else if (url.pathname === '/api/v1/system/metrics') body = { state: 'waiting', sample: null, sampling_interval_secs: 10 };
    else if (url.pathname.includes('timeseries')) body = { points: [] };
    else if (url.pathname === '/api/v1/sites') body = [];
    else if (url.pathname === '/api/v1/plans') body = { plans: [] };
    else if (url.pathname === '/api/v1/php/versions') body = { versions: [], ppa_configured: false };
    else if (url.pathname === '/api/v1/runtimes/node/versions') body = { versions: [], installed_count: 0 };
    else if (url.pathname.includes('notifications')) body = { notifications: [], total: 0, count: 0, unread_count: 0 };
    else if (url.pathname === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 1 };
    else { status = 404; body = { error: 'No fixture for route' }; }
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  return { page, calls, errors, deniedFrames, mode };
}
async function login(page, username = 'customer') {
  await page.locator('#login-username').fill(username);
  await page.locator('#login-password').fill('fixture-password');
  await page.locator('button[type=submit]').click();
}
async function dashboard(f) {
  await f.page.waitForURL(url => url.pathname === '/dashboard');
  await f.page.getByRole('heading', { name: 'Beranda', exact: true }).waitFor();
  check(await f.page.getByText('Anda tidak memiliki akses ke halaman ini.', { exact: false }).count() === 0, 'tenant sees dashboard immediately');
  check(f.deniedFrames.length === 0, 'post-login route never flashes an access-denied page');
}
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || 'msedge' });
  try {
    if (process.argv.includes('--reproduce')) {
      const f = await fixture(browser);
      await f.page.goto(base + '/login?returnTo=%2Fruntime', { waitUntil: 'domcontentloaded' });
      await login(f.page);
      await f.page.getByRole('alert').filter({ hasText: 'Anda tidak memiliki akses' }).waitFor();
      const active = await f.page.locator('a[aria-current=page]').allTextContents();
      await fs.writeFile(path.join(output, 'before.json'), JSON.stringify({ url: f.page.url(), active, deniedFrames: f.deniedFrames }, null, 2));
      check(active.some(label => label.trim() === 'Beranda'), 'reproduced: forbidden returnTo is presented as active Dashboard');
      return;
    }
    for (const target of ['/runtime', '/plans?sort=name', '/tenants/42', '/audit', '/settings']) {
      const f = await fixture(browser);
      await f.page.goto(base + '/login?returnTo=' + encodeURIComponent(target), { waitUntil: 'domcontentloaded' });
      await login(f.page);
      await dashboard(f);
      check(!f.calls.some(call => call.pathname === '/api/v1/php/versions' || call.pathname === '/api/v1/plans' || call.pathname === '/api/v1/system/metrics'), 'tenant login does not fetch admin page data');
      check(f.errors.length === 0, 'tenant login has no browser errors');
      await f.page.close();
    }
    const fresh = await fixture(browser);
    await fresh.page.goto(base, { waitUntil: 'domcontentloaded' });
    await login(fresh.page);
    await dashboard(fresh);
    await fresh.page.reload({ waitUntil: 'domcontentloaded' });
    await fresh.page.getByRole('heading', { name: 'Beranda', exact: true }).waitFor();
    check(true, 'tenant dashboard survives refresh');
    await fresh.page.close();

    const switching = await fixture(browser, { authenticated: true, role: 'admin' });
    await switching.page.goto(base + '/plans', { waitUntil: 'domcontentloaded' });
    await switching.page.getByRole('heading', { name: 'Paket Hosting (Hosting Plans)' }).waitFor();
    await switching.page.getByRole('button', { name: 'Keluar', exact: true }).click();
    await switching.page.locator('#login-username').waitFor();
    check(!new URL(switching.page.url()).searchParams.has('returnTo'), 'explicit logout clears the previous account returnTo');
    await login(switching.page);
    await dashboard(switching);
    await switching.page.close();

    const deep = await fixture(browser);
    await deep.page.goto(base + '/sites?status=active', { waitUntil: 'domcontentloaded' });
    await deep.page.locator('#login-username').waitFor();
    check(new URL(deep.page.url()).searchParams.get('returnTo') === '/sites?status=active', 'anonymous tenant deep link is preserved before login');
    await login(deep.page);
    await deep.page.waitForURL(url => url.pathname === '/sites' && url.search === '?status=active');
    check(deep.deniedFrames.length === 0, 'permitted tenant returnTo and query are restored');
    await deep.page.close();

    const denied = await fixture(browser, { authenticated: true });
    await denied.page.goto(base + '/runtime', { waitUntil: 'domcontentloaded' });
    await denied.page.getByRole('alert').filter({ hasText: 'Anda tidak memiliki akses' }).waitFor();
    check(await denied.page.locator('a[href="/dashboard"][aria-current=page]').count() === 0, 'direct forbidden route is not highlighted as Dashboard');
    check(!denied.calls.some(call => call.pathname === '/api/v1/php/versions'), 'direct tenant access to admin runtime stays blocked');
    await denied.page.close();

    const admin = await fixture(browser, { role: 'admin' });
    await admin.page.goto(base + '/login?returnTo=%2Fplans', { waitUntil: 'domcontentloaded' });
    await login(admin.page, 'admin');
    await admin.page.waitForURL(url => url.pathname === '/plans');
    await admin.page.getByRole('heading', { name: 'Paket Hosting (Hosting Plans)' }).waitFor();
    check(true, 'admin returnTo is preserved');
    await admin.page.close();

    const twoFactor = await fixture(browser, { twoFactor: true });
    await twoFactor.page.goto(base + '/login?returnTo=%2Fruntime', { waitUntil: 'domcontentloaded' });
    await login(twoFactor.page);
    await twoFactor.page.locator('#login-code').fill('123456');
    check(new URL(twoFactor.page.url()).pathname === '/login', '2FA challenge does not navigate before verification');
    await twoFactor.page.locator('button[type=submit]').click();
    await dashboard(twoFactor);
    await twoFactor.page.setViewportSize({ width: 390, height: 844 });
    await twoFactor.page.screenshot({ path: path.join(output, 'tenant-dashboard-mobile.png'), fullPage: true });
    check(await twoFactor.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'tenant dashboard fits a mobile viewport');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
