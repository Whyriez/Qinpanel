/* API fixtures only; never starts panel daemons or executes jobs. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
let checks = 0;
function check(value, message) { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); }
async function fixture(browser, role = 'admin', width = 1280) {
  const page = await browser.newPage({ viewport: { width, height: 900 } });
  page.setDefaultTimeout(15000);
  const mode = { queued: 240, running: 2, fail: false, invalid: false };
  const calls = [], errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/v1/**', async route => {
    const url = new URL(route.request().url()), name = url.pathname;
    calls.push({ name, query: url.searchParams.toString() });
    let body = {}, status = 200;
    if (name === '/api/v1/auth/me') body = { user: { id: role === 'admin' ? 1 : 42, username: role, role }, csrf_token: 'fixture' };
    else if (name === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (name === '/api/v1/sites') body = [];
    else if (name === '/api/v1/jobs') {
      const filter = url.searchParams.get('status');
      if (filter && mode.fail) { status = 503; body = { error: 'Fixture count unavailable' }; }
      else body = { items: [], total: filter ? mode.invalid ? -1 : mode[filter] : 0, page: 1, total_pages: 0, per_page: 1 };
    } else if (name.includes('unread-count')) body = { count: 0, unread_count: 0 };
    else if (name.includes('notifications')) body = { notifications: [], total: 0 };
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  await page.goto(base + '/sites', { waitUntil: 'domcontentloaded' });
  return { page, mode, calls, errors, badge: page.locator('#panel-navigation .nav-job-count'), link: page.locator('#panel-navigation a[href="/jobs"]') };
}
async function refresh(f) {
  await f.page.evaluate(() => window.dispatchEvent(new Event('focus')));
}
async function waitText(locator, value) {
  await locator.filter({ hasText: new RegExp('^' + value.replace('+', '\\+') + '$') }).waitFor();
}
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    await fs.mkdir(path.join(__dirname, '../artifacts/sidebar-jobs'), { recursive: true });
    const f = await fixture(browser);
    await waitText(f.badge, '99+');
    check(await f.link.getAttribute('title') === '242 tugas aktif: 240 antre, 2 berjalan', 'badge uses totals across all pages, including jobs outside recent tray');
    check(await f.page.locator('#sidebar-active-jobs').textContent() === '242 tugas aktif: 240 antre, 2 berjalan', 'screen-reader description includes exact counts despite 99+ display');
    check(f.calls.filter(call => call.name === '/api/v1/jobs' && new URLSearchParams(call.query).get('status')).every(call => new URLSearchParams(call.query).get('per_page') === '1'), 'counter requests only one item per status');
    await f.page.getByRole('heading', { name: 'Situs', exact: true }).waitFor();
    await f.page.screenshot({ path: path.join(__dirname, '../artifacts/sidebar-jobs/desktop.png') });
    await f.page.locator('.desktop-collapse').click();
    check(await f.badge.isVisible(), 'count remains visible in collapsed sidebar');
    const geometry = await f.badge.evaluate(element => {
      const box = element.getBoundingClientRect(), sidebar = element.closest('aside').getBoundingClientRect();
      return box.left >= sidebar.left && box.right <= sidebar.right && box.top >= sidebar.top && box.bottom <= sidebar.bottom;
    });
    check(geometry, 'collapsed badge fits within sidebar');
    await f.page.screenshot({ path: path.join(__dirname, '../artifacts/sidebar-jobs/collapsed.png') });
    f.mode.queued = 3; f.mode.running = 1;
    await waitText(f.badge, '4');
    check(await f.link.getAttribute('title') === '4 tugas aktif: 3 antre, 1 berjalan', 'polling automatically updates active count');
    await f.page.locator('.desktop-collapse').click();
    await f.link.click();
    await f.page.waitForURL('**/jobs');
    check(await f.badge.textContent() === '4', 'badge survives navigation and link opens Jobs');
    f.mode.fail = true; await refresh(f);
    await f.badge.waitFor({ state: 'detached' });
    check(await f.link.getAttribute('aria-describedby') === null, 'failed count request removes stale number and description');
    f.mode.fail = false; f.mode.queued = 0; f.mode.running = 1; await refresh(f);
    await waitText(f.badge, '1');
    check(true, 'focus refresh recovers a failed count');
    f.mode.invalid = true; await refresh(f); await f.badge.waitFor({ state: 'detached' });
    check(true, 'invalid server totals are not displayed');
    f.mode.invalid = false; f.mode.queued = 0; f.mode.running = 0; await refresh(f);
    await f.page.waitForResponse(response => response.url().includes('/jobs?') && response.url().includes('status=running'));
    check(await f.badge.count() === 0, 'no badge is shown for an empty queue');
    f.mode.queued = 1; await refresh(f); await waitText(f.badge, '1');
    await f.page.locator('.desktop-collapse').click();
    await f.page.setViewportSize({ width: 390, height: 844 });
    await f.page.getByRole('banner').getByRole('button', { name: 'Buka navigasi', exact: true }).click();
    await f.badge.waitFor({ state: 'visible' });
    await f.page.locator('#panel-navigation').evaluate(element => Promise.all(element.getAnimations().map(animation => animation.finished)));
    check(await f.badge.evaluate(element => { const box = element.getBoundingClientRect(); return box.left >= 0 && box.right <= innerWidth; }), 'mobile badge fits after opening a previously collapsed sidebar');
    check(await f.badge.isVisible() && !await f.page.evaluate(() => document.documentElement.scrollWidth > innerWidth), 'mobile menu shows count without horizontal overflow');
    await f.page.screenshot({ path: path.join(__dirname, '../artifacts/sidebar-jobs/mobile.png') });
    await f.link.click();
    await f.page.waitForFunction(() => !document.querySelector('.panel-shell.is-mobile-open'));
    check(true, 'Jobs navigation closes the mobile menu normally');
    check(f.errors.length === 0, `admin has no browser exceptions (${f.errors.join(', ')})`);
    await f.page.close();
    const tenant = await fixture(browser, 'user');
    await tenant.link.waitFor();
    await refresh(tenant);
    await tenant.page.waitForTimeout(300);
    check(await tenant.badge.count() === 0, 'admin task badge is absent for tenants');
    check(!tenant.calls.some(call => call.name === '/api/v1/jobs' && new URLSearchParams(call.query).get('status')), 'tenant does not poll admin task totals');
    check(tenant.errors.length === 0, 'tenant navigation remains usable without browser exceptions');
    await tenant.page.close();
    console.log(`${checks} sidebar job counter checks passed (API fixtures only).`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
