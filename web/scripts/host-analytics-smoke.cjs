/* In-process API fixtures and a static frontend only; never starts panel daemons. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
const output = path.join(__dirname, '../artifacts/host-analytics');
let checks = 0;
function check(value, message) { assert.ok(value, message); checks++; console.log(`ok ${checks}: ${message}`); }
const point = timestamp => ({ timestamp, cpu_avg: 20, cpu_max: 40, memory_used_avg_bytes: 100, memory_used_max_bytes: 150, swap_used_avg_bytes: 0, load1_avg: 1, load1_max: 2, disk_used_avg_bytes: 1000, net_rx_avg_bytes_sec: 100, net_tx_avg_bytes_sec: 200, io_read_avg_bytes_sec: 0, io_write_avg_bytes_sec: 0 });
const insight = (id, severity, site_id) => ({ id, site_id, severity, insight_type: 'latency_regression', title: `Diagnosis ${id}`, description: `Rincian ${id}`, action: null, detected_at: 1700000000, metadata_json: null });
const retention = {
  retention: { raw_retention_secs: 7200, one_minute_retention_secs: 1209600, one_hour_retention_secs: 34560000, one_day_retention_secs: null, security_events_retention_secs: 604800, max_db_size_bytes: 104857600, auto_vacuum: true },
  stats: { db_size_bytes: 15728640, wal_size_bytes: 2097152, total_file_size_bytes: 17825792, page_size: 4096, page_count: 3840, freelist_count: 384, free_bytes: 1572864, fragmentation_ratio: 0.1, max_db_size_bytes: 104857600, is_healthy: true, integrity_message: 'ok', table_stats: [{ table_name: 'server_metrics_raw', row_count: 10 }] },
  last_maintenance: null,
};
async function fixture(browser, width = 1280, role = 'admin') {
  const page = await browser.newPage({ viewport: { width, height: 800 } });
  page.setDefaultTimeout(15000);
  const errors = [], calls = [];
  const mode = { points: [], insights: [], metricsFail: false, insightsFail: false, malformed: false, retentionMalformed: false, retentionFail: false, hold: false, release: null };
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/v1/**', async route => {
    const pathname = new URL(route.request().url()).pathname;
    let status = 200, body = {};
    if (pathname === '/api/v1/auth/me') body = { user: { id: 1, username: role, role }, csrf_token: 'fixture' };
    else if (pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (pathname === '/api/v1/sites') body = [];
    else if (pathname === '/api/v1/system/analytics/timeseries') {
      const points = mode.points;
      if (mode.hold) await new Promise(resolve => { mode.release = resolve; });
      status = mode.metricsFail ? 503 : 200;
      body = mode.metricsFail ? { detail: 'Pengumpulan metrik belum tersedia' } : { from_ts: 1, to_ts: 2, interval: '1m', points };
    } else if (pathname === '/api/v1/system/analytics/insights') {
      status = mode.insightsFail ? 503 : 200;
      // Exact Rust SystemInsightsResponse contract, including an empty server.
      body = mode.insightsFail ? { detail: 'Insight sementara tidak tersedia' } : mode.malformed ? {} : { evaluated_at: 1700000000, total_insights: mode.insights.length, insights: mode.insights };
    } else if (pathname === '/api/v1/system/analytics/retention') {
      status = mode.retentionFail ? 503 : 200;
      body = mode.retentionFail ? { detail: 'Retensi belum tersedia' } : mode.retentionMalformed ? {} : retention;
    }
    else if (pathname.includes('notifications')) body = { notifications: [], total: 0, count: 0, unread_count: 0 };
    else if (pathname === '/api/v1/jobs') body = { items: [], total: 0, page: 1, total_pages: 1 };
    else { status = 503; body = { detail: 'Retensi belum tersedia' }; }
    calls.push({ pathname, status, keys: Object.keys(body) });
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  return { page, errors, calls, mode };
}
async function openHost(f) {
  await f.page.goto(base + '/analytics', { waitUntil: 'domcontentloaded' });
  await f.page.getByRole('button', { name: 'Server Host (Admin)', exact: true }).click();
}
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, ...(process.env.NUSA_BROWSER_PATH ? { executablePath: process.env.NUSA_BROWSER_PATH } : process.platform === 'win32' ? { executablePath: 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe' } : {}) });
  try {
    if (process.argv.includes('--reproduce') || process.argv.includes('--reproduce-retention')) {
      const f = await fixture(browser);
      if (process.argv.includes('--reproduce-retention')) f.mode.insightsFail = true;
      const crash = f.page.waitForEvent('pageerror');
      await openHost(f);
      await crash;
      const evidence = { errors: f.errors, requests: f.calls.filter(call => call.pathname.includes('/system/analytics/')), rootText: await f.page.locator('#root').innerText() };
      const name = process.argv.includes('--reproduce-retention') ? 'before-retention' : 'before';
      await fs.writeFile(path.join(output, `${name}.json`), JSON.stringify(evidence, null, 2));
      await f.page.screenshot({ path: path.join(output, `${name}.png`) });
      console.log(JSON.stringify(evidence));
      return;
    }
    const empty = await fixture(browser);
    await openHost(empty);
    await empty.page.getByText('Belum ada data metrik server sistem yang tercatat.', { exact: true }).waitFor();
    await empty.page.getByText('Semua Situs Normal', { exact: true }).waitFor();
    check(empty.errors.length === 0, 'canonical empty insights response renders without crashing');
    check(await empty.page.getByRole('heading', { name: 'Observabilitas & Analitik' }).isVisible(), 'analytics shell stays visible');
    await empty.page.getByText('15.00 MB', { exact: true }).waitFor();
    check(true, 'canonical retention/stats payload renders storage size');
    await empty.page.close();

    for (const width of [1280, 390]) {
      const f = await fixture(browser, width);
      f.mode.points = [point(1700000000), point(1700000060)];
      f.mode.insights = [insight('critical-site', 'critical', 7), insight('warning-site', 'warning', 8), insight('global-info', 'info', null)];
      await openHost(f);
      await f.page.getByText('Diagnosis critical-site', { exact: true }).waitFor();
      await f.page.getByText('Diagnosis global-info', { exact: true }).waitFor();
      check(await f.page.getByText('1 Kritis', { exact: true }).isVisible(), 'critical count comes from API insights');
      check(await f.page.getByText('1 Peringatan', { exact: true }).isVisible(), 'warning count comes from API insights');
      check(await f.page.getByText('Situs #7', { exact: true }).isVisible(), 'insights retain site attribution without a loaded site name');
      check(await f.page.getByRole('img', { name: 'Metrik server timeseries' }).isVisible(), 'host metric chart renders');
      await f.page.getByRole('button', { name: 'Load Average', exact: true }).click();
      await f.page.getByRole('button', { name: 'Jaringan (RX / TX)', exact: true }).click();
      await f.page.getByRole('button', { name: '1 Jam Terakhir', exact: true }).click();
      await f.page.getByText('Diagnosis critical-site', { exact: true }).waitFor();
      await f.page.getByRole('button', { name: 'Perbarui', exact: true }).click();
      await f.page.getByText('Diagnosis critical-site', { exact: true }).waitFor();
      check(f.errors.length === 0, `${width}px host analytics interactions produce no page errors`);
      await f.page.screenshot({ path: path.join(output, `host-${width}.png`), fullPage: true });
      await f.page.close();
    }
    const unavailable = await fixture(browser);
    unavailable.mode.metricsFail = true;
    unavailable.mode.insightsFail = true;
    unavailable.mode.retentionFail = true;
    await openHost(unavailable);
    await unavailable.page.getByRole('alert').filter({ hasText: 'Pengumpulan metrik belum tersedia' }).waitFor();
    check(await unavailable.page.getByRole('heading', { name: 'Observabilitas & Analitik' }).isVisible(), 'API failure leaves the page usable');
    unavailable.mode.metricsFail = false;
    unavailable.mode.insightsFail = false;
    await unavailable.page.getByRole('button', { name: 'Perbarui', exact: true }).click();
    await unavailable.page.getByText('Semua Situs Normal', { exact: true }).waitFor();
    check(unavailable.errors.length === 0, 'retry recovers from API unavailability');
    await unavailable.page.close();

    const malformed = await fixture(browser);
    malformed.mode.malformed = true;
    await openHost(malformed);
    await malformed.page.getByRole('alert').filter({ hasText: 'Format respons insight sistem tidak valid' }).waitFor();
    await malformed.page.getByText('Belum ada data metrik server sistem yang tercatat.', { exact: true }).waitFor();
    check(malformed.errors.length === 0, 'invalid insights show a diagnostic while metrics remain available');
    await malformed.page.close();

    const badRetention = await fixture(browser);
    badRetention.mode.retentionMalformed = true;
    await openHost(badRetention);
    await badRetention.page.getByRole('alert').filter({ hasText: 'Format respons retensi analitik tidak valid' }).waitFor();
    await badRetention.page.getByText('Semua Situs Normal', { exact: true }).waitFor();
    check(badRetention.errors.length === 0, 'malformed retention response cannot blank the host page');
    await badRetention.page.close();

    const pending = await fixture(browser);
    pending.mode.hold = true;
    await openHost(pending);
    await pending.page.getByRole('status').filter({ hasText: 'Memuat metrik server' }).waitFor();
    check(await pending.page.getByRole('button', { name: 'Perbarui', exact: true }).isDisabled(), 'loading state is visible while the request is pending');
    pending.mode.hold = false;
    const deadline = Date.now() + 15000;
    while (!pending.mode.release && Date.now() < deadline) await pending.page.waitForTimeout(20);
    assert.ok(pending.mode.release);
    pending.mode.release();
    await pending.page.getByText('Semua Situs Normal', { exact: true }).waitFor();
    await pending.page.close();

    const race = await fixture(browser);
    race.mode.hold = true;
    await openHost(race);
    await race.page.getByRole('status').filter({ hasText: 'Memuat metrik server' }).waitFor();
    const raceDeadline = Date.now() + 15000;
    while (!race.mode.release && Date.now() < raceDeadline) await race.page.waitForTimeout(20);
    assert.ok(race.mode.release);
    race.mode.hold = false;
    race.mode.points = [point(1700000000)];
    await race.page.getByRole('button', { name: '1 Jam Terakhir', exact: true }).click();
    await race.page.getByRole('img', { name: 'Metrik server timeseries' }).waitFor();
    const oldResponse = race.page.waitForResponse(response => new URL(response.url()).pathname.endsWith('/timeseries'));
    race.mode.release();
    await oldResponse;
    await race.page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    check(await race.page.getByRole('img', { name: 'Metrik server timeseries' }).isVisible(), 'late old range response cannot replace the latest host metrics');
    await race.page.close();

    const tenant = await fixture(browser, 1280, 'user');
    await tenant.page.goto(base + '/analytics', { waitUntil: 'domcontentloaded' });
    await tenant.page.getByRole('heading', { name: 'Observabilitas & Analitik', exact: true }).waitFor();
    check(await tenant.page.getByRole('button', { name: 'Server Host (Admin)', exact: true }).count() === 0, 'tenant cannot open admin host analytics');
    check(!tenant.calls.some(call => call.pathname.startsWith('/api/v1/system/analytics/')), 'tenant view never requests host analytics');
    await tenant.page.close();
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors: [] }, null, 2));
    console.log(`${checks} host analytics browser checks passed`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
