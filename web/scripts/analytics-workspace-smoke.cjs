/* Production frontend, local intercepted APIs only. No native services. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const http = require('node:http');
const frontendBase = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5206';
const output = path.join(__dirname, '../artifacts/analytics-workspace');
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1050 } });
  const page = await context.newPage();
  const csvBytes = 'timestamp,requests\n1,12\n';
  const downloadFixture = http.createServer(async (request, response) => {
    if (/^\/api\/v1\/sites\/\d+\/analytics\/export\?/.test(request.url)) {
      response.writeHead(200, { 'Content-Type': 'text/csv', 'Content-Disposition': 'attachment; filename=analytics.csv', 'Content-Length': Buffer.byteLength(csvBytes) }); response.end(csvBytes); return;
    }
    try {
      const upstream = await fetch(frontendBase + request.url);
      response.writeHead(upstream.status, { 'Content-Type': upstream.headers.get('content-type') || 'application/octet-stream' });
      response.end(Buffer.from(await upstream.arrayBuffer()));
    } catch { response.writeHead(502).end(); }
  });
  await new Promise(resolve => downloadFixture.listen(0, '127.0.0.1', resolve));
  const base = `http://127.0.0.1:${downloadFixture.address().port}`;
  await fs.mkdir(output, { recursive: true });
  const missing = [], errors = [], requests = [], mutations = [];
  let role = 'user', fail = false, failSites = false, emptySites = false, noSamples = false, delay = false, optionalFail = false, failServer = false, checks = 0;
  const check = (condition, label) => { assert.ok(condition, label); console.log(`ok ${++checks}: ${label}`); };
  page.on('pageerror', error => errors.push(error.message));
  const now = Math.floor(Date.now() / 1000);
  const sites = [7, 8].map(id => ({ id, user_id: 2, primary_domain: id === 7 ? 'store.example.test' : 'api.example.test', kind: id === 7 ? 'php' : 'nodejs', docroot: `/home/alice/site${id}`, status: 'active', domains: [], spa_fallback: false, force_https: true, created_at: 1, updated_at: 1 }));
  const points = Array.from({ length: 48 }, (_, index) => ({ timestamp: now - 86400 + index * 1800, requests: Math.round(95 + Math.sin(index / 4) * 55 + index * 2), bytes: 1048576 + index * 4096, status_2xx: 80, status_3xx: 10, status_4xx: index % 9 === 0 ? 8 : 0, status_5xx: index % 15 === 0 ? 2 : 0, visitors: 80, cache_hit: 65, cache_miss: 30, latency_p50_ms: 25 + index % 9, latency_p95_ms: 70 + index % 13 * 2, latency_p99_ms: 120 + index % 6 * 9, latency_avg_ms: 40 }));
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), url = new URL(request.url()), p = url.pathname;
    requests.push(url);
    if (request.method() !== 'GET') mutations.push(p);
    let body = {}, status = 200;
    if (p === '/api/v1/auth/me') body = { user: { id: role === 'admin' ? 1 : 2, username: 'alice', role, status: 'active' }, csrf_token: 'fixture' };
    else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (p === '/api/v1/notifications/unread-count') body = { unread_count: 0 };
    else if (p === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 0, page: 1, per_page: 6 };
    else if (p === '/api/v1/sites') { body = emptySites ? [] : sites; if (failSites) { status = 503; body = { error: 'Fixture site list unavailable' }; } }
    else if (/^\/api\/v1\/sites\/\d+$/.test(p)) body = sites.find(site => site.id === Number(p.split('/').at(-1)));
    else if (p.endsWith('/ssl')) body = { certificate: null, force_https: true };
    else if (p.endsWith('/deployments')) body = [{ id: 1, site_id: 7, release_id: 'rel123abcdef', branch: 'main', status: 'active', trigger_type: 'manual', is_current: true, created_at: points[20].timestamp }];
    else if (p.includes('/system/analytics/')) {
      assert.equal(role, 'admin', 'Tenant must not request host metrics or retention');
      if (p.endsWith('/timeseries')) {
        body = { from_ts: now - 86400, to_ts: now, interval: '1h', points: noSamples ? [] : points.map((point, index) => ({ timestamp: point.timestamp, cpu_avg: 20 + index % 8, cpu_max: 45, memory_used_avg_bytes: 2 * 1073741824, memory_used_max_bytes: 3 * 1073741824, swap_used_avg_bytes: 0, load1_avg: .8, load1_max: 1.5, disk_used_avg_bytes: 18 * 1073741824, net_rx_avg_bytes_sec: 4096 + index * 1024, net_tx_avg_bytes_sec: 8192, io_read_avg_bytes_sec: 2048, io_write_avg_bytes_sec: 4096 })) };
        if (failServer) { status = 503; body = { error: 'Fixture server metrics unavailable' }; }
      } else if (p.endsWith('/insights')) body = { evaluated_at: now, total_insights: 1, insights: [{ id: 'warning', site_id: 7, insight_type: 'latency_regression', severity: 'warning', title: 'Latensi meningkat', description: 'Fixture warning', detected_at: now }] };
      else if (p.endsWith('/retention')) body = { retention: { raw_retention_secs: 7200, one_minute_retention_secs: 1209600, one_hour_retention_secs: 34560000, one_day_retention_secs: null, security_events_retention_secs: 604800, max_db_size_bytes: 104857600, auto_vacuum: true }, stats: { db_size_bytes: 15728640, wal_size_bytes: 2097152, total_file_size_bytes: 17825792, page_size: 4096, page_count: 3840, freelist_count: 384, free_bytes: 1572864, fragmentation_ratio: .1, max_db_size_bytes: 104857600, is_healthy: true, integrity_message: 'ok', table_stats: [] }, last_maintenance: null };
      else { missing.push(p); status = 404; }
    } else if (p.includes('/analytics/')) {
      const siteId = Number(p.split('/')[4]);
      const from_ts = Number(url.searchParams.get('from') || now - 86400), to_ts = Number(url.searchParams.get('to') || now);
      if (delay && siteId === 7) await new Promise(resolve => setTimeout(resolve, 900));
      if (p.endsWith('/overview') && !p.endsWith('/security/overview')) {
        body = { site_id: siteId, from_ts, to_ts, interval_used: '1h', current: { total_requests: noSamples ? 0 : siteId === 7 ? 12450 : 880, total_bytes: noSamples ? 0 : 805306368, unique_visitors: noSamples ? 0 : 4180, status_2xx: 12000, status_3xx: 350, status_4xx: 90, status_5xx: 10, cache_hit_ratio: .82, latency_avg_ms: 40, latency_p50_ms: 28, latency_p95_ms: 96, latency_p99_ms: 170, error_rate_percent: .8 }, comparison: url.searchParams.get('compare') === 'false' ? null : { requests_change_percent: 12.4, bytes_change_percent: 8.2, visitors_change_percent: -2.1, avg_latency_change_percent: 4, error_rate_change_percent: -.3 } };
        if (fail) { status = 503; body = { error: 'Fixture analytics unavailable' }; }
      } else if (p.endsWith('/timeseries')) body = { site_id: siteId, from_ts, to_ts, interval: '1h', points: noSamples ? [] : points };
      else if (p.endsWith('/breakdowns')) body = { site_id: siteId, from_ts, to_ts, paths: [{ name: '/products', count: 5000, percentage: 40.2 }, { name: '/checkout', count: 1200, percentage: 9.6 }], referrers: [{ name: 'google.com', count: 800, percentage: 64.8 }], countries: [{ name: 'Indonesia', count: 1250, percentage: 80 }], browsers: [], os: [], devices: [], slow_urls: [], error_urls: [] };
      else if (p.endsWith('/app')) { body = { site_id: siteId, runtime: siteId === 7 ? 'php' : 'nodejs', php_fpm: null, app_process: null }; if (optionalFail) { status = 503; body = { error: 'Fixture runtime unavailable' }; } }
      else if (p.endsWith('/slowlog')) body = { events: [] };
      else if (p.endsWith('/insights')) body = { site_id: siteId, site_name: 'Fixture', insights: [], evaluated_at: now };
      else if (p.endsWith('/security/overview')) body = { site_id: siteId, from: from_ts, to: to_ts, total_probing_requests: 0, total_brute_force_attempts: 0, unique_suspicious_ips: 0, top_ips: [], probing_paths: [], brute_force_incidents: [], active_banned_ips_count: 0 };
      else if (p.endsWith('/export')) { await route.continue(); return; }
      else { missing.push(p); status = 404; }
    } else if (p.endsWith('/security/banned-ips')) body = [];
    else { missing.push(p); status = 404; body = { error: 'Unhandled fixture' }; }
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  const metric = () => page.locator('.analytics-metric-value').first();
  const ready = () => metric().waitFor();
  const mainTop = async () => page.locator('main').evaluate(element => { element.scrollTop = 0; });
  try {
    await page.goto(`${base}/analytics`); await ready();
    check(await page.getByRole('heading', { name: 'Analitik', exact: true }).isVisible(), 'Clear analytics page title');
    check(await page.getByRole('group', { name: 'Lingkup analitik' }).count() === 0, 'Tenant has no host controls');
    check(await page.locator('.analytics-metric').count() === 4, 'Four primary site metrics');
    check(await metric().innerText() === (12450).toLocaleString(), 'Metrics use actual API values');
    check(await page.getByText('Opsi tampilan', { exact: true }).isVisible() && await page.getByLabel('Bandingkan periode lalu', { exact: true }).count() === 0, 'Advanced options start collapsed');
    await mainTop(); await page.screenshot({ path: path.join(output, 'traffic-desktop-light.png') });
    const chart = page.getByRole('img', { name: /^Lalu lintas permintaan\./ });
    await chart.focus(); await page.keyboard.press('Home');
    check(await page.locator('.analytics-chart-readout').first().innerText().then(text => text.includes(String(points[0].requests))), 'Keyboard selects first chart sample');
    await page.keyboard.press('ArrowRight');
    check(await page.locator('.analytics-chart-readout').first().innerText().then(text => text.includes(String(points[1].requests))), 'Arrow keys inspect the next sample');
    check(await page.locator('.analytics-chart svg title').allTextContents().then(labels => labels.includes('Deploy rel123')), 'Deploy markers remain available');
    await page.getByRole('button', { name: 'Opsi tampilan', exact: true }).click();
    await page.getByLabel('Bandingkan periode lalu', { exact: true }).uncheck(); await ready();
    await page.waitForFunction(() => !document.querySelector('.analytics-change'));
    check(requests.some(url => url.pathname.endsWith('/overview') && url.searchParams.get('compare') === 'false'), 'Compare toggle reaches the API');
    await page.getByLabel('Interval data', { exact: true }).selectOption('1m'); await ready();
    const preset = page.getByRole('group', { name: 'Periode analitik', exact: true });
    await preset.getByRole('button', { name: '7 hari', exact: true }).click(); await ready();
    const requested = requests.filter(url => url.pathname.endsWith('/overview')).at(-1);
    check(Number(requested.searchParams.get('to')) - Number(requested.searchParams.get('from')) === 604800, 'Range selection requests seven days');
    const csv = new URL(await page.getByRole('link', { name: 'CSV', exact: true }).getAttribute('href'), base);
    check(csv.searchParams.get('from') === requested.searchParams.get('from') && csv.searchParams.get('to') === requested.searchParams.get('to'), 'CSV stays bound to the displayed snapshot');
    const downloadPromise = page.waitForEvent('download'); await page.getByRole('link', { name: 'CSV', exact: true }).click();
    const download = await downloadPromise;
    const downloadFailure = await download.failure();
    check(downloadFailure === null, `CSV export still downloads: ${downloadFailure || 'success'}`);
    check(await fs.readFile(await download.path(), 'utf8') === csvBytes, 'CSV download contains the actual fixture bytes');
    await preset.getByRole('button', { name: 'Kustom', exact: true }).click(); await ready();
    await page.getByLabel('Mulai', { exact: true }).fill('2026-10-15T12:00');
    await page.getByLabel('Sampai', { exact: true }).fill('2026-10-14T12:00');
    check(await page.getByRole('button', { name: 'Terapkan periode', exact: true }).isDisabled(), 'Invalid custom range cannot be applied');
    await page.getByLabel('Sampai', { exact: true }).fill('2026-10-16T12:00');
    await page.getByRole('button', { name: 'Terapkan periode', exact: true }).click(); await ready();
    check(requests.filter(url => url.pathname.endsWith('/overview')).at(-1).searchParams.get('from') === String(new Date('2026-10-15T12:00').getTime() / 1000), 'Valid custom range applies explicitly');
    await page.getByRole('tab', { name: 'Rincian', exact: true }).click();
    await page.getByText('/products', { exact: true }).waitFor();
    await page.getByRole('button', { name: 'Sumber trafik', exact: true }).click();
    check(await page.getByText('google.com', { exact: true }).isVisible(), 'Breakdown dimensions retain their data');
    await page.getByLabel('Cari rincian trafik', { exact: true }).fill('missing');
    await page.getByText('Tidak ada hasil yang cocok', { exact: true }).waitFor();
    await page.getByRole('button', { name: 'Hapus pencarian rincian', exact: true }).click();
    check(await page.getByText('google.com', { exact: true }).isVisible(), 'Breakdown search clears correctly');
    await mainTop(); await page.screenshot({ path: path.join(output, 'breakdowns-desktop.png') });
    await page.getByRole('tab', { name: 'Slowlog', exact: true }).click();
    check(await page.getByRole('heading', { name: 'PHP-FPM Slowlog Explorer', exact: true }).isVisible(), 'PHP slowlog remains accessible');
    await page.getByRole('tab', { name: 'Keamanan', exact: true }).click();
    await page.getByRole('heading', { name: /Keamanan|Security/ }).first().waitFor(); check(true, 'Security tab retains its existing interface');
    await page.getByRole('tab', { name: 'Trafik', exact: true }).click();
    await preset.getByRole('button', { name: '24 jam', exact: true }).click(); await ready();
    await page.getByRole('button', { name: 'Opsi tampilan', exact: true }).click();
    for (const theme of ['light', 'dark']) {
      await page.evaluate(dark => document.documentElement.classList.toggle('dark', dark), theme === 'dark');
      for (const width of [320, 375, 768, 1440]) {
        await page.setViewportSize({ width, height: 950 });
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `Traffic fits ${width}px ${theme}`);
        check(await chart.boundingBox().then(box => box.height > 180), `Chart remains readable at ${width}px ${theme}`);
      }
    }
    await page.setViewportSize({ width: 375, height: 950 }); await mainTop();
    await page.screenshot({ path: path.join(output, 'traffic-mobile-dark.png') });
    await chart.scrollIntoViewIfNeeded(); await page.screenshot({ path: path.join(output, 'chart-mobile-dark.png') });
    await page.getByRole('tab', { name: 'Rincian', exact: true }).click();
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'Breakdown table scrolls within its mobile container');
    await page.getByRole('tab', { name: 'Trafik', exact: true }).focus();
    await page.keyboard.press('ArrowRight');
    check(await page.getByRole('tab', { name: 'Rincian', exact: true }).getAttribute('aria-selected') === 'true', 'Tab navigation supports arrow keys');
    await page.getByRole('tab', { name: 'Trafik', exact: true }).click();
    await page.setViewportSize({ width: 1440, height: 1050 });
    delay = true;
    await page.getByRole('button', { name: 'Muat ulang analitik', exact: true }).click();
    await page.getByLabel('Situs yang dianalisis', { exact: true }).selectOption('8'); await ready();
    await new Promise(resolve => setTimeout(resolve, 1100));
    check(await metric().innerText() === '880', 'Late response cannot replace another site metrics');
    check(await page.getByRole('tab', { name: 'Slowlog', exact: true }).count() === 0, 'Node projects do not show PHP slowlog');
    delay = false; fail = true;
    await page.getByRole('button', { name: 'Muat ulang analitik', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: 'Fixture analytics unavailable' }).waitFor();
    check(await page.locator('.analytics-metric').count() === 0, 'Failed loads do not display stale metrics as fresh');
    fail = false; noSamples = true;
    await page.getByRole('button', { name: 'Coba lagi', exact: true }).click();
    await page.getByRole('heading', { name: 'Belum ada data metrik lalu lintas untuk periode ini', exact: true }).waitFor();
    check(await metric().innerText() === '0', 'Measured zero remains distinct from failed requests');
    noSamples = false; optionalFail = true;
    await page.getByRole('button', { name: 'Muat ulang analitik', exact: true }).click(); await ready();
    await page.getByRole('tab', { name: 'Runtime', exact: true }).click();
    await page.getByRole('heading', { name: 'Metrik runtime belum tersedia', exact: true }).waitFor(); check(true, 'Optional runtime failure has an honest unavailable state');
    optionalFail = false;
    await page.goto(`${base}/sites/store.example.test/analytics`); await ready();
    check(await page.getByRole('group', { name: 'Periode analitik', exact: true }).isVisible(), 'Project detail shares the analytics workspace');
    failSites = true; await page.goto(`${base}/analytics`);
    await page.getByRole('alert').filter({ hasText: 'Fixture site list unavailable' }).waitFor();
    check(await page.getByText('Belum ada situs untuk dianalisis', { exact: true }).count() === 0, 'Site loading failure is not mislabeled as empty');
    failSites = false; emptySites = true; await page.getByRole('button', { name: 'Coba lagi', exact: true }).click();
    await page.getByText('Belum ada situs untuk dianalisis', { exact: true }).waitFor(); check(true, 'No-sites state explains availability');
    emptySites = false; role = 'admin'; await page.reload(); await ready();
    await page.getByRole('group', { name: 'Lingkup analitik', exact: true }).getByRole('button', { name: 'Server', exact: true }).click();
    await page.getByRole('region', { name: 'Metrik sistem server', exact: true }).waitFor();
    await page.getByRole('group', { name: 'Jenis metrik server', exact: true }).getByRole('button', { name: 'Memori', exact: true }).click();
    check(await page.getByRole('heading', { name: 'Memori & swap', exact: true }).isVisible(), 'Host memory uses actual byte metrics');
    await page.getByRole('group', { name: 'Jenis metrik server', exact: true }).getByRole('button', { name: 'I/O disk', exact: true }).click();
    check(await page.getByRole('heading', { name: 'I/O disk', exact: true }).isVisible(), 'Host disk I/O remains available');
    await mainTop(); await page.screenshot({ path: path.join(output, 'server-desktop-dark.png') });
    await page.getByText('Penyimpanan & retensi analitik', { exact: true }).click();
    await page.getByRole('heading', { name: 'Siklus Data & Retensi Basis Data', exact: true }).waitFor(); check(true, 'Admin retention tools remain accessible');
    await page.setViewportSize({ width: 320, height: 950 });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'Host analytics fits 320px');
    await page.setViewportSize({ width: 1440, height: 1050 });
    failServer = true;
    await page.getByRole('group', { name: 'Periode metrik server', exact: true }).getByRole('button', { name: '1 jam', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: 'Fixture server metrics unavailable' }).waitFor(); check(true, 'Host failures have a visible error');
    await page.getByRole('button', { name: 'Ganti bahasa', exact: true }).click();
    await page.getByRole('heading', { name: 'Analytics', exact: true }).waitFor();
    check(await page.getByText('Monitor server resources and the health of your sites.', { exact: true }).isVisible(), 'English labels work in the analytics workspace');
    check(mutations.length === 0, 'Analytics browsing never performs unexpected mutations');
    check(errors.length === 0, `No runtime errors: ${errors.join(', ')}`);
    check(missing.length === 0, `All requests handled: ${missing.join(', ')}`);
    console.log(`${checks} analytics workspace checks passed`);
  } catch (error) { await page.screenshot({ path: path.join(output, 'failure.png') }); throw error; }
  finally { await browser.close(); await new Promise(resolve => downloadFixture.close(resolve)); }
})().catch(error => { console.error(error); process.exitCode = 1; });
