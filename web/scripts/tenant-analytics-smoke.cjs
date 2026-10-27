/* Static frontend and API fixtures only. Never starts panel-api or panel-agent. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
const output = path.join(__dirname, '../artifacts/tenant-analytics');
let checks = 0;
function check(value, message) { assert.ok(value, message); checks++; console.log(`ok ${checks}: ${message}`); }
const sites = [7, 8].map(id => ({ id, user_id: 42, kind: 'php', primary_domain: `tenant-${id}.example.test`, docroot: `/home/p_customer/site-${id}`, runtime: 'php:8.4', status: 'active', domains: [] }));
async function fixture(browser, { authenticated = true, empty = false } = {}) {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  page.setDefaultTimeout(15000);
  const calls = [], errors = [], unexpected = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/v1/**', async route => {
    const req = route.request(), url = new URL(req.url()), pathname = url.pathname;
    calls.push(pathname);
    let body = {}, status = 200;
    const user = { id: 42, username: 'customer', role: 'user' };
    const site = pathname.match(/^\/api\/v1\/sites\/(7|8)\/(analytics\/([^/]+)|git\/deployments)$/);
    const siteId = site ? Number(site[1]) : null;
    const from_ts = Number(url.searchParams.get('from') || 1700000000), to_ts = Number(url.searchParams.get('to') || 1700003600);
    if (pathname === '/api/v1/auth/me') {
      status = authenticated ? 200 : 401;
      body = authenticated ? { user, csrf_token: 'fixture' } : { error: 'Unauthenticated' };
    } else if (pathname === '/api/v1/auth/login') {
      authenticated = true;
      body = { user, csrf_token: 'fixture' };
    } else if (pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (pathname === '/api/v1/sites') body = empty ? [] : sites;
    else if (site && !empty) {
      const common = { site_id: siteId, from_ts, to_ts };
      if (site[3] === 'overview') body = { ...common, interval_used: '1h', current: { total_requests: siteId === 7 ? 731 : 842, total_bytes: 4096, unique_visitors: 41, status_2xx: 700, status_3xx: 20, status_4xx: 10, status_5xx: 1, cache_hit_ratio: 0.75, latency_avg_ms: 10, latency_p50_ms: 8, latency_p95_ms: 20, latency_p99_ms: 25, error_rate_percent: 0.1 }, previous: null, comparison: null };
      else if (site[3] === 'timeseries') body = { ...common, interval: '1h', points: [] };
      else if (site[3] === 'breakdowns') body = { ...common, paths: [], referrers: [], countries: [], browsers: [], os: [], devices: [], slow_urls: [], error_urls: [] };
      else if (site[3] === 'app-metrics') body = { ...common, runtime: 'php', php_fpm: null, app_process: null };
      else if (site[3] === 'slowlog') body = { site_id: siteId, events: [] };
      else if (site[3] === 'insights') body = { site_id: siteId, evaluated_at: to_ts, insights: [] };
      else if (site[2] === 'git/deployments') body = [];
      else { unexpected.push(pathname); status = 404; }
    } else if (pathname.includes('notifications')) body = { notifications: [], total: 0, count: 0, unread_count: 0 };
    else if (pathname === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 1 };
    else if (pathname === '/api/v1/system/info') body = { hostname: 'fixture', os_name: 'Debian', os_version: '12', uptime_secs: 60, agent_connected: true, panel_version: '0.1.1' };
    else if (pathname.includes('timeseries')) body = { points: [] };
    else { unexpected.push(pathname); status = 404; body = { error: 'No fixture for route' }; }
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  return { page, calls, errors, unexpected };
}
async function analytics(f, requests = '731') {
  await f.page.getByRole('heading', { name: 'Observabilitas & Analitik', exact: true }).waitFor();
  try { await f.page.getByText(requests, { exact: true }).waitFor(); }
  catch (error) {
    console.error(JSON.stringify({ calls: f.calls, errors: f.errors, unexpected: f.unexpected, text: await f.page.locator('#root').innerText() }, null, 2));
    throw error;
  }
  check(await f.page.getByText('Anda tidak memiliki akses ke halaman ini.', { exact: false }).count() === 0, 'tenant opens analytics without access denial');
  check(await f.page.getByRole('button', { name: 'Server Host (Admin)', exact: true }).count() === 0, 'host tab stays hidden for tenant');
  check(!f.calls.some(p => p.startsWith('/api/v1/system/analytics/')), 'tenant never fetches host metrics, global insights or retention');
}
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || 'msedge' });
  try {
    const tenant = await fixture(browser);
    await tenant.page.goto(base + '/dashboard', { waitUntil: 'domcontentloaded' });
    await tenant.page.getByRole('heading', { name: 'Beranda', exact: true }).waitFor();
    await tenant.page.locator('a.nav-link[href="/analytics"]').click();
    await analytics(tenant);
    check(new URL(tenant.page.url()).pathname === '/analytics', 'sidebar opens analytics on the first click');
    check(await tenant.page.locator('a.nav-link[href="/analytics"][aria-current=page]').count() === 1, 'analytics is highlighted in the sidebar');
    const picker = tenant.page.locator('select').first();
    check((await picker.locator('option').allTextContents()).join('|') === 'tenant-7.example.test (PHP)|tenant-8.example.test (PHP)', 'site picker contains the sites returned for this tenant');
    await picker.selectOption('8');
    await analytics(tenant, '842');
    check(tenant.calls.some(p => p === '/api/v1/sites/8/analytics/overview'), 'switching sites loads the selected site analytics');
    check(tenant.calls.filter(p => /\/sites\/\d+\//.test(p)).every(p => /^\/api\/v1\/sites\/(7|8)\//.test(p)), 'site requests use only tenant site IDs from the list');
    await tenant.page.goto(base + '/analytics', { waitUntil: 'domcontentloaded' });
    await analytics(tenant);
    await tenant.page.reload({ waitUntil: 'domcontentloaded' });
    await analytics(tenant);
    await tenant.page.screenshot({ path: path.join(output, 'tenant-desktop.png'), fullPage: true });
    await tenant.page.setViewportSize({ width: 390, height: 844 });
    await tenant.page.screenshot({ path: path.join(output, 'tenant-mobile.png'), fullPage: true });
    check(await tenant.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'tenant analytics fits the mobile viewport');
    check(tenant.errors.length === 0 && tenant.unexpected.length === 0, 'menu, direct URL, refresh and site switch have no browser or fixture errors');
    await tenant.page.close();

    const login = await fixture(browser, { authenticated: false });
    await login.page.goto(base + '/analytics?range=1h', { waitUntil: 'domcontentloaded' });
    await login.page.locator('#login-username').fill('customer');
    check(new URL(login.page.url()).searchParams.get('returnTo') === '/analytics?range=1h', 'anonymous analytics deep link is preserved');
    await login.page.locator('#login-password').fill('fixture-password');
    await login.page.locator('button[type=submit]').click();
    await analytics(login);
    check(new URL(login.page.url()).pathname + new URL(login.page.url()).search === '/analytics?range=1h', 'tenant login returns to analytics with its query');
    check(login.errors.length === 0 && login.unexpected.length === 0, 'analytics login flow has no browser or fixture errors');
    await login.page.close();

    const empty = await fixture(browser, { empty: true });
    await empty.page.goto(base + '/analytics', { waitUntil: 'domcontentloaded' });
    await empty.page.getByText('Belum ada situs terdaftar', { exact: true }).waitFor();
    check(await empty.page.getByText('Pilih situs untuk melihat analitik dan metrik performa.', { exact: true }).isVisible(), 'tenant without sites sees a useful empty state');
    check(!empty.calls.some(p => p.includes('/analytics/')), 'empty tenant does not request site or server analytics');
    check(empty.errors.length === 0 && empty.unexpected.length === 0, 'empty tenant view has no browser or fixture errors');
    await empty.page.close();
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
