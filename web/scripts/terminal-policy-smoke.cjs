/* Browser checks use intercepted API fixtures; no API/agent or real PTY runs. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
let checks = 0;
const check = (value, message) => { assert.ok(value, message); checks++; };
const output = path.join(__dirname, '../artifacts/terminal-policy');
const tenant = id => ({ id, username: id === 42 ? 'alice' : 'bob', role: 'user', status: 'active', linux_username: `p_tenant${id}`, linux_uid: 10000 + id, plan_id: null, created_at: 1, updated_at: 1 });

async function fixture(browser, role = 'admin') {
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  page.setDefaultTimeout(10000);
  const calls = [], errors = [];
  const mode = { policy: 'none', fail: false, mismatch: false, override: false };
  page.on('pageerror', error => { errors.push(error.message); console.error('Browser page error:', error.message); });
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), url = new URL(request.url()), name = url.pathname;
    calls.push({ name, method: request.method(), body: request.postDataJSON() });
    let status = 200, body = {};
    if (name === '/api/v1/auth/me') body = { user: { id: role === 'admin' ? 1 : 42, username: role === 'admin' ? 'admin' : 'alice', role }, csrf_token: 'fixture' };
    else if (name === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (/\/tenants\/\d+\/shell-policy$/.test(name)) {
      const id = Number(name.split('/')[4]);
      if (mode.fail) { status = 503; body = { error: 'Vérifikasi kebijakan fixture gagal' }; }
      else body = { user_id: mode.mismatch ? 99 : id, username: tenant(id).username,
        shell_policy: mode.override ? mode.policy : null, effective_policy: mode.policy,
        inherited_from_plan: !mode.override, plan_policy: mode.override ? 'full' : mode.policy };
    } else if (name === '/api/v1/tenants') body = { tenants: [tenant(42), tenant(43)] };
    else if (/\/tenants\/\d+\/resources$/.test(name)) body = { metrics: { memory_current_bytes: 0, cpu_percent: 0 }, quota: {} };
    else if (/\/tenants\/\d+$/.test(name)) body = { tenant: tenant(Number(name.split('/')[4])), plan: null };
    else if (name === '/api/v1/terminal/sessions') { status = 400; body = { error: 'Fixture: panggilan PTY terverifikasi tanpa membuka shell' }; }
    else if (name === '/api/v1/files/list') body = { entries: [], total_count: 0, has_more: false };
    else if (name === '/api/v1/files/trash') body = { entries: [] };
    else if (name.includes('quota')) body = { bytes_used: 0, bytes_limit: 100000, inodes_used: 0, inodes_limit: 1000 };
    else if (name.includes('unread-count')) body = { count: 0, unread_count: 0 };
    else if (name.includes('notifications')) body = { notifications: [], total: 0 };
    else if (name === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 20, total_pages: 0 };
    else if (name === '/api/v1/system/metrics') body = { state: 'waiting', sample: null, sampling_interval_secs: 10 };
    else if (name === '/api/v1/system/info') body = { api_version: '0.1.1', agent_version: '0.1.1', proto_version: 1, agent_connected: true, os: 'linux', arch: 'x86_64', environment: 'testing' };
    else if (name === '/api/v1/system/hardening') body = { cgroup_v2: { ok: true }, hidepid: { ok: true }, tenant_group: { ok: true }, quota: { ok: true }, acl: { ok: true } };
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  return { page, mode, calls, errors, posts: () => calls.filter(call => call.name === '/api/v1/terminal/sessions' && call.method === 'POST') };
}

(async () => {
  const browser = await chromium.launch({ headless: true, executablePath: process.env.NUSA_BROWSER_PATH || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe' });
  try {
    await fs.mkdir(output, { recursive: true });
    const admin = await fixture(browser);
    await admin.page.goto(`${base}/terminal?tenant=42`);
    await admin.page.getByRole('heading', { name: 'Terminal belum diaktifkan' }).waitFor();
    check(admin.posts().length === 0, 'none policy does not request a PTY');
    check(await admin.page.getByRole('button', { name: 'Tab Baru', exact: true }).isDisabled(), 'new tab is disabled while access is blocked');
    check(await admin.page.getByRole('link', { name: 'Atur akses terminal' }).getAttribute('href') === '/tenants/42/overview?section=shell', 'admin settings point to the selected tenant');
    admin.mode.policy = 'sftp';
    await admin.page.getByRole('button', { name: 'Periksa ulang akses' }).click();
    await admin.page.getByText(/Tenant ini memakai mode SFTP saja/).waitFor();
    check(admin.posts().length === 0, 'SFTP policy does not request a PTY');
    await admin.page.screenshot({ path: path.join(output, 'terminal-sftp-admin.png'), fullPage: true });
    await admin.page.getByRole('link', { name: 'Atur akses terminal' }).click();
    await admin.page.getByLabel('Pilih kebijakan shell untuk tenant ini').waitFor();
    check(await admin.page.locator('[aria-label="Kebijakan shell dan akses terminal"]').evaluate(element => element === document.activeElement), 'settings route focuses the shell policy card');
    check(await admin.page.getByLabel('Pilih kebijakan shell untuk tenant ini').inputValue() === 'inherit', 'opening settings does not change policy');
    check(!admin.calls.some(call => call.method === 'PUT'), 'no automatic policy upgrade or privilege change');

    admin.mode.policy = 'full';
    await admin.page.goto(`${base}/terminal?tenant=42`);
    await admin.page.getByRole('alert').filter({ hasText: 'Fixture: panggilan PTY' }).waitFor();
    check(admin.posts().length === 1 && admin.posts()[0].body.tenant === '42', 'full access creates one session for the selected tenant');
    await admin.page.waitForTimeout(400);
    check(admin.posts().length === 1, 'failed session does not automatically retry');
    admin.mode.fail = true;
    await admin.page.goto(`${base}/terminal?tenant=42`);
    await admin.page.getByRole('alert').filter({ hasText: 'fixture gagal' }).waitFor();
    check(admin.posts().length === 1, 'failed policy check blocks session creation');
    admin.mode.fail = false;
    admin.mode.mismatch = true;
    await admin.page.getByRole('button', { name: 'Periksa ulang akses' }).click();
    await admin.page.getByRole('alert').filter({ hasText: 'tidak sesuai dengan tenant' }).waitFor();
    check(admin.posts().length === 1, 'wrong tenant policy response cannot grant access');
    admin.mode.mismatch = false;
    admin.mode.policy = 'none';
    admin.mode.override = true;
    await admin.page.getByRole('button', { name: 'Periksa ulang akses' }).click();
    await admin.page.getByText(/Pengaturan khusus tenant/).waitFor();
    check(admin.posts().length === 1, 'tenant none override stays blocked even with a full plan');

    await admin.page.goto(`${base}/files?tenant=42`);
    await admin.page.getByRole('button', { name: 'Terminal di sini' }).click();
    await admin.page.getByRole('heading', { name: 'Terminal belum diaktifkan' }).waitFor();
    check(admin.posts().length === 1, 'file-manager terminal drawer enforces policy preflight');
    await admin.page.getByRole('button', { name: 'Tutup terminal berkas', exact: true }).click();
    admin.mode.policy = 'full';
    await admin.page.getByRole('button', { name: 'Terminal di sini' }).click();
    await admin.page.getByRole('alert').filter({ hasText: 'Fixture: panggilan PTY' }).waitFor();
    check(admin.posts().length === 2, 'reopened drawer rechecks the policy before opening a session');

    const customer = await fixture(browser, 'user');
    customer.mode.policy = 'sftp';
    await customer.page.setViewportSize({ width: 390, height: 700 });
    await customer.page.goto(`${base}/terminal`);
    await customer.page.getByText(/Hubungi administrator/).waitFor();
    check(await customer.page.getByRole('link', { name: 'Atur akses terminal' }).count() === 0, 'regular tenant has no administrator policy action');
    check(customer.posts().length === 0, 'blocked customer does not request a PTY');
    check(customer.calls.some(call => call.name === '/api/v1/tenants/42/shell-policy'), 'customer preflight uses their authenticated identity');
    check(await customer.page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'mobile denial state has no horizontal overflow');
    await customer.page.screenshot({ path: path.join(output, 'terminal-sftp-tenant-mobile.png'), fullPage: true });
    check(admin.errors.length === 0 && customer.errors.length === 0, `no browser errors: ${admin.errors.concat(customer.errors)}`);
    const result = { checks, errors: admin.errors.concat(customer.errors) };
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
  } catch (error) {
    const pages = browser.contexts().flatMap(context => context.pages());
    for (const page of pages) console.error('Fixture browser state:', page.url(), (await page.locator('body').innerText()).slice(0, 4000));
    throw error;
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
