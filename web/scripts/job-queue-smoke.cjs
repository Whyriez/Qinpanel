/* Static UI + API/EventSource fixtures only. No panel daemons or package operations. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
const output = path.join(__dirname, '../artifacts/job-queue');
let checks = 0;
const check = (value, message) => { assert.ok(value, message); checks++; console.log(`ok ${checks}: ${message}`); };
const queuedMessage = 'Menunggu giliran eksekusi. Tugas belum dimulai.';
const item = (id, name, status, progress) => ({ id, name, status, progress, user_id: 1, payload_json: '{}', result_json: null, error_message: null, retry_count: 0, max_retries: 3, created_at: 1, updated_at: 1 });
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || 'msedge' });
  try {
    const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
    const errors = [], calls = [];
    page.on('pageerror', error => errors.push(error.message));
    let php = item('php-repo-fixture', 'php.repo.setup', 'queued', 0);
    let node = item('node-running-fixture', 'node.install', 'running', 15);
    const mode = { detailFail: false, hold: false, release: null };
    await page.addInitScript(() => {
      const streams = [];
      window.EventSource = class extends EventTarget {
        constructor(url) {
          super(); this.url = url; this.closed = false; streams.push(this);
          if (url.includes('/php-repo-fixture/')) setTimeout(() => this.send({ job_id: 'php-repo-fixture', status: 'queued', progress: 0, message: null, timestamp: 1 }), 50);
        }
        send(event) { if (!this.closed) this.dispatchEvent(new MessageEvent('progress', { data: JSON.stringify(event) })); }
        close() { this.closed = true; }
      };
      window.pushJobEvent = event => streams.filter(stream => stream.url.includes(`/${event.job_id}/`)).forEach(stream => stream.send(event));
    });
    await page.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url());
      calls.push({ path: url.pathname, method: req.method() });
      let status = 200, body = {};
      if (url.pathname === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (url.pathname.startsWith('/api/v1/jobs/') && url.pathname.endsWith('/logs')) body = [];
      else if (url.pathname === '/api/v1/jobs/php-repo-fixture') {
        body = { ...php };
        if (mode.hold) { mode.hold = false; await new Promise(resolve => { mode.release = resolve; }); }
        if (mode.detailFail) { status = 503; body = { error: 'Fixture temporarily unavailable' }; }
      } else if (url.pathname === '/api/v1/jobs') {
        const filter = url.searchParams.get('status');
        const items = [php, node].filter(job => !filter || job.status === filter);
        body = { items, total: items.length, page: 1, per_page: 100, total_pages: 1 };
      } else if (url.pathname === '/api/v1/php/versions') body = { ppa_configured: false, versions: [] };
      else if (url.pathname === '/api/v1/runtimes/node/versions') body = { versions: [], installed_count: 0, is_offline_fallback: false };
      else if (url.pathname.includes('notifications')) body = { notifications: [], count: 0, unread_count: 0, total: 0 };
      else { status = 404; body = { error: 'No fixture for route' }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const openMonitor = async () => {
      await page.goto(base + '/jobs', { waitUntil: 'domcontentloaded' });
      await page.getByRole('row').filter({ hasText: 'php.repo.setup' }).getByTitle('Monitor Progres Real-Time (SSE)').click();
      return page.getByRole('dialog', { name: 'Live Monitor: php.repo.setup' });
    };
    let dialog = await openMonitor();
    const queue = dialog.getByRole('status', { name: 'Status antrean tugas' });
    await queue.getByText('Menunggu tugas yang sedang berjalan:').waitFor();
    check(await queue.getByText('Tugas ini masih antre. Eksekusi belum dimulai.').isVisible(), 'queued job explains that execution has not started');
    check((await queue.innerText()).includes('node.install') && (await queue.innerText()).includes('15%'), 'monitor identifies the running blocker and actual progress');
    check(await dialog.getByText('0%', { exact: true }).isVisible(), 'queued progress stays at actual zero');
    await dialog.getByRole('log').getByText(queuedMessage, { exact: true }).waitFor();
    check(await dialog.getByRole('log').locator('[data-log-message]').count() === 1, 'null SSE snapshot does not duplicate the opening queue log');
    check(await dialog.getByRole('button', { name: 'Tutup monitor' }).isEnabled(), 'monitor remains closable while queued');

    node = { ...node, status: 'cancelled' };
    await queue.getByText('Menunggu tugas yang sedang berjalan:').waitFor({ state: 'hidden' });
    check(await queue.isVisible(), 'cancelled blocker disappears without inventing a running status for PHP');
    mode.detailFail = true;
    await dialog.getByRole('alert').filter({ hasText: 'Pembaruan status belum dapat dimuat' }).waitFor();
    check(await dialog.getByRole('log').isVisible(), 'polling failure keeps existing log and queue state');
    mode.detailFail = false;
    php = { ...php, status: 'running', progress: 10, updated_at: 2 };
    await dialog.getByText('10%', { exact: true }).waitFor();
    check(await queue.count() === 0, 'authoritative polling recovers a missed running SSE event');
    check(await dialog.getByRole('alert').count() === 0, 'successful polling clears transient status error');

    mode.hold = true;
    const deadline = Date.now() + 10000;
    while (!mode.release && Date.now() < deadline) await page.waitForTimeout(30);
    assert.ok(mode.release);
    await page.evaluate(() => window.pushJobEvent({ job_id: 'php-repo-fixture', status: 'running', progress: 40, message: 'Pembaruan baru dari SSE', timestamp: 3 }));
    await dialog.getByText('40%', { exact: true }).waitFor();
    const staleResponse = page.waitForResponse(res => new URL(res.url()).pathname === '/api/v1/jobs/php-repo-fixture');
    php = { ...php, progress: 40, updated_at: 3 };
    mode.release();
    await staleResponse;
    await page.waitForTimeout(100);
    check(await dialog.getByText('40%', { exact: true }).isVisible(), 'stale poll cannot overwrite a newer SSE event');
    php = { ...php, status: 'succeeded', progress: 100, updated_at: 4 };
    await dialog.getByText('100%', { exact: true }).waitFor();
    check(await dialog.getByRole('button', { name: 'Batalkan Tugas' }).count() === 0, 'terminal state is recovered without a final SSE event');
    await page.evaluate(() => window.pushJobEvent({ job_id: 'php-repo-fixture', status: 'queued', progress: 0, message: null, timestamp: 1 }));
    check(await dialog.getByText('100%', { exact: true }).isVisible(), 'late queued SSE snapshot cannot undo a terminal state');

    php = { ...php, status: 'queued', progress: 0, updated_at: 1 };
    await page.goto(base + '/runtime', { waitUntil: 'domcontentloaded' });
    const progress = page.getByRole('progressbar', { name: 'Progres tugas repositori PHP' });
    await progress.waitFor();
    check(await progress.getAttribute('value') === '0', 'repository card recovers queued job after navigation');
    check(await page.getByText('Menunggu giliran worker. Tugas belum dimulai.', { exact: true }).isVisible(), 'repository card explains waiting');
    check(await page.getByRole('button', { name: 'Konfigurasi repositori PHP', exact: true }).isDisabled(), 'repository setup stays blocked against duplicate submission');
    check(await page.getByRole('link', { name: 'Lihat progres', exact: true }).isVisible(), 'repository setup offers a progress link');

    await page.setViewportSize({ width: 390, height: 844 });
    node = { ...node, status: 'running' };
    dialog = await openMonitor();
    await dialog.getByRole('status', { name: 'Status antrean tugas' }).getByText('Menunggu tugas yang sedang berjalan:').waitFor();
    const panel = dialog.locator('.dialog-panel');
    await panel.evaluate(el => Promise.all(el.getAnimations().map(animation => animation.finished)));
    check(await panel.evaluate(el => el.scrollWidth <= el.clientWidth), 'mobile queue notice does not overflow the monitor');
    await page.screenshot({ path: path.join(output, 'queued-mobile.png') });
    await dialog.getByRole('button', { name: 'Tutup monitor' }).click();
    await dialog.waitFor({ state: 'hidden' });
    check(!calls.some(call => call.method !== 'GET'), 'queue monitoring never retries installs or cancels other jobs');
    check(errors.length === 0, 'no browser render errors');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors, calls }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
