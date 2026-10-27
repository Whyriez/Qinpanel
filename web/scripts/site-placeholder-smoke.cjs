/* Actual Rust renderer exports; no panel daemons or hosting services. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const output = path.join(__dirname, '../artifacts/site-placeholder');
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };

(async () => {
  await fs.mkdir(output, { recursive: true });
  const render = domain => execFileSync('cargo', [
    'run', '--quiet', '--locked', '--offline', '-p', 'panel-webserver',
    '--example', 'placeholder_preview', '--', domain,
  ], { cwd: path.join(__dirname, '../..'), encoding: 'utf8', maxBuffer: 1024 * 1024 });
  const html = render('testing.incogmsg.my.id');
  const longHtml = render(`toko-é${'a'.repeat(48)}.${'b'.repeat(60)}.${'c'.repeat(60)}.contoh.id`);
  await fs.writeFile(path.join(output, 'index.html'), html);
  await fs.writeFile(path.join(output, 'long-domain.html'), longHtml);
  const browser = await chromium.launch({ headless: true, channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge' });
  const errors = [], requests = [];
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce' });
    page.on('pageerror', error => errors.push(error.message));
    page.on('request', req => requests.push(req.url()));
    await page.route('https://welcome.test/**', route => route.fulfill({ contentType: 'text/html; charset=utf-8', body: route.request().url().endsWith('/long') ? longHtml : html }));
    await page.goto('https://welcome.test/', { waitUntil: 'domcontentloaded' });
    await page.evaluate(() => document.fonts.ready);
    check(await page.locator('html').getAttribute('lang') === 'id', 'Indonesian document language');
    check(await page.getByRole('heading', { level: 1, name: 'testing.incogmsg.my.id', exact: true }).count() === 1, 'one heading identifies the actual site');
    check(await page.getByRole('main').count() === 1 && await page.getByRole('listitem').count() === 3, 'semantic main and three publishing steps');
    check(await page.getByText('Situs berhasil dibuat', { exact: true }).isVisible(), 'site status has a readable label');
    check(await page.getByRole('complementary').isVisible(), 'visitor guidance is separate from owner instructions');
    check(await page.evaluate(() => document.fonts.check('14px "Plus Jakarta Sans"')), 'bundled Jakarta font loads');
    check(await page.locator('script, a, button').count() === 0, 'no unnecessary scripts or invented panel login links');
    const colors = {};
    for (const colorScheme of ['light', 'dark']) {
      await page.emulateMedia({ colorScheme });
      colors[colorScheme] = await page.locator('body').evaluate(el => getComputedStyle(el).backgroundColor);
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${colorScheme} desktop has no horizontal overflow`);
      check(await page.locator('body').evaluate(el => getComputedStyle(el).backgroundImage === 'none'), `${colorScheme} uses a flat background`);
      const contrast = await page.evaluate(() => {
        const luminance = value => {
          const parts = value.match(/[\d.]+/g).slice(0, 3).map(Number).map(n => {
            const v = n / 255;
            return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
          });
          return parts[0] * 0.2126 + parts[1] * 0.7152 + parts[2] * 0.0722;
        };
        const background = luminance(getComputedStyle(document.body).backgroundColor);
        return ['h1', '.lead', '.status', 'li', 'li strong', 'footer'].every(selector => {
          const ink = luminance(getComputedStyle(document.querySelector(selector)).color);
          return (Math.max(ink, background) + 0.05) / (Math.min(ink, background) + 0.05) >= 4.5;
        });
      });
      check(contrast, `${colorScheme} primary text meets AA contrast`);
      await page.screenshot({ path: path.join(output, `desktop-${colorScheme}.png`), fullPage: true });
    }
    check(colors.light !== colors.dark, 'theme follows operating-system preference');
    for (const colorScheme of ['light', 'dark']) {
      await page.emulateMedia({ colorScheme });
      await page.setViewportSize({ width: 320, height: 740 });
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${colorScheme} 320px layout fits`);
      await page.screenshot({ path: path.join(output, `mobile-${colorScheme}.png`), fullPage: true });
    }
    await page.goto('https://welcome.test/long', { waitUntil: 'domcontentloaded' });
    await page.evaluate(() => document.fonts.ready);
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'long international domain wraps on mobile');
    await page.screenshot({ path: path.join(output, 'long-domain-mobile.png'), fullPage: true });
    await page.setViewportSize({ width: 640, height: 450 });
    await page.locator('html').evaluate(el => { el.style.zoom = '2'; });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), '200% zoom reflows without horizontal overflow');
    check(requests.every(url => url.startsWith('https://welcome.test/')), 'no external asset or font requests');
    check(errors.length === 0, 'no browser errors');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors, requests }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
