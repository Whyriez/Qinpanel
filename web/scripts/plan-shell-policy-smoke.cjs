/* Static frontend + API fixtures only; never starts panel daemons. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
const output = path.join(__dirname, '../artifacts/plan-shell-policy');
let checks = 0;
function check(value, message) { assert.ok(value, message); checks++; console.log(`ok ${checks}: ${message}`); }

(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, ...(process.env.NUSA_BROWSER_CHANNEL ? { channel: process.env.NUSA_BROWSER_CHANNEL } : {}) });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    const errors = [], writes = [];
    page.on('pageerror', error => errors.push(error.message));
    let plan = {
      id: 7, name: 'Paket uji', description: null, cpu_quota_percent: 100,
      memory_limit_bytes: 536870912, memory_high_bytes: 419430400, tasks_max: 100,
      io_weight: 100, disk_limit_bytes: 2147483648, inode_limit: 100000,
      max_sites: 5, max_domains: 10, max_databases: 5, max_cron_jobs: 10,
      max_terminal_sessions: 2, max_upload_bytes: 52428800, shell_policy: 'full',
      allow_external_upstream: false, created_at: 1, updated_at: 1,
    };
    await page.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url());
      let status = 200, body = {};
      if (url.pathname === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (url.pathname.includes('notifications')) body = { notifications: [], total: 0, count: 0, unread_count: 0 };
      else if (url.pathname === '/api/v1/jobs') body = { items: [], total: 0, page: 1, total_pages: 1 };
      else if (url.pathname.startsWith('/api/v1/plans')) {
        if (req.method() === 'GET') body = { plans: [plan] };
        else {
          const input = req.postDataJSON();
          writes.push({ method: req.method(), input });
          if (!['none', 'sftp', 'full'].includes(input.shell_policy)) {
            status = 400; body = { error: 'Akses shell harus none, sftp, atau full.' };
          } else {
            plan = { ...plan, ...input, id: req.method() === 'POST' ? plan.id + 1 : plan.id };
            status = req.method() === 'POST' ? 201 : 200;
            body = { plan };
          }
        }
      }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    await page.goto(`${base}/plans`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('heading', { name: 'Paket uji', exact: true }).waitFor();
    check(await page.getByText('Shell interaktif', { exact: true }).isVisible(), 'existing full policy has the correct badge');

    const labels = { none: 'Tanpa akses shell', sftp: 'SFTP saja', full: 'Shell interaktif' };
    for (const policy of ['sftp', 'full', 'none']) {
      await page.getByRole('button', { name: 'Edit', exact: true }).click();
      const field = page.getByLabel('Akses shell', { exact: true });
      check(await field.inputValue() === plan.shell_policy, `edit loads persisted ${plan.shell_policy} policy`);
      await field.selectOption(policy);
      const response = page.waitForResponse(res => res.url().includes('/api/v1/plans/') && res.request().method() === 'PUT');
      await page.getByRole('button', { name: 'Perbarui Paket', exact: true }).click();
      check((await response).status() === 200, `editing ${policy} is accepted`);
      await page.getByRole('dialog').waitFor({ state: 'hidden' });
      check(writes.at(-1).input.shell_policy === policy, `PUT sends canonical ${policy}`);
      await page.getByText(labels[policy], { exact: true }).waitFor();
      await page.reload({ waitUntil: 'domcontentloaded' });
      await page.getByRole('button', { name: 'Edit', exact: true }).click();
      check(await field.inputValue() === policy, `${policy} selection survives refresh and reopening`);
      await page.getByRole('button', { name: 'Batal', exact: true }).click();
      await page.getByRole('dialog').waitFor({ state: 'hidden' });
    }
    for (const policy of ['none', 'sftp', 'full']) {
      await page.getByRole('button', { name: 'Buat Paket Baru', exact: true }).click();
      const dialog = page.getByRole('dialog');
      await dialog.getByRole('textbox').first().fill(`Paket ${policy}`);
      await page.getByLabel('Akses shell', { exact: true }).selectOption(policy);
      const response = page.waitForResponse(res => res.url().endsWith('/api/v1/plans') && res.request().method() === 'POST');
      await page.getByRole('button', { name: 'Simpan Paket', exact: true }).click();
      check((await response).status() === 201, `creating ${policy} is accepted`);
      await dialog.waitFor({ state: 'hidden' });
      check(writes.at(-1).input.shell_policy === policy, `POST sends canonical ${policy}`);
      await page.getByText(labels[policy], { exact: true }).waitFor();
    }
    check(writes.length === 6, 'one request per submitted edit/create');
    check(errors.length === 0, 'no browser render errors');
    await page.screenshot({ path: path.join(output, 'plans.png'), fullPage: true });
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, writes, errors }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
