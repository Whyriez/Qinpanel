/* Static frontend + intercepted API/404 fixtures. No real Nginx or panel daemons. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const fixture = require('./fixtures/site-rules.json');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5192';
const output = path.join(__dirname, '../artifacts/qpanel-brand');
const domain = 'testing.incogmsg.my.id';
const site = { id: 1, user_id: 2, primary_domain: domain, kind: 'static', docroot: `/home/p_alice/sites/${domain}/public`, status: 'active', spa_fallback: false, force_https: false, created_at: 1770000000, updated_at: 1770000000, domains: [] };
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
(async () => {
  await fs.mkdir(output, { recursive: true });
  const html = await fs.readFile(path.join(__dirname, '../public/errors/404.html'), 'utf8');
  const browser = await chromium.launch({ headless: true, channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge' });
  try {
    const boot = await browser.newPage({ javaScriptEnabled: false, viewport: { width: 320, height: 740 }, reducedMotion: 'reduce' });
    await boot.goto(base);
    check(await boot.getByRole('status').innerText().then(text => text.includes('Qinpanel')), 'initial HTML loading already uses Qinpanel before JavaScript starts');
    check(await boot.locator('.panel-loading__spinner').evaluate(el => getComputedStyle(el).animationName === 'none'), 'initial loading respects reduced motion');
    check(await boot.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'initial loading fits 320px');
    await boot.screenshot({ path: path.join(output, 'boot-mobile.png') });
    await boot.close();
    const loading = await browser.newPage({ viewport: { width: 390, height: 844 }, reducedMotion: 'reduce' });
    let releaseSession;
    const sessionGate = new Promise(resolve => { releaseSession = resolve; });
    await loading.route('**/api/v1/auth/me', async route => { await sessionGate; await route.fulfill({ status: 401, contentType: 'application/json', body: '{}' }); });
    await loading.goto(base, { waitUntil: 'domcontentloaded' });
    await loading.locator('.brand-loading--full').waitFor();
    check((await loading.getByRole('status').innerText()).includes('Qinpanel'), 'session loading shares the canonical logo and name');
    check(await loading.locator('.panel-loading__spinner').evaluate(el => getComputedStyle(el).animationName === 'none'), 'session loading respects reduced motion');
    await loading.screenshot({ path: path.join(output, 'session-loading-mobile.png') });
    releaseSession();
    await loading.locator('.auth-page').waitFor();
    await loading.close();
    for (const theme of ['light', 'dark']) {
      const identity = await browser.newPage({ viewport: { width: 320, height: 740 }, reducedMotion: 'reduce' });
      await identity.addInitScript(value => localStorage.setItem('nusa_theme', value), theme);
      await identity.route('**/api/v1/**', route => route.fulfill({ status: 401, contentType: 'application/json', body: '{}' }));
      await identity.goto(`${base}/login`, { waitUntil: 'domcontentloaded' });
      const mark = identity.locator('.panel-brand:visible .brand-mark').first();
      await mark.waitFor();
      check(await identity.locator('html').evaluate(el => el.classList.contains('dark')) === (theme === 'dark'), `${theme}: actual saved theme is active`);
      check(await mark.evaluate(async el => {
        await el.decode();
        const bounds = el.getBoundingClientRect();
        return el.naturalWidth > 0 && el.naturalWidth === el.naturalHeight && bounds.width > 0 && bounds.width === bounds.height;
      }), `${theme}: geometric logo renders at its square aspect ratio`);
      check(await mark.getAttribute('src') === '/brand/qinpanel-mark.svg?v=4' && await identity.locator('link[rel="icon"]').getAttribute('href') === '/favicon.svg?v=4', `${theme}: boot, panel and favicon use the new asset revision`);
      check(await identity.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${theme}: login identity fits 320px without overflow`);
      await identity.screenshot({ path: path.join(output, `login-mobile-${theme}.png`) });
      await identity.close();
    }
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce' });
    page.setDefaultTimeout(10000);
    let authenticated = false, saved;
    const errors = [], external = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('request', req => { if (new URL(req.url()).origin !== new URL(base).origin) external.push(req.url()); });
    await page.addInitScript(() => localStorage.setItem('nusa_theme', 'dark'));
    await page.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url());
      let status = 200, body = {};
      if (url.pathname === '/api/v1/auth/me') {
        if (!authenticated) { status = 401; body = { error: 'Fixture session required' }; }
        else body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      }
      else if (url.pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/sites') body = [site];
      else if (url.pathname === '/api/v1/sites/1') body = site;
      else if (url.pathname === '/api/v1/sites/1/ssl') body = { site_id: 1, has_ssl: false, certificate: null };
      else if (url.pathname === '/api/v1/sites/1/rules') {
        if (req.method() === 'PUT') { saved = req.postDataJSON(); body = saved; }
        else body = saved || { ...fixture, custom_error_pages: [] };
      }
      else if (url.pathname.includes('notifications')) body = { notifications: [], count: 0, unread_count: 0, total: 0 };
      else { status = 404; body = { error: 'No fixture' }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    await page.goto(`${base}/login`, { waitUntil: 'domcontentloaded' });
    await page.locator('.panel-brand:visible').first().waitFor();
    check((await page.title()).includes('Qinpanel'), 'browser title uses Qinpanel');
    check((await page.locator('.panel-brand:visible').first().innerText()).includes('Qinpanel'), 'login uses the Q logo and Qinpanel name');
    check(!await page.getByText(/Nusa Panel|Nusapanel|QonPanel/i).count(), 'login contains no former product name');
    await page.screenshot({ path: path.join(output, 'login.png') });
    const icon = await page.request.get(`${base}/favicon.svg`);
    check(icon.ok() && (await icon.text()).trim() === (await fs.readFile(path.join(__dirname, '../public/brand/qinpanel-mark.svg'), 'utf8')).trim(), 'Q favicon is packaged and served');
    authenticated = true;
    await page.goto(`${base}/page-that-does-not-exist`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('heading', { name: 'Halaman tidak ditemukan.', exact: true }).waitFor();
    check(await page.getByRole('link', { name: 'Ke beranda', exact: true }).getAttribute('href') === '/dashboard', 'panel missing page offers a dashboard link');
    check(await page.getByRole('link', { name: 'Lihat situs', exact: true }).getAttribute('href') === '/sites', 'panel missing page offers a site-list link');
    check(await page.locator('html').evaluate(el => el.classList.contains('dark')), 'legacy theme preference survives the rebrand');
    check(!/Nusa Panel|Nusapanel|QonPanel/i.test(await page.locator('body').innerText()), 'panel shell and breadcrumb use Qinpanel');
    check(!await page.getByText('nav.page-that-does-not-exist', { exact: true }).count(), 'missing-route breadcrumb shows a readable fallback instead of a translation key');
    await page.screenshot({ path: path.join(output, 'panel-404-dark.png') });
    await page.goto(`${base}/sites/${domain}/rules`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('button', { name: 'Halaman Error', exact: true }).click();
    await page.getByText('Belum ada halaman error kustom. Error 404 dari server memakai halaman bawaan Qinpanel.', { exact: true }).waitFor();
    const download = page.getByRole('link', { name: 'Unduh contoh 404 Qinpanel', exact: true });
    check(await download.getAttribute('download') === 'qinpanel-404.html' && await download.getAttribute('href') === '/errors/404.html', 'site rules offer an editable downloadable Qinpanel 404 template');
    check((await (await page.request.get(`${base}/errors/404.html`)).text()) === html, 'downloaded HTML is the same source embedded in generated vhosts');
    await page.getByRole('button', { name: 'Tambah Error Page', exact: true }).click();
    await page.getByPlaceholder('/custom_404.html', { exact: true }).fill('/errors/my-404.html');
    await page.locator('.dialog-panel').getByRole('button', { name: 'Tambahkan', exact: true }).click();
    await page.getByRole('button', { name: 'Simpan Perubahan Aturan', exact: true }).click();
    await page.getByText(/Aturan situs berhasil diterapkan/).waitFor();
    assert.deepEqual(saved.custom_error_pages, [{ status_code: 404, target: '/errors/my-404.html' }]);
    check(true, 'custom 404 form sends the canonical code/target fields');
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.getByRole('button', { name: 'Halaman Error', exact: true }).click();
    check(await page.getByRole('cell', { name: '/errors/my-404.html', exact: true }).isVisible(), 'custom 404 remains visible after reload');
    await page.route('**/fixture-missing', route => route.fulfill({ status: 404, contentType: 'text/html; charset=utf-8', body: html }));
    const response = await page.goto(`${base}/fixture-missing`, { waitUntil: 'domcontentloaded' });
    check(response.status() === 404, 'branded HTML can be rendered with an HTTP 404 response fixture');
    await page.getByRole('heading', { name: 'Halaman tidak ditemukan.', exact: true }).waitFor();
    check(!/nginx|Nusa/i.test(await page.locator('body').innerText()), 'website error body shows Qinpanel without the default server footer');
    check(await page.getByRole('link', { name: 'Ke halaman utama', exact: true }).getAttribute('href') === '/', 'website action stays on the current website');
    await page.keyboard.press('Tab');
    check(await page.getByRole('link', { name: 'Ke halaman utama', exact: true }).evaluate(el => el === document.activeElement), 'website recovery link supports keyboard focus');
    await page.emulateMedia({ colorScheme: 'light' });
    await page.screenshot({ path: path.join(output, 'website-404-light.png') });
    await page.emulateMedia({ colorScheme: 'dark' });
    await page.screenshot({ path: path.join(output, 'website-404-dark.png') });
    await page.setViewportSize({ width: 320, height: 740 });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'website 404 fits a 320px screen without horizontal overflow');
    await page.screenshot({ path: path.join(output, 'website-404-mobile.png') });
    check(!html.includes('<script') && external.length === 0, 'error page needs no script or external image/font service');
    check(errors.length === 0, 'no browser runtime errors');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors, external, saved }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
