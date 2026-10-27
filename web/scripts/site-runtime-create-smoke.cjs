/* Intercepted API fixtures only. No panel daemon, package installation, or server access. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5185';
let checks = 0;
function check(condition, message) { assert.ok(condition, message); checks++; }
const owner = { id: 42, username: 'customer', linux_username: 'p_actual', linux_uid: 10042, status: 'active', role: 'user' };
const versions = { php: '8.3', node: '24.2.1', python: '3.13.7' };

async function fixture(browser, initial = {}) {
  const page = await browser.newPage();
  const mode = { installed: true, fail: false, createFail: false, role: 'admin', ...initial };
  const calls = [], errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), path = new URL(request.url()).pathname;
    calls.push({ path, method: request.method(), body: request.postDataJSON() });
    let body = {}, status = 200;
    if (path === '/api/v1/auth/me') body = { user: { id: mode.role === 'admin' ? 1 : 42, username: mode.role, role: mode.role }, csrf_token: 'fixture' };
    else if (path === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (path === '/api/v1/sites' && request.method() === 'POST') {
      status = mode.createFail ? 422 : 201;
      body = mode.createFail ? { error: 'Runtime gagal diterapkan: entrypoint fixture tidak ditemukan.', code: 'site_provisioning_failed' }
        : { id: 7, user_id: 42, ...request.postDataJSON(), status: 'active', primary_domain: 'app.example.test', domains: [] };
    } else if (path === '/api/v1/sites') body = [];
    else if (path === '/api/v1/php/versions') body = { ppa_configured: true, versions: [{ version: versions.php, installed: true, fpm_binary_exists: true }] };
    else if (path === '/api/v1/tenants') body = { tenants: [owner] };
    else if (path === '/api/v1/tenants/42') body = { tenant: owner, plan: null };
    else if (path === '/api/v1/runtimes/node/versions' || path === '/api/v1/runtimes/python') {
      const kind = path.includes('/node/') ? 'node' : 'python';
      if (kind === 'node' && mode.nodeGate) await mode.nodeGate;
      status = mode.fail ? 503 : 200;
      body = mode.fail ? { error: 'Inventaris runtime tidak tersedia.' } : { versions: [
        { version: versions[kind], is_installed: mode.installed, bin_path: '/fixture/bin/runtime' },
        { version: kind === 'node' ? '22.14.0' : '3.12.9', is_installed: false },
      ] };
    } else if (path === '/api/v1/notifications') body = { notifications: [], unread_count: 0 };
    else if (path === '/api/v1/notifications/unread-count') body = { unread_count: 0 };
    else if (path === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 6 };
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  await page.goto(base + '/sites', { waitUntil: 'domcontentloaded' });
  try {
    await page.getByRole('button', { name: 'Situs baru', exact: true }).click({ timeout: 10000 });
  } catch (error) {
    console.error({ url: page.url(), errors, text: (await page.locator('body').innerText()).slice(0, 1500), paths: calls.map(call => call.path) });
    throw error;
  }
  return { page, mode, calls, errors, dialog: page.getByRole('dialog') };
}
const name = kind => ({ php: 'PHP', node: 'Node.js', python: 'Python' })[kind];
async function choose(f, kind) {
  await f.dialog.getByRole('button', { name: new RegExp(kind === 'php' ? '^PHP' : kind === 'node' ? 'Node\\.js' : 'Aplikasi Python') }).click();
}
async function ready(f, kind) {
  await f.page.waitForFunction(() => [...document.querySelectorAll('[role="dialog"] button')].some(button => button.textContent.trim() === 'Lanjutkan' && !button.disabled));
  check(await f.dialog.getByText(/\d+ versi .* terpasang\./).count() === 0, `${kind}: ready inventory has no redundant installed-version count`);
  check(await f.dialog.getByRole('status').count() === 0, `${kind}: successful inventory leaves no empty status panel`);
}
async function review(f, kind) {
  await f.dialog.getByRole('button', { name: 'Lanjutkan', exact: true }).click();
  const select = f.dialog.getByLabel(`Versi ${name(kind)}`);
  check(await select.inputValue() === versions[kind], `${kind}: selects an actually installed version`);
  assert.deepEqual(await select.locator('option').evaluateAll(options => options.map(option => option.value)), [versions[kind]]); checks++;
  await f.dialog.getByPlaceholder('situsku.com atau demo.perusahaan.id').fill('app.example.test');
  await f.dialog.getByRole('button', { name: 'Lanjutkan', exact: true }).click();
  if (f.mode.role === 'admin') await f.dialog.getByRole('button', { name: /customer/ }).click();
  await f.dialog.getByRole('button', { name: 'Lanjutkan', exact: true }).click();
}

(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    const php = await fixture(browser);
    await choose(php, 'php'); await ready(php, 'php');
    await review(php, 'php');
    check(php.errors.length === 0, 'PHP wizard retains installed version selection without browser exceptions');
    await php.page.close();
    for (const kind of ['node', 'python']) {
      let release;
      const nodeGate = kind === 'node' ? new Promise(resolve => { release = resolve; }) : undefined;
      const f = await fixture(browser, { nodeGate, createFail: true });
      check(await f.dialog.getByText('Tersedia', { exact: true }).count() === 0 && await f.dialog.getByText('PHP-FPM', { exact: true }).count() === 0, 'site type cards omit redundant badges');
      if (kind === 'node') {
        const card = f.dialog.getByRole('button', { name: /Node\.js/ });
        await card.focus(); await card.press('Space');
        check(await card.getAttribute('aria-pressed') === 'true', 'runtime card supports keyboard selection');
      } else await choose(f, kind);
      if (release) {
        await f.dialog.getByRole('status').filter({ hasText: 'Memuat runtime Node.js' }).waitFor();
        check(await f.dialog.getByRole('button', { name: 'Lanjutkan' }).isDisabled(), 'pending inventory blocks progression');
        release();
      }
      await ready(f, kind);
      await review(f, kind);
      await f.dialog.getByRole('button', { name: 'Terapkan & Buat Situs' }).click();
      await f.dialog.getByRole('alert').filter({ hasText: 'entrypoint fixture tidak ditemukan' }).waitFor();
      check(await f.dialog.isVisible(), `${kind}: diagnostic stays visible after a 422 rejection`);
      const request = f.calls.find(call => call.path === '/api/v1/sites' && call.method === 'POST').body;
      check(request[kind === 'node' ? 'node_version' : 'python_version'] === versions[kind], `${kind}: sends the installed version`);
      check(request.user_id === 42, `${kind}: retains explicit tenant identity`);
      f.mode.createFail = false;
      await f.dialog.getByRole('button', { name: 'Terapkan & Buat Situs' }).click();
      await f.dialog.waitFor({ state: 'hidden' }); checks++;
      check(f.errors.length === 0, `${kind}: no browser exceptions`);
      await f.page.close();
    }
    for (const kind of ['node', 'python']) {
      for (const fail of [false, true]) {
        const f = await fixture(browser, { installed: false, fail });
        await choose(f, kind);
        await f.dialog.getByRole(fail ? 'alert' : 'status').filter({ hasText: fail ? 'Inventaris runtime tidak tersedia' : `Belum ada runtime ${name(kind)}` }).waitFor();
        check(await f.dialog.getByRole('button', { name: 'Lanjutkan' }).isDisabled(), `${kind}: empty/failed inventory blocks progression`);
        check(!f.calls.some(call => call.method === 'POST' && call.path === '/api/v1/sites'), `${kind}: no create during inventory failure`);
        f.mode.installed = true; f.mode.fail = false;
        await f.dialog.getByRole('button', { name: 'Coba lagi' }).click();
        await ready(f, kind);
        check(await f.dialog.getByRole('button', { name: 'Lanjutkan' }).isEnabled(), `${kind}: explicit retry restores progression`);
        await f.page.close();
      }
    }
    let release;
    const f = await fixture(browser, { nodeGate: new Promise(resolve => { release = resolve; }), role: 'user' });
    await f.page.setViewportSize({ width: 390, height: 844 });
    await choose(f, 'node'); await choose(f, 'python'); await ready(f, 'python');
    const lateNode = f.page.waitForResponse(response => response.url().includes('/runtimes/node/versions'));
    release(); await lateNode;
    check(await f.dialog.getByRole('button', { name: /Aplikasi Python/ }).getAttribute('aria-pressed') === 'true' && await f.dialog.getByRole('status').count() === 0, 'late Node response does not replace ready Python selection');
    await review(f, 'python');
    check(await f.dialog.evaluate(element => element.scrollWidth <= element.clientWidth + 1), 'mobile dialog has no horizontal overflow');
    check(f.errors.length === 0, 'tenant wizard has no browser exceptions');
    await f.dialog.focus(); await f.page.keyboard.press('Escape');
    await f.dialog.waitFor({ state: 'hidden' }); checks++;
    await f.page.close();
    console.log(`${checks} runtime-create browser checks passed (API fixtures only).`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
