/* Static frontend and API fixtures only. Never starts panel-api or panel-agent. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
const output = path.join(__dirname, '../artifacts/command-palette');
let checks = 0;
function check(value, message) { assert.ok(value, message); checks++; console.log(`ok ${checks}: ${message}`); }
async function capture(page, name) {
  await page.getByRole('dialog').evaluate(async el => { await Promise.all(el.getAnimations({ subtree: true }).filter(a => a.effect?.getTiming().iterations !== Infinity).map(a => a.finished.catch(() => {}))); });
  await page.screenshot({ path: path.join(output, name) });
}
async function fixture(browser, role = 'user') {
  const page = await browser.newPage({ viewport: { width: 1366, height: 900 } });
  page.setDefaultTimeout(10000);
  const errors = [], calls = [];
  let authenticated = true;
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/v1/**', async route => {
    const req = route.request(), url = new URL(req.url());
    calls.push({ path: url.pathname, method: req.method() });
    let body = {}, status = 200;
    if (url.pathname === '/api/v1/auth/me') { status = authenticated ? 200 : 401; body = authenticated ? { user: { id: role === 'admin' ? 1 : 42, username: role === 'admin' ? 'admin' : 'customer', role }, csrf_token: 'fixture' } : { error: 'Unauthenticated' }; }
    else if (url.pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (url.pathname === '/api/v1/auth/logout') { authenticated = false; body = { message: 'Logged out' }; }
    else if (url.pathname === '/api/v1/system/info') body = { hostname: 'fixture', os_name: 'Debian', os_version: '12', uptime_secs: 60, agent_connected: true, panel_version: '0.1.1' };
    else if (url.pathname === '/api/v1/system/metrics') body = { state: 'waiting', sample: null, sampling_interval_secs: 10 };
    else if (url.pathname.includes('timeseries')) body = { points: [] };
    else if (url.pathname === '/api/v1/sites') body = [];
    else if (url.pathname === '/api/v1/plans') body = { plans: [] };
    else if (url.pathname === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 1 };
    else if (url.pathname.includes('notifications')) body = { notifications: [], count: 0, unread_count: 0, total: 0 };
    else { status = 404; body = { error: 'No fixture' }; }
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  return { page, errors, calls };
}
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || 'msedge' });
  try {
    const f = await fixture(browser);
    const page = f.page;
    await page.goto(base + '/dashboard', { waitUntil: 'domcontentloaded' });
    await page.getByRole('heading', { name: 'Beranda', exact: true }).waitFor();
    const trigger = page.locator('button.workspace-search');
    await trigger.click();
    if (process.argv.includes('--before')) {
      await page.getByRole('dialog').waitFor();
      await capture(page, 'before.png');
      return;
    }
    const dialog = page.getByRole('dialog', { name: 'Pencarian cepat', exact: true });
    const input = dialog.getByRole('combobox');
    const options = dialog.getByRole('option');
    const open = async () => { await trigger.click(); await input.waitFor(); };
    check(await input.evaluate(el => el === document.activeElement), 'opening from navbar focuses the search input');
    check(await page.locator('#root').evaluate(el => el.inert), 'background is inert while search is open');
    check(await dialog.getByText('Halaman ini', { exact: true }).isVisible(), 'current page has a discreet indicator');
    check(await options.count() === 12, 'tenant sees seven navigation items and five preference/account commands');
    check(await dialog.locator('[cmdk-group-heading]').allTextContents().then(text => text.join('|') === 'Navigasi|Preferensi|Akun'), 'tenant groups separate navigation, preferences and account');
    check(await dialog.locator('[cmdk-item][data-value="page-analytics"]').count() === 1 && await dialog.locator('[cmdk-item][data-value="page-runtime"]').count() === 0, 'tenant sees analytics and no admin-only commands');
    await capture(page, 'desktop-light.png');
    await input.press('Tab');
    check(await dialog.getByRole('button', { name: 'Tutup pencarian', exact: true }).evaluate(el => el === document.activeElement), 'Tab stays inside the dialog');
    await page.keyboard.press('Shift+Tab');
    check(await input.evaluate(el => el === document.activeElement), 'Shift+Tab returns to search');
    await input.fill('analitik');
    await page.waitForFunction(() => document.querySelector('[cmdk-item][data-selected=true]')?.getAttribute('data-value') === 'page-analytics');
    check(await options.count() < 12 && await options.first().innerText().then(text => text.includes('Analitik')), 'search ranks the matching Indonesian label first');
    await input.press('Enter');
    await page.waitForURL(url => url.pathname === '/analytics');
    check(await page.getByRole('dialog').count() === 0, 'Enter navigates to the selected page and closes search');
    await page.getByRole('heading', { name: 'Observabilitas & Analitik', exact: true }).waitFor();
    await open();
    check(await input.inputValue() === '', 'reopening clears the previous query');
    await input.fill('zzzz-nothing');
    await dialog.getByText('Tidak ada hasil', { exact: true }).waitFor();
    check(await options.count() === 0 && await dialog.getByText('Tidak ditemukan untuk “zzzz-nothing”. Coba nama menu lain.', { exact: true }).isVisible(), 'empty search explains the query and how to recover');
    await capture(page, 'empty.png');
    await dialog.locator('.command-reset').click();
    check(await input.inputValue() === '' && await input.evaluate(el => el === document.activeElement), 'clear button restores input focus and the full menu');
    await input.fill('lalu lintas');
    await page.waitForFunction(() => document.querySelector('[cmdk-item][data-selected=true]')?.getAttribute('data-value') === 'page-analytics');
    check(await options.first().innerText().then(text => text.includes('Analitik')), 'search also matches descriptive text');
    await input.press('Escape');
    check(await page.getByRole('dialog').count() === 0 && await trigger.evaluate(el => el === document.activeElement), 'Escape closes search and restores navbar focus');
    check(!await page.locator('#root').evaluate(el => el.inert), 'closing restores background interaction');
    await page.keyboard.press('Control+k'); await input.waitFor();
    await input.press('ArrowUp');
    check(await dialog.locator('[cmdk-item][data-selected=true]').getAttribute('data-value') === 'logout', 'up arrow wraps from the first result to the last');
    await input.press('ArrowDown');
    check(await dialog.locator('[cmdk-item][data-selected=true]').getAttribute('data-value') === 'page-dashboard', 'down arrow wraps back to the first result');
    await input.press('ArrowDown');
    check(await dialog.locator('[cmdk-item][data-selected=true]').getAttribute('data-value') === 'page-sites', 'arrow keys change the selected result');
    await input.press('ArrowUp');
    check(await dialog.locator('[cmdk-item][data-selected=true]').getAttribute('data-value') === 'page-dashboard', 'up arrow restores the previous result');
    await page.keyboard.press('Control+k');
    check(await page.getByRole('dialog').count() === 0, 'Ctrl K toggles search closed');
    await page.keyboard.press('Meta+k'); await input.waitFor();
    check(true, 'Meta K also opens search');
    await dialog.getByRole('button', { name: 'Tutup pencarian', exact: true }).click();
    await open();
    await input.fill('gelap');
    await page.waitForFunction(() => document.querySelector('[cmdk-item][data-selected=true]')?.getAttribute('data-value') === 'theme-dark').catch(async error => { console.error(await dialog.innerText()); throw error; });
    await input.press('Enter');
    await page.waitForFunction(() => document.documentElement.classList.contains('dark'));
    await open();
    check(await dialog.locator('[cmdk-item][data-value="theme-dark"] .command-active').count() === 1, 'theme action applies dark mode and marks the active preference');
    await capture(page, 'desktop-dark.png');
    await page.mouse.click(5, 5);
    check(await page.getByRole('dialog').count() === 0, 'clicking the backdrop dismisses search');
    await open();
    await input.fill('bahasa');
    await page.waitForFunction(() => document.querySelector('[cmdk-item][data-selected=true]')?.getAttribute('data-value') === 'language');
    await input.press('Enter');
    await page.waitForFunction(() => document.documentElement.lang === 'en');
    await trigger.click();
    await page.getByRole('dialog', { name: 'Quick search', exact: true }).waitFor();
    check(await page.getByRole('heading', { name: 'Quick search', exact: true }).isVisible(), 'language action translates the entire dialog');
    await page.getByRole('combobox').fill('language');
    await page.waitForFunction(() => document.querySelector('[cmdk-item][data-selected=true]')?.getAttribute('data-value') === 'language');
    await page.getByRole('combobox').press('Enter');
    await page.waitForFunction(() => document.documentElement.lang === 'id');
    await page.setViewportSize({ width: 390, height: 844 }); await open();
    const bounds = await dialog.locator('.command-palette').boundingBox();
    check(bounds.x >= 12 && bounds.x + bounds.width <= 390 && bounds.y + bounds.height <= 844, 'mobile dialog fits inside viewport margins');
    check(await dialog.locator('.command-results').evaluate(el => el.scrollHeight > el.clientHeight), 'long result lists scroll inside the modal');
    await capture(page, 'mobile-dark.png');
    await dialog.locator('.command-results').evaluate(el => { el.scrollTop = el.scrollHeight; });
    await dialog.locator('[cmdk-item][data-value="theme-light"]').click();
    await open();
    await capture(page, 'mobile-light.png');
    await page.setViewportSize({ width: 320, height: 568 });
    const enlargedText = await page.addStyleTag({ content: '.command-caption h2 { font-size: 36px; } .command-caption p { font-size: 26px; } .command-search-row input { font-size: 32px; } .command-item-title { font-size: 28px; } .command-item-description { font-size: 24px; } .command-footer > span { font-size: 22px; }' });
    check(await dialog.locator('.command-palette').evaluate(el => el.getBoundingClientRect().right <= innerWidth && el.scrollWidth <= el.clientWidth), '320px viewport with 200 percent text size has no horizontal overflow');
    check(await dialog.locator('.command-footer').evaluate(el => el.getBoundingClientRect().bottom <= innerHeight), '200 percent text size keeps keyboard hints inside the viewport');
    await capture(page, 'mobile-zoom.png');
    await enlargedText.evaluate(el => el.remove());
    await page.setViewportSize({ width: 568, height: 320 });
    check(await dialog.locator('.command-footer').evaluate(el => el.getBoundingClientRect().bottom <= innerHeight), 'short viewport keeps the footer and close controls available');
    await input.fill('zz'.repeat(80)); await dialog.getByText('Tidak ada hasil', { exact: true }).waitFor();
    check(await dialog.locator('.command-palette').evaluate(el => el.scrollWidth <= el.clientWidth), 'long empty-state queries do not cause horizontal overflow');
    await input.press('Escape');
    await page.emulateMedia({ reducedMotion: 'reduce' }); await open();
    check(await dialog.locator('.command-palette').evaluate(el => getComputedStyle(el).animationName === 'none'), 'reduced motion disables the dialog entrance animation');
    await input.fill('keluar');
    await page.waitForFunction(() => document.querySelector('[cmdk-item][data-selected=true]')?.getAttribute('data-value') === 'logout');
    await input.press('Enter');
    await page.locator('#login-username').waitFor();
    check(f.calls.filter(call => call.path === '/api/v1/auth/logout').length === 1, 'logout action runs once and returns to login');
    check(f.errors.length === 0, f.errors.join('\n') || 'tenant interactions produce no browser errors');
    await page.close();

    const admin = await fixture(browser, 'admin');
    await admin.page.goto(base + '/dashboard', { waitUntil: 'domcontentloaded' });
    await admin.page.locator('button.workspace-search').click();
    const adminDialog = admin.page.getByRole('dialog');
    check(await adminDialog.getByRole('option').count() === 17, 'admin receives the complete navigation catalog');
    await capture(admin.page, 'admin.png');
    await adminDialog.getByRole('combobox').fill('paket');
    await admin.page.waitForFunction(() => document.querySelector('[cmdk-item][data-selected=true]')?.getAttribute('data-value') === 'page-plans');
    await Promise.all([
      admin.page.waitForResponse(response => new URL(response.url()).pathname === '/api/v1/plans'),
      adminDialog.getByRole('combobox').press('Enter'),
    ]);
    await admin.page.waitForURL(url => url.pathname === '/plans');
    check(admin.calls.some(call => call.path === '/api/v1/plans'), 'admin command opens hosting plans');
    check(admin.errors.length === 0, 'admin search and navigation produce no browser errors');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
