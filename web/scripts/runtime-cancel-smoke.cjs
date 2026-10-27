/* API fixtures only: no daemon, package manager or runtime installation. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };

async function fixture(browser, runtime, status = 'queued') {
  const page = await browser.newPage({ viewport: { width: runtime === 'python' ? 390 : 1280, height: 850 } });
  page.setDefaultTimeout(20000);
  const mode = { status, installed: false, fail: false, claim: false, hold: false, release: null, win: false };
  const calls = [], errors = [];
  const version = runtime === 'php' ? '8.4' : runtime === 'python' ? '3.15.0' : '24.20.0';
  const job = () => ({ id: 'cancel-fixture', user_id: 1, name: runtime + '.install', status: mode.status,
    progress: mode.status === 'succeeded' ? 100 : 15, payload_json: JSON.stringify({ version }),
    error_message: null, result_json: null, retry_count: 0, max_retries: 1, created_at: 1, updated_at: 2 });
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), url = new URL(request.url()), name = url.pathname;
    calls.push({ name, method: request.method() });
    let body = {}, code = 200;
    if (name === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
    else if (name === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (name === '/api/v1/runtimes/catalog') body = { enabled: true, interval_hours: 24, sources: [] };
    else if (name === '/api/v1/php/versions') body = { ppa_configured: true, versions: runtime === 'php' ? [{ version, installed: mode.installed }] : [] };
    else if (name === '/api/v1/runtimes/node/versions') body = { installed_count: mode.installed ? 1 : 0, is_offline_fallback: false, versions: runtime === 'node' ? [{ version, raw_version: 'v' + version, is_installed: mode.installed, is_lts: true, used_by_sites_count: 0, used_by_domains: [], can_delete: true, corepack_enabled: false }] : [] };
    else if (name === '/api/v1/runtimes/python/versions') body = { catalog_ready: true, versions: runtime === 'python' ? [{ version, major_minor: '3.15', is_installed: mode.installed, is_default: true, is_lts_or_stable: true, description: 'CPython fixture' }] : [] };
    else if (name === '/api/v1/jobs/cancel-fixture/cancel') {
      if (mode.hold) await new Promise(resolve => { mode.release = resolve; });
      if (mode.claim) { mode.status = 'running'; code = 409; body = { error: 'Paket sudah berjalan dan tidak dapat dibatalkan dengan aman' }; }
      else if (mode.fail) { code = 503; body = { error: 'Pembatalan sementara tidak tersedia' }; }
      else { mode.status = mode.win ? 'succeeded' : 'cancelled'; mode.installed = mode.win; body = job(); }
    } else if (name === '/api/v1/jobs') {
      const items = url.searchParams.get('status') === mode.status ? [job()] : [];
      body = { items, total: items.length, total_pages: 1, page: 1, per_page: 100 };
    } else if (name === '/api/v1/jobs/cancel-fixture') body = job();
    else if (name.includes('unread-count')) body = { count: 0, unread_count: 0 };
    else if (name.includes('notifications')) body = { notifications: [], total: 0 };
    await route.fulfill({ status: code, contentType: 'application/json', body: JSON.stringify(body) });
  });
  await page.goto(base + '/runtime?tab=' + runtime, { waitUntil: 'domcontentloaded' });
  const label = runtime === 'php' ? 'PHP ' + version : runtime === 'python' ? 'Python ' + version : 'Node.js v' + version;
  const cancel = page.getByRole('button', { name: 'Batalkan pemasangan ' + label, exact: true });
  const progress = page.getByRole('progressbar', { name: 'Progres tugas ' + label });
  await progress.waitFor();
  return { page, mode, calls, errors, cancel, progress, runtime };
}
const cancellationCalls = f => f.calls.filter(call => call.name.endsWith('/cancel')).length;
async function finish(f) {
  check(!f.calls.some(call => call.method === 'POST' && call.name.endsWith('/install')), `${f.runtime}: cancellation never resubmits installation`);
  check(f.errors.length === 0, `${f.runtime}: no browser exceptions (${f.errors.join(', ')})`);
  await f.page.close();
}

(async () => {
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || 'msedge' });
  try {
    await fs.mkdir(path.join(__dirname, '../artifacts/runtime-cancel'), { recursive: true });
    for (const runtime of ['node', 'python']) {
      const f = await fixture(browser, runtime);
      check(await f.cancel.isEnabled(), `${runtime}: queued install has a direct Cancel button`);
      await f.cancel.scrollIntoViewIfNeeded();
      await f.page.screenshot({ path: path.join(__dirname, `../artifacts/runtime-cancel/${runtime}.png`) });
      f.mode.hold = true;
      await f.cancel.evaluate(button => { button.click(); button.click(); });
      await f.page.waitForFunction(() => [...document.querySelectorAll('button')].some(button => button.textContent.includes('Membatalkan')));
      check(await f.cancel.isDisabled(), `${runtime}: duplicate cancellation blocked while pending`);
      const deadline = Date.now() + 20000;
      while (!f.mode.release && Date.now() < deadline) await f.page.waitForTimeout(20);
      assert.ok(f.mode.release); f.mode.release();
      await f.page.getByText('Permintaan pembatalan diterima.', { exact: false }).waitFor();
      check(cancellationCalls(f) === 1 && await f.cancel.isDisabled(), `${runtime}: acknowledged cancellation stays blocked pending inventory`);
      await f.progress.waitFor({ state: 'detached' });
      await f.page.reload({ waitUntil: 'domcontentloaded' });
      const install = f.page.getByRole('button', { name: runtime === 'node' ? 'Pasang Node.js 24.20.0' : 'Pasang Python 3.15.0', exact: true });
      await install.waitFor();
      check(await install.isEnabled() && await f.cancel.count() === 0, `${runtime}: cancelled job stays cancelled after reload and retry is available`);
      check(!await f.page.evaluate(() => document.documentElement.scrollWidth > innerWidth), `${runtime}: cancellation layout fits viewport`);
      await finish(f);

      const running = await fixture(browser, runtime, 'running');
      check(await running.cancel.isEnabled(), `${runtime}: running installation can be cancelled`);
      running.mode.fail = true;
      await running.cancel.click();
      await running.page.getByRole('alert').filter({ hasText: 'Pembatalan sementara tidak tersedia' }).waitFor();
      check(await running.cancel.isEnabled() && await running.progress.isVisible(), `${runtime}: failed cancel retains progress and allows retry`);
      running.mode.fail = false;
      await running.cancel.click();
      await running.progress.waitFor({ state: 'detached' });
      check(cancellationCalls(running) === 2 && running.mode.status === 'cancelled', `${runtime}: retry cancels running job once`);
      await finish(running);
    }
    const php = await fixture(browser, 'php');
    await php.cancel.click(); await php.progress.waitFor({ state: 'detached' });
    check(php.mode.status === 'cancelled', 'PHP: queued package installation can be cancelled');
    await finish(php);
    const claimed = await fixture(browser, 'php');
    claimed.mode.claim = true;
    await claimed.cancel.click();
    await claimed.page.getByRole('alert').filter({ hasText: 'Paket sudah berjalan' }).waitFor();
    await claimed.cancel.waitFor({ state: 'detached' });
    await claimed.page.getByText('Pemasangan paket sedang berjalan', { exact: false }).waitFor();
    check(claimed.mode.status === 'running', 'PHP: queued-to-running race preserves the active package operation');
    await finish(claimed);
    const runningPhp = await fixture(browser, 'php', 'running');
    check(await runningPhp.cancel.count() === 0, 'PHP: APT is not force-stopped while running');
    await finish(runningPhp);
    const won = await fixture(browser, 'python', 'running');
    won.mode.win = true;
    await won.cancel.click();
    await won.page.getByRole('button', { name: 'Hapus Python 3.15.0', exact: true }).waitFor();
    check(await won.page.getByText('Pemasangan Python dibatalkan', { exact: true }).count() === 0, 'Python: completion winning the race is reported as installed, never as cancelled');
    await finish(won);
    await fs.mkdir(path.join(__dirname, '../artifacts/runtime-cancel'), { recursive: true });
    await fs.writeFile(path.join(__dirname, '../artifacts/runtime-cancel/results.json'), JSON.stringify({ checks, note: 'API fixtures only' }, null, 2));
    console.log(`${checks} runtime cancellation checks passed.`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
