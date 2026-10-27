/* Production frontend with intercepted APIs only; no mail/panel daemons or CA requests. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
(async () => {
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || undefined });
  try {
    const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
    const page = await context.newPage();
    page.setDefaultTimeout(20000);
    const errors = [], installs = [];
    page.on('pageerror', error => errors.push(error.message));
    const mode = { certificates: [], accepted: false, fail: false };
    const site = { id: 7, user_id: 2, kind: 'static', primary_domain: 'mail.example.com', docroot: '/home/p_customer/web/mail/public', status: 'active', domains: [], created_at: 1, updated_at: 1 };
    const job = () => ({ id: 'email-tls-fixture', user_id: 1, name: 'email.install', status: 'queued', progress: 0, payload_json: '{}', created_at: 1, updated_at: 1 });
    await context.route('**/api/v1/**', async route => {
      const request = route.request(), url = new URL(request.url()), name = url.pathname;
      let body = {}, status = 200;
      if (name === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (name === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (name === '/api/v1/email') body = { mailboxes: [], domains: [{ id: 1, site_id: 7, name: 'mail.example.com' }], certificates: mode.certificates, server: { configured: false, server: null, services: [] }, server_error: null };
      else if (name === '/api/v1/sites') body = [site];
      else if (name === '/api/v1/sites/7') body = site;
      else if (name === '/api/v1/sites/7/ssl') body = { has_ssl: false, certificate: null, force_https: false };
      else if (name === '/api/v1/email/install') {
        installs.push(request.postDataJSON());
        if (mode.fail) { status = 400; body = { error: 'Sertifikat sudah tidak berlaku. Muat ulang sertifikat.' }; }
        else { mode.accepted = true; status = 202; body = { job: job(), duplicate: false }; }
      } else if (name === '/api/v1/jobs') {
        const items = mode.accepted && url.searchParams.get('status') === 'queued' ? [job()] : [];
        body = { items, total: items.length, total_pages: 1, page: 1, per_page: 100 };
      } else if (name === '/api/v1/jobs/email-tls-fixture') body = job();
      else if (name.includes('unread-count')) body = { count: 0, unread_count: 0 };
      else if (name.includes('notifications')) body = { notifications: [], total: 0 };
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    await page.goto(`${base}/email`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('button', { name: 'Server email', exact: true }).click();
    await page.getByRole('heading', { name: 'Belum ada sertifikat TLS yang dapat dipakai.' }).waitFor();
    const host = page.getByLabel('Hostname server email', { exact: true });
    const select = page.getByLabel('Sertifikat TLS hostname email', { exact: true });
    const install = page.getByRole('button', { name: 'Pasang server email', exact: true });
    check(await install.isDisabled() && await select.isDisabled(), 'missing certificate blocks installation with a visible setup guide');
    check(await page.getByRole('link', { name: 'Buka SSL mail.example.com' }).getAttribute('href') === '/sites/7/ssl', 'guide links to the domain project SSL tab');
    await page.getByRole('link', { name: 'Buka SSL mail.example.com' }).click();
    await page.waitForURL('**/sites/mail.example.com/ssl');
    await page.getByRole('button', { name: /Let.s Encrypt/ }).first().waitFor();
    check(true, 'SSL shortcut resolves the canonical project route and opens certificate controls');
    await page.goto(`${base}/email`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('button', { name: 'Server email', exact: true }).click();
    await host.waitFor();
    await host.fill('mail.example.com');
    check(await page.getByText('Arahkan A/AAAA hostname email', { exact: false }).isVisible(), 'empty state explains DNS only and public IP setup');
    mode.certificates = [{ id: 9, hostname: 'unrelated.test', san_domains: [] }];
    await page.getByRole('button', { name: 'Muat ulang email', exact: true }).click();
    await page.getByRole('heading', { name: 'Belum ada sertifikat yang mencakup mail.example.com.' }).waitFor();
    check(await install.isDisabled(), 'unrelated certificates cannot enable installation');
    mode.certificates.push({ id: 10, hostname: 'example.com', san_domains: ['mail.example.com'] });
    await page.getByRole('button', { name: 'Muat ulang email', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('select')?.value === '10');
    check(await host.inputValue() === 'mail.example.com' && await select.locator('option').count() === 2, 'inventory refresh keeps hostname and selects the single matching SAN certificate');
    await host.fill('deep.mail.example.com');
    check(await install.isDisabled() && await select.inputValue() === '', 'changing hostname invalidates the old certificate selection');
    await host.fill('mail.example.com');
    mode.fail = true; await install.click();
    await page.getByRole('alert').filter({ hasText: 'Sertifikat sudah tidak berlaku' }).waitFor();
    check(await host.inputValue() === 'mail.example.com', 'rejected install preserves the form and shows the server error');
    mode.fail = false; await install.click();
    await page.getByRole('button', { name: 'Pemasangan sedang berjalan', exact: true }).waitFor();
    check(JSON.stringify(installs[1]) === JSON.stringify({ hostname: 'mail.example.com', certificate_id: 10 }), 'installation sends only hostname and selected certificate ID');
    check(await page.getByRole('button', { name: 'Pemasangan sedang berjalan', exact: true }).isDisabled(), 'queued installation prevents duplicate submissions');
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.getByRole('button', { name: 'Server email', exact: true }).click();
    await page.getByRole('button', { name: 'Pemasangan sedang berjalan', exact: true }).waitFor();
    check(installs.length === 2, 'reload restores the queued job without another install request');
    check(!await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), 'email setup fits a 390px viewport');
    check(errors.length === 0, `no browser exceptions (${errors.join(', ')})`);
    console.log(`${checks} email TLS browser checks passed (API fixtures only).`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
