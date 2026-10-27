/* Static production bundle with API fixtures; no panel daemon or service changes. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5196';
const output = path.join(__dirname, '../artifacts/audit-categories');
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
const row = (id, category, action, target, status = 'success', details = '{"source":"fixture"}') => ({ id, category, action, target_resource: target, status, details_json: details, user_id: 1, actor_username: 'admin', ip_address: '203.0.113.5', created_at: 1792857600 + id });
const records = [
  ...Array.from({ length: 17 }, (_, i) => row(100 + i, 'websites', i % 2 ? 'domain.create' : 'site.create', i ? `demo-${i}.test` : 'special.test', i ? 'success' : 'failure')),
  ...Array.from({ length: 18 }, (_, i) => row(200 + i, 'authentication', i % 2 ? 'auth.logout' : 'auth.login', 'auth')),
  row(300, 'other', 'future.action', 'legacy-target', 'success', '{legacy-invalid-json'),
  row(400, 'databases', 'database.create', 'demo_db'),
];
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge' });
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1080 }, reducedMotion: 'reduce' });
    const errors = [], external = [], requests = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('request', request => { if (new URL(request.url()).origin !== new URL(base).origin) external.push(request.url()); });
    let failDatabase = true;
    await page.route('**/api/v1/**', async route => {
      const url = new URL(route.request().url());
      let status = 200, body = {};
      if (url.pathname === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/audit/logs') {
        const params = url.searchParams;
        requests.push(Object.fromEntries(params));
        if (params.get('category') === 'runtimes') await new Promise(resolve => setTimeout(resolve, 350));
        if (params.get('category') === 'databases' && failDatabase) {
          status = 503; body = { error: 'Fixture unavailable' }; failDatabase = false;
        } else if (params.get('q') === '__invalid') body = { items: null, total: -1, total_pages: 0 };
        else {
          let selected = records.filter(item => !params.get('category') || item.category === params.get('category'));
          if (params.get('status')) selected = selected.filter(item => item.status === params.get('status'));
          if (params.get('q')) selected = selected.filter(item => [item.action, item.actor_username, item.target_resource, item.ip_address].some(value => value.toLowerCase().includes(params.get('q').toLowerCase())));
          const number = Number(params.get('page') || 1), perPage = Number(params.get('per_page') || 15);
          body = { items: selected.slice((number - 1) * perPage, number * perPage), total: selected.length, page: number, per_page: perPage, total_pages: Math.ceil(selected.length / perPage) };
        }
      } else if (url.pathname.includes('notifications')) body = { notifications: [], count: 0, unread_count: 0, total: 0 };
      else if (url.pathname === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 20, total_pages: 0 };
      else { status = 404; body = { error: 'No fixture' }; }
      try { await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) }); } catch { /* intentionally aborted stale request */ }
    });
    const categories = () => page.getByRole('group', { name: 'Kategori aktivitas' });
    const category = name => categories().getByRole('button', { name, exact: true });
    const ready = () => page.getByRole('region', { name: 'Daftar aktivitas audit' }).waitFor();
    await page.goto(`${base}/audit?category=websites`);
    await ready();
    check(await category('Website dan domain').getAttribute('aria-pressed') === 'true', 'deep link selects website category');
    check((await page.locator('tbody tr').count()) === 15, 'website category is paginated');
    check(!(await page.locator('tbody').innerText()).includes('auth.login'), 'website view excludes authentication fixture records');
    check(await page.getByText('17 catatan', { exact: true }).isVisible(), 'total reflects full selected category');
    await page.screenshot({ path: path.join(output, 'desktop-light.png'), fullPage: true });
    await page.getByRole('button', { name: 'Halaman berikutnya' }).click();
    await page.getByText('Halaman 2 dari 2', { exact: true }).waitFor();
    check((await page.locator('tbody tr').count()) === 2, 'last page has remaining website records');
    await category('Autentikasi').click(); await ready();
    check(!new URL(page.url()).searchParams.has('page'), 'category change resets pagination');
    check(await page.getByText('18 catatan', { exact: true }).isVisible(), 'authentication category has independent total');
    check(!(await page.locator('tbody').innerText()).includes('site.create'), 'authentication category excludes website records');
    await category('Website dan domain').click(); await ready();
    await page.getByLabel('Cari aktivitas', { exact: true }).fill('special.test');
    await page.getByLabel('Cari aktivitas', { exact: true }).press('Enter');
    await page.getByText('1 catatan', { exact: true }).waitFor();
    await page.getByLabel('Status', { exact: true }).selectOption('failure'); await ready();
    check(requests.at(-1).category === 'websites' && requests.at(-1).q === 'special.test' && requests.at(-1).status === 'failure', 'category search and status are sent together to API');
    await page.reload(); await ready();
    check(await page.getByLabel('Cari aktivitas', { exact: true }).inputValue() === 'special.test' && await page.getByLabel('Status', { exact: true }).inputValue() === 'failure', 'reload preserves category search and status');
    await page.getByRole('button', { name: 'Reset filter', exact: true }).first().click(); await ready();
    check(!new URL(page.url()).search, 'reset restores all activity');
    await category('Email').click();
    await page.getByRole('heading', { name: 'Belum ada catatan audit yang cocok' }).waitFor();
    check(await page.locator('tbody').count() === 0, 'empty category never shows old rows');
    await category('Lainnya').click(); await ready();
    const inspect = page.getByRole('button', { name: 'Lihat detail audit 300', exact: true });
    await inspect.click();
    await page.getByRole('dialog', { name: 'Detail audit #300' }).waitFor();
    check(await page.getByRole('dialog').getByText('{legacy-invalid-json', { exact: true }).isVisible(), 'malformed historical JSON remains inspectable');
    check(await page.getByRole('dialog').getByText('Lainnya', { exact: true }).isVisible(), 'detail includes category');
    await page.keyboard.press('Escape');
    check(await page.getByRole('dialog').count() === 0, 'Escape closes audit details');
    check(await inspect.evaluate(el => el === document.activeElement), 'closing details restores focus to its button');
    await category('Database').click();
    await page.getByRole('alert').waitFor();
    check(await page.locator('tbody').count() === 0, 'failed category request removes previous results');
    await page.getByRole('button', { name: 'Coba lagi', exact: true }).click(); await ready();
    check((await page.locator('tbody').innerText()).includes('database.create'), 'retry recovers selected category');
    await category('Runtime').click();
    await category('Jaringan').click();
    await page.getByRole('heading', { name: 'Belum ada catatan audit yang cocok' }).waitFor();
    await page.waitForTimeout(450);
    check(await category('Jaringan').getAttribute('aria-pressed') === 'true' && !(await page.locator('tbody').count()), 'delayed previous category cannot overwrite current results');
    await category('Website dan domain').click(); await ready();
    await page.getByLabel('Cari aktivitas', { exact: true }).fill('__invalid');
    await page.getByLabel('Cari aktivitas', { exact: true }).press('Enter');
    await page.getByRole('alert').waitFor();
    check((await page.getByRole('alert').innerText()).includes('tidak valid'), 'malformed API response has a recoverable error');
    await page.getByRole('button', { name: 'Reset filter', exact: true }).click(); await ready();
    await page.goBack(); await page.getByRole('alert').waitFor();
    check(await page.getByLabel('Cari aktivitas', { exact: true }).inputValue() === '__invalid', 'browser back restores the previous filter');
    await page.goto(`${base}/audit?category=websites&page=999`); await ready();
    check(new URL(page.url()).searchParams.get('page') === '2', 'out-of-range page is corrected while preserving category');
    await page.evaluate(() => localStorage.setItem('nusa_theme', 'dark'));
    await page.goto(`${base}/audit?category=websites`); await ready();
    check(await page.locator('html').evaluate(el => el.classList.contains('dark')), 'dark theme remains supported');
    await page.screenshot({ path: path.join(output, 'desktop-dark.png'), fullPage: true });
    await page.setViewportSize({ width: 320, height: 800 });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), '320px page has no horizontal overflow outside table');
    const mobileCategory = page.getByRole('combobox', { name: 'Kategori aktivitas', exact: true });
    check(await mobileCategory.evaluate(el => el.getBoundingClientRect().height >= 44), 'mobile category selector has a 44px touch target');
    check(await mobileCategory.locator('option').count() === 13 && await mobileCategory.inputValue() === 'websites', 'compact mobile selector includes every category and keeps selection');
    await mobileCategory.selectOption('authentication'); await ready();
    check(requests.at(-1).category === 'authentication' && await page.getByText('18 catatan', { exact: true }).isVisible(), 'mobile category selection filters records through the API');
    await mobileCategory.selectOption('websites'); await ready();
    await page.screenshot({ path: path.join(output, 'mobile-dark.png'), fullPage: true });
    await page.getByRole('button', { name: 'Lihat detail audit 100', exact: true }).click();
    await page.getByRole('dialog').waitFor();
    check(await page.getByRole('dialog').evaluate(el => el.scrollWidth <= el.clientWidth), 'detail dialog fits narrow screens');
    await page.screenshot({ path: path.join(output, 'mobile-detail.png') });
    await page.keyboard.press('Escape');
    await page.setViewportSize({ width: 1440, height: 1080 });
    await page.getByRole('button', { name: 'Ganti bahasa', exact: true }).click();
    await page.getByRole('heading', { name: 'Audit log', exact: true }).waitFor();
    check(await page.getByRole('group', { name: 'Activity categories' }).getByRole('button', { name: 'Websites and domains', exact: true }).getAttribute('aria-pressed') === 'true', 'English labels retain the selected category');
    await page.getByRole('region', { name: 'Audit activity records' }).waitFor();
    check(errors.length === 0, 'no browser runtime errors');
    check(external.length === 0, 'browser fixture made no external requests');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors, external, requests }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
