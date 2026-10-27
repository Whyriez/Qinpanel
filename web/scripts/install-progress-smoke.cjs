/* Browser fixtures only: no package manager, API daemon, or agent daemon. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5193';
const output = path.join(__dirname, '../artifacts/install-progress');
const item = (id, name, payload) => ({ id, name, payload_json: JSON.stringify(payload), status: 'queued', progress: 0, user_id: 1, result_json: null, error_message: null, retry_count: 0, max_retries: 1, created_at: 1, updated_at: 1 });
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge' });
  const errors = [], posts = [];
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 980 } });
    page.setDefaultTimeout(15000);
    page.on('pageerror', error => errors.push(error.message));
    const php = item('php-log-fixture', 'php.install', { version: '8.4' });
    const jobs = new Map([[php.id, php]]);
    const logs = new Map([[php.id, [
      { sequence: 1, job_id: php.id, status: 'queued', progress: 0, timestamp: 1, message: 'Tugas masuk antrean.' },
    ]]]);
    let historyFail = false, inventoryFail = false, inventoryStale = false, holdList = false, releaseList, listRequests = 0;
    let installed = new Set();
    await page.addInitScript(() => {
      const streams = [];
      window.EventSource = class extends EventTarget {
        constructor(url) { super(); this.url = url; this.closed = false; streams.push(this); }
        close() { this.closed = true; }
        send(event) { if (!this.closed) this.dispatchEvent(new MessageEvent('progress', { data: JSON.stringify(event) })); }
      };
      window.pushJobEvent = event => streams.filter(stream => stream.url.includes(`/${event.job_id}/`)).forEach(stream => stream.send(event));
    });
    await page.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url()), name = url.pathname;
      let status = 200, body = {};
      if (name === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (name === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (name === '/api/v1/jobs') {
        if (url.searchParams.get('per_page') === '15') listRequests++;
        const filter = url.searchParams.get('status');
        const items = [...jobs.values()].filter(job => !filter || job.status === filter).map(job => ({ ...job }));
        body = { items, total: items.length, page: 1, per_page: 100, total_pages: 1 };
        if (holdList && url.searchParams.get('per_page') === '15') {
          holdList = false;
          await new Promise(resolve => { releaseList = resolve; });
        }
      } else if (/\/jobs\/[^/]+\/logs$/.test(name)) {
        if (historyFail) { status = 503; body = { error: 'Fixture log unavailable' }; }
        else body = (logs.get(name.split('/').at(-2)) || []).filter(event => event.sequence > Number(url.searchParams.get('after') || 0));
      } else if (/\/jobs\/[^/]+$/.test(name)) body = jobs.get(name.split('/').at(-1)) || {};
      else if (name === '/api/v1/databases/servers/install') {
        const { engine } = req.postDataJSON(); posts.push(engine);
        const attempt = posts.filter(value => value === engine).length;
        const job = item(`db-${engine}${attempt > 1 ? `-${attempt}` : ''}`, 'database.server.install', { engine });
        jobs.set(job.id, job);
        status = 202; body = { success: true, message: 'Instalasi dijadwalkan', job_id: job.id };
      } else if (name === '/api/v1/databases/servers') {
        if (inventoryFail) { status = 503; body = { error: 'Fixture inventory unavailable' }; }
        else body = ['mariadb', 'postgresql'].map(engine => ({ engine, is_installed: installed.has(engine) && !inventoryStale, is_running: installed.has(engine) && !inventoryStale, port: engine === 'mariadb' ? 3306 : 5432, version: null, socket_path: null, updated_at: 1 }));
      } else if (name === '/api/v1/php/versions') body = { ppa_configured: true, versions: [] };
      else if (name.includes('notifications')) body = { notifications: [], total: 0, unread_count: 0, count: 0 };
      else { status = 404; body = { error: 'No fixture' }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const push = async (sequence, message, progress, status = 'running') => {
      Object.assign(php, { status, progress, updated_at: sequence });
      const event = { sequence, job_id: php.id, status, progress, timestamp: sequence, message };
      logs.get(php.id).push(event);
      await page.evaluate(event => window.pushJobEvent(event), event);
    };
    await page.goto(`${base}/jobs`, { waitUntil: 'domcontentloaded', timeout: 60000 });
    let row = page.getByRole('row').filter({ hasText: php.id });
    await row.getByText('queued', { exact: true }).waitFor();
    Object.assign(php, { status: 'running', progress: 35, updated_at: 2 });
    const outputText = 'Get:1 php8.4-fpm\nUnpacking php8.4-fpm\n  detail indented\n\n' + Array.from({ length: 45 }, (_, index) => `Package diagnostic line ${index}`).join('\n');
    logs.get(php.id).push({ sequence: 2, job_id: php.id, status: 'running', progress: 35, timestamp: 2, message: outputText });
    await row.getByText('running', { exact: true }).waitFor();
    check(await row.getByText('35%', { exact: true }).isVisible(), 'jobs list refreshes without opening a monitor or refreshing the page');
    await row.getByTitle('Monitor Progres Real-Time (SSE)').click();
    let dialog = page.getByRole('dialog', { name: 'Live Monitor: php.install' });
    await dialog.getByRole('log').getByText(/Unpacking php8.4-fpm/).waitFor();
    check(true, 'opening a running job replays package output emitted earlier');
    await push(3, 'Setting up php8.4-fpm', 55);
    await dialog.getByRole('log').getByText('Setting up php8.4-fpm', { exact: true }).waitFor({ timeout: 2000 });
    check(true, 'SSE displays new installation output immediately');
    const repeated = { sequence: 4, job_id: php.id, status: 'running', progress: 55, timestamp: 3, message: 'Setting up php8.4-fpm' };
    logs.get(php.id).push(repeated);
    await page.evaluate(event => window.pushJobEvent(event), repeated);
    await page.waitForFunction(() => [...document.querySelectorAll('[role="log"] [data-log-message]')].filter(el => el.textContent === 'Setting up php8.4-fpm').length === 2, null, { timeout: 2000 });
    check(true, 'live output keeps equal lines with distinct sequence IDs');
    await dialog.locator('.dialog-panel').evaluate(el => Promise.all(el.getAnimations().map(animation => animation.finished)));
    const logRows = dialog.locator('.job-log-entry');
    const spacing = await logRows.evaluateAll(rows => {
      const a = rows.at(-2), b = rows.at(-1), style = getComputedStyle(b);
      return { gap: b.getBoundingClientRect().top - a.getBoundingClientRect().bottom, height: b.getBoundingClientRect().height, lineHeight: style.lineHeight, messageTop: b.querySelector('[data-log-message]').getBoundingClientRect().top, timeTop: b.querySelector('time').getBoundingClientRect().top };
    });
    check(spacing.gap <= 1 && Math.abs(spacing.height - 18) < 0.1 && spacing.lineHeight === '18px', `single-line events are consecutive 18px console rows without padding or blank gaps: ${JSON.stringify(spacing)}`);
    check(Math.abs(spacing.messageTop - spacing.timeTop) <= 4, 'timestamp, status and message share a line');
    check(await dialog.locator('[data-log-message]').filter({ hasText: 'Unpacking php8.4-fpm' }).textContent() === outputText, 'compact layout keeps original blank lines and indentation');
    holdList = true;
    for (let i = 0; i < 80 && !releaseList; i++) await pause(100);
    check(Boolean(releaseList), 'held a stale list response for race regression');
    const pendingCount = listRequests;
    await pause(3500);
    check(listRequests === pendingCount, 'slow list responses do not create overlapping polls or perpetual loading');
    await push(5, 'Memeriksa service FPM', 75);
    releaseList();
    await pause(300);
    check(await row.getByText('75%', { exact: true }).isVisible(), 'slow list response cannot rewind a newer live update');
    logs.get(php.id).push(...[6, 7].map(sequence => ({ sequence, job_id: php.id, status: 'running', progress: 75, timestamp: 5, message: 'Processing triggers for PHP' })));
    await page.waitForFunction(() => [...document.querySelectorAll('[role="log"] [data-log-message]')].filter(el => el.textContent === 'Processing triggers for PHP').length === 2);
    check(true, 'polling recovers missed lines and preserves identical output at one timestamp');
    const logView = dialog.getByRole('log');
    check(await logView.evaluate(el => el.scrollHeight - el.scrollTop - el.clientHeight < 3), 'monitor follows the latest log output automatically');
    await dialog.getByLabel('Ikuti log terbaru', { exact: true }).uncheck();
    await logView.evaluate(el => { el.scrollTop = 0; });
    await push(8, 'Tugas selesai.', 100, 'succeeded');
    await dialog.getByText('succeeded', { exact: true }).first().waitFor();
    check(await logView.evaluate(el => el.scrollTop === 0), 'pausing follow mode preserves the reader position when new output arrives');
    await page.keyboard.press('Escape');
    await row.getByTitle('Monitor Progres Real-Time (SSE)').click();
    dialog = page.getByRole('dialog', { name: 'Live Monitor: php.install' });
    await dialog.getByRole('log').getByText(/Unpacking php8.4-fpm/).waitFor();
    check(await dialog.getByRole('log').getByText('Processing triggers for PHP', { exact: true }).count() === 2, 'completed job retains full history when reopened');
    historyFail = true;
    await dialog.getByRole('alert').getByText(/Riwayat log belum/).waitFor();
    check(await dialog.getByRole('log').getByText(/Unpacking php8.4-fpm/).isVisible(), 'history outage retains existing output and shows retry status');
    historyFail = false;
    await dialog.getByRole('alert').waitFor({ state: 'hidden' });
    await page.screenshot({ path: path.join(output, 'jobs-monitor.png'), fullPage: true });
    await page.keyboard.press('Escape');

    if (process.env.QPANEL_MONITOR_ONLY === '1') {
      await page.setViewportSize({ width: 390, height: 740 });
      await row.getByTitle('Monitor Progres Real-Time (SSE)').click();
      dialog = page.getByRole('dialog', { name: 'Live Monitor: php.install' });
      await dialog.locator('[data-log-message]').filter({ hasText: 'Unpacking php8.4-fpm' }).waitFor();
      await dialog.locator('.dialog-panel').evaluate(el => Promise.all(el.getAnimations().map(animation => animation.finished)));
      check(await dialog.evaluate(el => el.scrollWidth <= el.clientWidth + 1), 'compact monitor fits the mobile viewport');
      check(await dialog.getByRole('log').evaluate(el => el.scrollHeight > el.clientHeight), 'full multi-line history still scrolls on mobile');
      await page.screenshot({ path: path.join(output, 'jobs-monitor-mobile.png') });
      await page.keyboard.press('Escape');
      await dialog.waitFor({ state: 'detached' });
      check(errors.length === 0, 'monitor has no browser runtime errors');
      await fs.writeFile(path.join(output, 'monitor-results.json'), JSON.stringify({ checks, errors }, null, 2));
      console.log(`${checks} monitor browser checks passed`);
      return;
    }

    const openDatabase = async () => {
      await page.goto(`${base}/runtime`, { waitUntil: 'domcontentloaded', timeout: 60000 });
      await page.getByRole('button', { name: 'Server Basis Data', exact: true }).click();
    };
    await openDatabase();
    await page.getByRole('button', { name: 'Pasang Server MariaDB', exact: true }).waitFor();
    await page.getByRole('button', { name: 'Pasang Server MariaDB', exact: true }).click();
    const mariaQueued = page.getByRole('button', { name: 'Instalasi MariaDB dalam antrean', exact: true });
    await mariaQueued.waitFor();
    check(await mariaQueued.isDisabled() && posts.filter(engine => engine === 'mariadb').length === 1, 'accepted MariaDB install stays blocked and shows queued status');
    await openDatabase();
    await mariaQueued.waitFor();
    check(await mariaQueued.isDisabled(), 'returning to the database tab recovers accepted installation');
    const maria = jobs.get('db-mariadb');
    Object.assign(maria, { status: 'running', progress: 15, updated_at: 2 });
    await page.getByRole('button', { name: 'Memasang MariaDB…', exact: true }).waitFor();
    check(true, 'database card follows running job without manual refresh');
    inventoryFail = true;
    Object.assign(maria, { status: 'succeeded', progress: 100, updated_at: 3 });
    const verifying = page.getByRole('button', { name: 'Memeriksa hasil instalasi MariaDB', exact: true });
    await verifying.waitFor();
    check(await verifying.isDisabled(), 'failed inventory refresh cannot re-enable Install after success');
    inventoryFail = false; inventoryStale = true;
    installed.add('mariadb');
    await pause(3500);
    check(await verifying.isDisabled(), 'stale inventory remains blocked until installation is confirmed');
    inventoryStale = false;
    await page.getByText('Terpasang', { exact: true }).waitFor();
    check(await page.getByRole('button', { name: /Pasang Server MariaDB/ }).count() === 0, 'confirmed installation replaces Install with service controls');
    await page.getByRole('button', { name: 'Pasang Server PostgreSQL', exact: true }).click();
    await page.getByRole('button', { name: 'Instalasi PostgreSQL dalam antrean', exact: true }).waitFor();
    check(posts.filter(engine => engine === 'postgresql').length === 1, 'PostgreSQL uses the same persistent installation tracking');
    Object.assign(jobs.get('db-postgresql'), { status: 'failed', error_message: 'E: fixture package failure', updated_at: 4 });
    await page.getByRole('button', { name: 'Pasang Server PostgreSQL', exact: true }).waitFor();
    check(true, 'failed installation exposes a retry after the job ends');
    await page.getByRole('button', { name: 'Pasang Server PostgreSQL', exact: true }).click();
    await page.getByRole('button', { name: 'Instalasi PostgreSQL dalam antrean', exact: true }).waitFor();
    check(posts.filter(engine => engine === 'postgresql').length === 2, 'explicit retry enqueues exactly one new attempt');
    await page.emulateMedia({ colorScheme: 'dark' });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.getByRole('button', { name: 'Instalasi PostgreSQL dalam antrean', exact: true }).scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(output, 'database-mobile.png'), fullPage: true });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'database progress fits mobile viewport');
    await page.getByRole('link', { name: 'Lihat progres', exact: true }).last().click();
    await page.getByRole('dialog', { name: 'Live Monitor: database.server.install', exact: true }).waitFor();
    check(new URL(page.url()).searchParams.get('job') === 'db-postgresql-2', 'database progress link opens the corresponding job monitor directly');
    check(errors.length === 0, 'no browser runtime errors');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors, posts }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
