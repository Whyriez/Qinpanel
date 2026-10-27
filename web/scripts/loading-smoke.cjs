/* Production frontend with delayed assets/session fixtures; no panel daemon. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5197';
const output = path.join(__dirname, '../artifacts/loading');
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
const gate = () => { let release; const promise = new Promise(resolve => { release = resolve; }); return { promise, release }; };
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge', headless: true });
  try {
    for (const [name, preference, system, width] of [['desktop-light', 'light', 'dark', 1440], ['desktop-dark', 'dark', 'light', 1440], ['mobile-dark', 'dark', 'light', 320]]) {
      const page = await browser.newPage({ viewport: { width, height: 900 }, colorScheme: system, reducedMotion: 'reduce' });
      const bundle = gate(), session = gate(), routeChunk = gate();
      const errors = [], external = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('request', request => { if (new URL(request.url()).origin !== new URL(base).origin) external.push(request.url()); });
      await page.addInitScript(value => localStorage.setItem('nusa_theme', value), preference);
      await page.route('**/assets/index-*.js', async route => { await bundle.promise; await route.continue(); });
      await page.route('**/assets/AuditLogPage-*.js', async route => { await routeChunk.promise; await route.continue(); });
      await page.route('**/api/v1/**', async route => {
        const url = new URL(route.request().url());
        let status = 200, body = {};
        if (url.pathname === '/api/v1/auth/me') { await session.promise; body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' }; }
        else if (url.pathname.includes('notifications')) body = { notifications: [], total: 0, count: 0, unread_count: 0 };
        else if (url.pathname === '/api/v1/jobs' || url.pathname === '/api/v1/audit/logs') body = { items: [], total: 0, total_pages: 0, page: 1, per_page: 15 };
        else { status = 404; body = { error: 'No fixture' }; }
        await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
      });
      try {
        await page.goto(`${base}/fixture-missing-page`, { waitUntil: 'commit' });
        await page.locator('.boot-loading').waitFor();
        await page.locator('.boot-loading .brand-mark').evaluate(img => img.decode());
        await page.evaluate(() => document.fonts.ready);
        const boot = await page.locator('.panel-loading__content').boundingBox();
        const bootBackground = await page.locator('.boot-loading').evaluate(el => getComputedStyle(el).backgroundColor);
        check(await page.locator('html').evaluate(el => el.classList.contains('dark')) === (preference === 'dark'), `${name}: saved theme wins before React starts`);
        check(await page.locator('.panel-loading__spinner').evaluate(el => getComputedStyle(el).animationName === 'none'), `${name}: initial loader respects reduced motion`);
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${name}: initial loader fits viewport`);
        await page.screenshot({ path: path.join(output, `${name}-boot.png`) });
        bundle.release();
        await page.locator('.brand-loading--full').waitFor();
        const checking = await page.locator('.panel-loading__content').boundingBox();
        check(Math.abs(boot.x - checking.x) < 1 && Math.abs(boot.y - checking.y) < 1 && Math.abs(boot.width - checking.width) < 1 && Math.abs(boot.height - checking.height) < 1, `${name}: initial and session loading have the same geometry`);
        check(await page.locator('.brand-loading--full').evaluate(el => getComputedStyle(el).backgroundColor) === bootBackground, `${name}: background does not change when React starts`);
        check(await page.locator('.panel-loading .brand-mark').getAttribute('src') === '/brand/qinpanel-mark.svg?v=4', `${name}: loading uses canonical logo`);
        await page.screenshot({ path: path.join(output, `${name}-session.png`) });
        session.release();
        await page.locator('.panel-shell').waitFor();
        if (width >= 768) {
          await page.getByRole('link', { name: 'Audit', exact: true }).click();
        } else {
          await page.evaluate(() => { history.pushState(null, '', '/audit'); dispatchEvent(new PopStateEvent('popstate')); });
        }
        await page.locator('.panel-loading--page').waitFor();
        check(await page.locator('.panel-loading--page .panel-brand').count() === 0, `${name}: route loading does not repeat sidebar branding`);
        check(await page.locator('.panel-loading__placeholder').getAttribute('aria-hidden') === 'true', `${name}: decorative placeholders are hidden from assistive technology`);
        check(await page.locator('.panel-loading--page').innerText().then(value => value.includes('Memuat halaman')), `${name}: route loading announces its state`);
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${name}: route fallback fits viewport`);
        await page.screenshot({ path: path.join(output, `${name}-route.png`) });
        routeChunk.release();
        await page.getByRole('heading', { name: 'Catatan audit', exact: true }).waitFor();
        check(await page.locator('.panel-loading').count() === 0, `${name}: loader leaves when content is ready`);
        check(errors.length === 0 && external.length === 0, `${name}: no runtime errors or external requests`);
      } finally { bundle.release(); session.release(); routeChunk.release(); await page.close(); }
    }
    const noJs = await browser.newPage({ javaScriptEnabled: false, colorScheme: 'dark', viewport: { width: 320, height: 740 }, reducedMotion: 'reduce' });
    await noJs.goto(base);
    check(await noJs.locator('.boot-loading').evaluate(el => getComputedStyle(el).backgroundColor) === 'rgb(16, 23, 28)', 'initial loader follows system dark without JavaScript');
    check(await noJs.getByRole('status').innerText().then(value => value.includes('Qinpanel')), 'initial loader is readable without JavaScript');
    await noJs.close();
    const animated = await browser.newPage({ reducedMotion: 'no-preference' });
    await animated.route('**/assets/index-*.js', route => route.abort());
    await animated.goto(base);
    check(await animated.locator('.panel-loading__spinner').evaluate(el => getComputedStyle(el).animationName === 'panel-loading-spin'), 'motion-enabled loader uses a small indeterminate indicator');
    check(await animated.getByRole('progressbar').count() === 0, 'indeterminate loading makes no percentage claim');
    await animated.setViewportSize({ width: 320, height: 740 });
    await animated.evaluate(() => { document.documentElement.style.zoom = '2'; });
    check(await animated.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'initial loader fits 320px at 200% zoom');
    await animated.screenshot({ path: path.join(output, 'mobile-zoom-200.png') });
    await animated.close();
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
