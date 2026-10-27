/* Browser regression with API fixtures only. Never starts panel services or installs packages. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
const output = path.join(__dirname, '../artifacts/runtime-install');
let checks = 0;
const check = (value, message) => { assert.ok(value, message); checks++; console.log(`ok ${checks}: ${message}`); };
const longError = 'Instalasi PHP 8.4 gagal:\n' + Array.from({ length: 64 }, (_, i) => `E: Unable to locate package php8.4-extension-${i}`).join('\n') + '\n' + 'unbroken-diagnostic-'.repeat(48) + '\nAKHIR_DIAGNOSTIK_LENGKAP';
const job = (id, name, status = 'queued') => ({ id, name, user_id: 1, status, progress: status === 'succeeded' ? 100 : 35, payload_json: JSON.stringify({ version: name === 'node.install' ? '24.20.0' : '8.4' }), result_json: null, error_message: status === 'failed' ? longError : null, retry_count: 0, max_retries: 3, created_at: 1, updated_at: 1 });
async function fixture(browser, width = 1280) {
  const page = await browser.newPage({ viewport: { width, height: 800 } });
  page.setDefaultTimeout(20000);
  const calls = [], errors = [];
  const mode = { node: 'queued', php: 'queued', phpEnqueued: false, enqueued: false, hold: false, fail: false, jobsFail: false, noise: 0, release: null, inventoryHold: false, inventoryFail: false, inventoryStale: false, inventoryRelease: null };
  page.on('pageerror', e => errors.push(e.message));
  await page.route('**/api/v1/**', async route => {
    const req = route.request(), name = new URL(req.url()).pathname;
    calls.push({ name, query: new URL(req.url()).search, method: req.method(), body: req.postDataJSON() });
    const completedInventory = (name === '/api/v1/php/versions' && mode.php === 'succeeded') ||
      (name === '/api/v1/runtimes/node/versions' && mode.node === 'succeeded');
    if (completedInventory && mode.inventoryHold) await new Promise(resolve => { mode.inventoryRelease = resolve; });
    if (completedInventory && mode.inventoryFail) {
      return route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: 'Inventaris runtime sementara tidak tersedia' }) });
    }
    let status = 200, body = {};
    if (name === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
    else if (name === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (name === '/api/v1/php/versions') body = { ppa_configured: true, versions: [{ version: '8.4', installed: mode.php === 'succeeded' && !mode.inventoryStale, status: 'not_installed', fpm_binary_exists: false, cli_binary_exists: false, service_masked: false }] };
    else if (name === '/api/v1/php/versions/8.4/install') { mode.phpEnqueued = true; status = 202; body = job('php-fixture', 'php.install'); }
    else if (name === '/api/v1/runtimes/node/versions') body = { mirror_url: 'https://nodejs.org/dist', installed_count: mode.node === 'succeeded' && !mode.inventoryStale ? 1 : 0, is_offline_fallback: false, versions: [{ version: '24.20.0', raw_version: 'v24.20.0', is_installed: mode.node === 'succeeded' && !mode.inventoryStale, installed_path: null, is_lts: true, lts_codename: 'Krypton', corepack_enabled: false, used_by_sites_count: 0, used_by_domains: [], can_delete: true }] };
    else if (name === '/api/v1/runtimes/node/install') {
      if (mode.hold) await new Promise(resolve => { mode.release = resolve; });
      status = mode.fail ? 503 : 202;
      mode.enqueued = !mode.fail;
      body = mode.fail ? { error: 'Antrean tugas sementara tidak tersedia' } : job('node-fixture', 'node.install');
    } else if (name.startsWith('/api/v1/jobs/') && name.endsWith('/logs')) body = [];
    else if (name === '/api/v1/jobs/node-fixture') body = job('node-fixture', 'node.install', mode.node);
    else if (name === '/api/v1/jobs/php-fixture') body = job('php-fixture', 'php.install', mode.php);
    else if (name === '/api/v1/jobs') {
      if (mode.jobsFail) { status = 503; body = { error: 'Status antrean tidak tersedia' }; }
      else {
        const params = new URL(req.url()).searchParams;
        const filter = params.get('status'), currentPage = Number(params.get('page') || 1), perPage = Number(params.get('per_page') || 20);
        const items = [...Array.from({ length: mode.noise }, (_, index) => job(`other-${index}`, 'system.echo')), job('php-failed', 'php.install', 'failed'), ...(mode.enqueued ? [job('node-fixture', 'node.install', mode.node)] : []), ...(mode.phpEnqueued ? [job('php-fixture', 'php.install', mode.php)] : [])].filter(item => !filter || item.status === filter);
        body = { items: items.slice((currentPage - 1) * perPage, currentPage * perPage), total: items.length, page: currentPage, per_page: perPage, total_pages: Math.max(1, Math.ceil(items.length / perPage)) };
      }
    }
    else if (name.endsWith('/events')) {
      const node = name.includes('/node-fixture/');
      return route.fulfill({ contentType: 'text/event-stream', body: 'event: progress\ndata: ' + JSON.stringify({ job_id: node ? 'node-fixture' : 'php-failed', status: node ? mode.node : 'failed', progress: 35, message: node ? 'Pemasangan Node.js sedang berlangsung' : longError, timestamp: 1 }) + '\n\n' });
    } else if (name.includes('unread-count')) body = { count: 0, unread_count: 0 };
    else if (name.includes('notifications')) body = { notifications: [], total: 0 };
    else { status = 404; body = { error: 'No API fixture for this route' }; }
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  return { page, calls, mode, errors };
}
async function openNode(page) {
  await page.goto(base + '/runtime', { waitUntil: 'domcontentloaded', timeout: 30000 });
  await page.getByRole('button', { name: /^Node\.js/ }).click();
  await page.getByRole('button', { name: 'Pasang Node.js 24.20.0', exact: true }).waitFor();
}
async function monitor(browser, width) {
  const f = await fixture(browser, width), p = f.page;
  await p.goto(base + '/jobs', { waitUntil: 'domcontentloaded', timeout: 30000 });
  await Promise.all([
    p.waitForResponse(response => new URL(response.url()).pathname.endsWith('/php-failed/events')),
    p.getByTitle('Monitor Progres Real-Time (SSE)').click(),
  ]);
  const dialog = p.getByRole('dialog', { name: 'Live Monitor: php.install' });
  const log = dialog.getByRole('log');
  await log.getByText('AKHIR_DIAGNOSTIK_LENGKAP', { exact: false }).first().waitFor();
  await dialog.locator('.dialog-panel').evaluate(element => Promise.all(element.getAnimations().map(animation => animation.finished)));
  check((await log.innerText()).includes(longError), 'stored and streamed error includes the entire original text');
  check(await log.locator('[data-log-message]').filter({ hasText: 'AKHIR_DIAGNOSTIK_LENGKAP' }).count() === 1, 'initial SSE snapshot does not duplicate the stored error');
  const dimensions = await dialog.evaluate(element => {
    const panel = element.querySelector('.dialog-panel'), body = element.querySelector('[role=log]'), header = element.querySelector('.dialog-header');
    const paragraph = element.querySelector('[role=log] [data-log-message]'), box = panel.getBoundingClientRect();
    return { fits: box.left >= 0 && box.right <= innerWidth && box.top >= 0 && box.bottom <= innerHeight, overflow: panel.scrollWidth > panel.clientWidth, ellipsis: getComputedStyle(paragraph).textOverflow, whiteSpace: getComputedStyle(paragraph).whiteSpace, scrolls: body.scrollHeight > body.clientHeight, headerY: header.getBoundingClientRect().y };
  });
  check(dimensions.fits && !dimensions.overflow, `${width}px monitor stays inside the viewport`);
  check(dimensions.ellipsis !== 'ellipsis' && dimensions.whiteSpace === 'pre-wrap', 'long lines wrap with original newlines and no ellipsis');
  check(dimensions.scrolls, 'long diagnostics scroll within the log viewport');
  await dialog.getByRole('log').evaluate(el => { el.scrollTop = el.scrollHeight; });
  check(Math.abs((await dialog.locator('.dialog-header').boundingBox()).y - dimensions.headerY) < 1, 'monitor header remains visible while reading the end of a log');
  await p.screenshot({ path: path.join(output, `monitor-${width}.png`) });
  await dialog.getByRole('button', { name: 'Tutup monitor' }).click();
  await dialog.waitFor({ state: 'detached' });
  check(f.errors.length === 0, f.errors.join('\n'));
  await p.close();
}
async function completionInventory(browser, runtime) {
  const f = await fixture(browser, runtime === 'node' ? 390 : 1280), p = f.page;
  f.mode[runtime] = 'running';
  f.mode[runtime === 'node' ? 'enqueued' : 'phpEnqueued'] = true;
  await p.goto(base + '/runtime', { waitUntil: 'domcontentloaded' });
  if (runtime === 'node') await p.getByRole('button', { name: /^Node\.js/ }).click();
  const label = runtime === 'node' ? 'Node.js v24.20.0' : 'PHP 8.4';
  const progress = p.getByRole('progressbar', { name: `Progres tugas ${label}` });
  await progress.waitFor();
  await progress.scrollIntoViewIfNeeded();
  check(await progress.getAttribute('value') === '35', `${label} shows actual job progress`);
  check(await p.getByRole('link', { name: 'Lihat progres', exact: true }).isVisible(), `${label} offers a task-monitor link`);
  const progressBox = await progress.boundingBox();
  check(progressBox.x >= 0 && progressBox.x + progressBox.width <= p.viewportSize().width, `${label} progress stays inside the viewport`);
  await p.screenshot({ path: path.join(output, `progress-${runtime}.png`) });

  f.mode.inventoryHold = true;
  f.mode[runtime] = 'succeeded';
  const updating = p.getByRole('button', { name: runtime === 'node' ? 'Pasang Node.js 24.20.0' : 'Memperbarui daftar', exact: true });
  await updating.filter({ hasText: 'Memperbarui daftar' }).waitFor();
  check(await updating.isDisabled(), `${label} stays blocked while successful inventory refresh is pending`);
  check(await p.getByRole('button', { name: 'Pasang', exact: true }).count() === 0, `${label} never exposes Install between job success and inventory refresh`);
  await p.waitForFunction(() => document.querySelector('progress')?.value === 100);
  const deadline = Date.now() + 20000;
  while (!f.mode.inventoryRelease && Date.now() < deadline) await p.waitForTimeout(20);
  assert.ok(f.mode.inventoryRelease, 'successful job starts inventory refresh');
  f.mode.inventoryHold = false;
  f.mode.inventoryFail = true;
  f.mode.inventoryRelease();
  await p.getByText('Inventaris runtime sementara tidak tersedia', { exact: true }).waitFor();
  check(await updating.isDisabled(), `${label} stays blocked after inventory fetch fails`);

  f.mode.inventoryFail = false;
  f.mode.inventoryStale = true;
  const inventoryPath = runtime === 'node' ? '/api/v1/runtimes/node/versions' : '/api/v1/php/versions';
  await p.waitForResponse(response => new URL(response.url()).pathname === inventoryPath && response.status() === 200);
  check(await updating.isDisabled(), `${label} stays blocked while a successful inventory response still reports not installed`);
  f.mode.inventoryStale = false;
  await updating.waitFor({ state: 'detached' });
  await p.getByText('Terpasang', { exact: true }).waitFor();
  check(await p.getByRole('button', { name: 'Pasang', exact: true }).count() === 0, `${label} becomes installed without reopening Install`);
  check(!f.calls.some(call => call.method === 'POST' && call.name.endsWith('/install')), `${label} inventory recovery never submits a second install`);
  check(f.errors.length === 0, f.errors.join('\n'));
  await p.close();
}
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, ...(process.env.NUSA_BROWSER_PATH ? { executablePath: process.env.NUSA_BROWSER_PATH } : process.platform === 'win32' ? { executablePath: 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe' } : {}) });
  try {
    const f = await fixture(browser), p = f.page;
    await openNode(p);
    f.mode.hold = true;
    const directInstall = p.getByRole('button', { name: 'Pasang Node.js 24.20.0', exact: true });
    await directInstall.click();
    check(await directInstall.isDisabled(), 'direct installation blocks duplicate submission while enqueue is pending');
    check(await p.getByRole('dialog').count() === 0, 'installing Node.js opens no extra dialog');
    await p.getByRole('navigation', { name: 'Navigasi runtime' }).getByRole('button', { name: /^PHP/ }).click();
    await p.getByRole('button', { name: /^Node\.js/ }).click();
    check(await directInstall.isDisabled(), 'changing tabs during enqueue keeps installation blocked');
    await assert.doesNotReject(async () => {
      const deadline = Date.now() + 20000;
      while (!f.mode.release && Date.now() < deadline) await p.waitForTimeout(20);
      assert.ok(f.mode.release, 'pending enqueue reaches the API fixture');
    });
    f.mode.release();
    await p.getByText('Tugas pemasangan Node.js v24.20.0 telah dibuat', { exact: true }).waitFor();
    check(await p.getByRole('dialog').count() === 0, 'accepted enqueue remains inline');
    check(!await p.getByText('Node.js v24.20.0 berhasil dipasang', { exact: true }).count(), 'enqueue does not claim install succeeded');
    f.mode.node = 'succeeded';
    await p.getByText('Pemasangan Node.js selesai', { exact: true }).waitFor();
    check(f.calls.filter(c => c.name === '/api/v1/runtimes/node/install').length === 1, 'background polling never resubmits installation');
    check(f.calls.filter(c => c.name === '/api/v1/runtimes/node/versions').length >= 2, 'runtime inventory refreshes when job completes');
    check(f.calls.find(c => c.name === '/api/v1/runtimes/node/install').body.version === '24.20.0', 'chosen version preserved');
    check(f.calls.find(c => c.name === '/api/v1/runtimes/node/install').body.enable_corepack === true, 'direct installation preserves the default Corepack option');
    check(f.errors.length === 0, f.errors.join('\n'));
    await p.close();

    const failure = await fixture(browser);
    failure.mode.fail = true;
    await openNode(failure.page);
    await failure.page.getByRole('button', { name: 'Pasang Node.js 24.20.0', exact: true }).click();
    await failure.page.getByText('Antrean tugas sementara tidak tersedia', { exact: false }).first().waitFor();
    check(await failure.page.getByRole('button', { name: 'Pasang Node.js 24.20.0', exact: true }).isEnabled(), 'enqueue failure is readable and allows direct retry');
    check(failure.errors.length === 0, failure.errors.join('\n'));
    await failure.page.close();

    const accepted = await fixture(browser);
    await openNode(accepted.page);
    await accepted.page.getByRole('button', { name: 'Pasang Node.js 24.20.0', exact: true }).click();
    await accepted.page.getByRole('progressbar', { name: 'Progres tugas Node.js v24.20.0' }).waitFor();
    check(await accepted.page.getByRole('dialog').count() === 0, 'accepted job shows progress directly in the runtime list');
    check(await accepted.page.getByRole('button', { name: 'Pasang Node.js 24.20.0', exact: true }).isDisabled(), 'accepted installation remains busy while queued');
    await accepted.page.goto(base + '/jobs', { waitUntil: 'domcontentloaded' });
    await accepted.page.locator('tbody').getByText('node.install', { exact: true }).waitFor();
    check(true, 'queued installation stays visible after navigating away from Runtime');
    await accepted.page.reload({ waitUntil: 'domcontentloaded' });
    await accepted.page.locator('tbody').getByText('node.install', { exact: true }).waitFor();
    check(accepted.calls.filter(c => c.name === '/api/v1/runtimes/node/install').length === 1, 'refresh reads persisted job without resubmitting or cancelling the install');
    accepted.mode.noise = 105;
    await accepted.page.goto(base + '/runtime', { waitUntil: 'domcontentloaded' });
    await accepted.page.getByRole('button', { name: /^Node\.js/ }).click();
    await accepted.page.getByRole('progressbar', { name: 'Progres tugas Node.js v24.20.0' }).waitFor();
    check(await accepted.page.getByRole('button', { name: 'Pasang Node.js 24.20.0', exact: true }).isDisabled(), 'returning to Runtime restores the active job and blocks duplicate install');
    check(accepted.calls.some(call => call.name === '/api/v1/jobs' && new URLSearchParams(call.query).get('page') === '2'), 'active install is recovered beyond the first 100 queued jobs');
    accepted.mode.node = 'running';
    await accepted.page.getByRole('status').filter({ hasText: 'Sedang dipasang' }).waitFor();
    await accepted.page.reload({ waitUntil: 'domcontentloaded' });
    await accepted.page.getByRole('button', { name: /^Node\.js/ }).click();
    await accepted.page.getByRole('status').filter({ hasText: 'Sedang dipasang' }).waitFor();
    check(await accepted.page.getByRole('button', { name: 'Pasang Node.js 24.20.0', exact: true }).isDisabled(), 'refresh restores a running install with visible progress');
    check(await accepted.page.getByRole('dialog').count() === 0, 'running installation stays inline');
    accepted.mode.node = 'succeeded';
    await accepted.page.getByText('Pemasangan Node.js selesai', { exact: true }).waitFor();
    await accepted.page.getByText('Terpasang', { exact: true }).waitFor();
    check(accepted.calls.filter(c => c.name === '/api/v1/runtimes/node/install').length === 1, 'completion after refresh updates inventory without a second installation');
    await accepted.page.close();

    const php = await fixture(browser);
    await php.page.goto(base + '/runtime', { waitUntil: 'domcontentloaded' });
    await php.page.getByRole('button', { name: 'Pasang', exact: true }).click();
    await php.page.getByRole('button', { name: 'Sedang diproses', exact: true }).waitFor();
    check(await php.page.getByRole('button', { name: 'Sedang diproses', exact: true }).isDisabled(), 'PHP install stays busy after enqueue returns');
    await php.page.reload({ waitUntil: 'domcontentloaded' });
    await php.page.getByRole('button', { name: 'Sedang diproses', exact: true }).waitFor();
    check(await php.page.getByRole('button', { name: 'Sedang diproses', exact: true }).isDisabled(), 'PHP queued state survives refresh');
    php.mode.php = 'failed';
    await php.page.getByText('Tugas PHP gagal', { exact: true }).waitFor();
    check(await php.page.getByRole('button', { name: 'Pasang', exact: true }).isEnabled(), 'failed PHP install unlocks retry');
    check(php.calls.filter(c => c.name === '/api/v1/php/versions/8.4/install').length === 1, 'PHP polling never installs again');
    await php.page.close();

    const unavailable = await fixture(browser);
    unavailable.mode.jobsFail = true;
    await unavailable.page.goto(base + '/runtime', { waitUntil: 'domcontentloaded' });
    await unavailable.page.getByRole('alert').filter({ hasText: 'Status tugas runtime gagal dimuat' }).waitFor();
    check(await unavailable.page.getByRole('button', { name: 'Pasang', exact: true }).isDisabled(), 'unknown queue status blocks duplicate install and explains the failure');
    unavailable.mode.jobsFail = false;
    await unavailable.page.getByRole('button', { name: 'Pasang', exact: true }).waitFor({ state: 'visible' });
    await unavailable.page.waitForFunction(() => [...document.querySelectorAll('button')].some(button => button.textContent.trim() === 'Pasang' && !button.disabled));
    check(true, 'queue polling recovers after a temporary status failure');
    await unavailable.page.close();

    const failedJob = await fixture(browser);
    await openNode(failedJob.page);
    await failedJob.page.getByRole('button', { name: 'Pasang Node.js 24.20.0', exact: true }).click();
    await failedJob.page.getByRole('progressbar', { name: 'Progres tugas Node.js v24.20.0' }).waitFor();
    failedJob.mode.node = 'failed';
    await failedJob.page.getByText('Pemasangan Node.js gagal', { exact: true }).waitFor();
    check(failedJob.calls.filter(c => c.name === '/api/v1/runtimes/node/install').length === 1, 'worker failure is reported without retrying the HTTP install request');
    check(failedJob.errors.length === 0, failedJob.errors.join('\n'));
    await failedJob.page.close();

    await completionInventory(browser, 'php');
    await completionInventory(browser, 'node');
    await monitor(browser, 1280);
    await monitor(browser, 390);
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors: [] }, null, 2));
    console.log(`${checks} runtime installation browser checks passed`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
