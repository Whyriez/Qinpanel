/* Read-only browser fixtures. No tuning, firewall, DNS, backup or update is run. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5198';
const output = path.join(__dirname, '../artifacts/settings-mobile');
let checks = 0;
const check = (ok, message) => { assert.ok(ok, message); console.log(`ok ${++checks}: ${message}`); };
const profile = {
  profile_type: 'standard', display_name: 'Standard (2 - 4 GB RAM)', description: 'Profil server fixture',
  nginx: { worker_processes: 'auto', worker_connections: 2048, worker_rlimit_nofile: 8192, multi_accept: true, keepalive_timeout_secs: 65, client_body_buffer_size_kb: 128, fastcgi_buffer_size_kb: 64, fastcgi_buffers: '64 64k', open_file_cache_max: 10000, open_file_cache_inactive_secs: 60 },
  sysctl: { somaxconn: 4096, tcp_max_syn_backlog: 4096, tcp_tw_reuse: 1, ip_local_port_range: '10240 65535', swappiness: 10, file_max: 500000, netdev_max_backlog: 5000 },
  php_fpm: { pm_type: 'dynamic', max_children: 12, start_servers: 4, min_spare_servers: 2, max_spare_servers: 6, process_idle_timeout_secs: 10, max_requests: 1000 },
  hints: { recommended_node_workers: 2, recommended_node_max_old_space_mb: 512, recommended_python_workers: 3 },
};
const optimizer = { hardware: { vcpu_count: 2, ram_total_mb: 4096, ram_free_mb: 2048, swap_total_mb: 1024, swap_free_mb: 1024, kernel_version: '6.1.0-fixture-amd64', os_distro: 'Debian GNU/Linux 12 (bookworm)' }, recommended_profile: 'standard', active_profile: 'standard', applied_at: 1780000000, current_settings: profile, recommended_settings: profile, has_backup: true, last_backup_at: 1780000000 };
const doctor = { timestamp: 1780000000, healthy: false, overall_status: 'warn', total_passed: 1, total_warnings: 1, total_failures: 0, checks: [{ id: 'fixture', name: 'Socket runtime', category: 'socket', status: 'warn', message: 'Pemeriksaan fixture untuk socket layanan dengan nama panjang', remediation_hint: 'Periksa konfigurasi layanan', fixable: true }] };
const sections = ['optimizer', 'memory_profile', 'info', 'security_hardening', 'network', 'connectivity', 'server_backup', 'updates', 'reliability'];

(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge', headless: true });
  const errors = [], writes = [], missing = [], external = [];
  let failOptimizer = false;
  try {
    const page = await browser.newPage({ viewport: { width: 320, height: 850 }, reducedMotion: 'reduce' });
    page.on('pageerror', error => errors.push(error.message));
    page.on('request', req => { if (new URL(req.url()).origin !== new URL(base).origin) external.push(req.url()); });
    await page.route('**/api/v1/**', async route => {
      const request = route.request(), p = new URL(request.url()).pathname;
      if (request.method() !== 'GET') { writes.push(p); await route.fulfill({ status: 405, body: '{}' }); return; }
      let body = {}, status = 200;
      if (p === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (p === '/api/v1/system/optimizer') { body = optimizer; if (failOptimizer) { status = 503; body = { detail: 'Status server sementara tidak tersedia' }; } }
      else if (p === '/api/v1/system/security-hardening') body = { asvs_audit: { total_controls: 1, passed_controls: 1, failed_controls: 0, compliance_percentage: 100, requirements: [{ id: 'V1.1.1', chapter: 'Arsitektur', title: 'Isolasi runtime', description: 'Pemeriksaan fixture', implementation_evidence: 'Konfigurasi fixture', status: 'Pass', level: 'L2' }] }, nftables_status: { enabled: true, table_name: 'panel_loopback_guard', active_rules_count: 2 }, systemd_security: [{ service_name: 'panel-api.service', exposure_score: 1.2, is_root: false, directives: [] }], secret_key_version: 1 };
      else if (p === '/api/v1/system/memory-profile') body = { generated_at: '2026-10-25T10:00:00Z', allocator: 'system', release_profile: { opt_level: 'z', lto: 'thin', codegen_units: 1, panic_strategy: 'abort', strip: true, memory_allocator: 'system' }, api_process: { rss_mb: 25 }, agent_process: { rss_mb: 15 }, total_panel_rss_mb: 40, bundle_gzip_kb: 140, cold_start_api_ms: 100, api_read_latency_p95_ms: 15, targets: [{ target_id: 'api', name: 'Memori panel', threshold: '40 MB', measured: '25 MB', passed: true, notes: 'Fixture' }], all_passed: true, summary: 'Seluruh target fixture terpenuhi.' };
      else if (p === '/api/v1/system/doctor') body = doctor;
      else if (p === '/api/v1/system/reconcile') body = { timestamp: 1, total_resources_checked: 2, is_clean: true, drifts: [] };
      else if (p === '/api/v1/system/backups') body = [];
      else if (p === '/api/v1/system/server-backup') body = { supported: true, job: null, backups: [], inspection: null };
      else if (p === '/api/v1/system/update') body = { current_version: 'v0.1.0', installed_digest: null, supported: true, checked_at: null, available: null, check_error: null, update: null };
      else if (p === '/api/v1/connectivity') body = { can_manage: true, endpoints: [], page: 1, per_page: 25, total: 0, total_pages: 0, service_counts: {}, projects: [], certificates: [] };
      else if (p === '/api/v1/connectivity/panel-url') body = { revision: 1, canonical_url: null, current_url: base, endpoint_id: null, endpoint_revision: null, aliases: [] };
      else if (p === '/api/v1/connectivity/panel-tls') body = { certificates: [], profile: 'shortlived', challenge_port: 80 };
      else if (p === '/api/v1/network') body = { ipv4: '192.0.2.10', ipv6: '2001:db8::10', projects: [] };
      else if (p === '/api/v1/network/cloudflare') body = { configured: false, account_id: null, connector_installed: false, managed_tunnels: [] };
      else if (p === '/api/v1/network/dns01') body = { provider: { configured: false, pending: [] }, projects: [], orders: [], bindings: [] };
      else if (p === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 0, page: 1, per_page: 100 };
      else if (p.includes('notifications')) body = { notifications: [], unread_count: 0, count: 0, total: 0 };
      else { status = 404; missing.push(p); body = { detail: `Missing fixture: ${p}` }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const root = () => page.getByTestId('settings-page');
    const picker = () => root().getByLabel('Bagian pengaturan', { exact: true });
    const content = () => root().locator('#server-settings-content');
    async function ready(section) {
      const view = content();
      const target = {
        optimizer: () => view.getByText(optimizer.hardware.os_distro, { exact: true }),
        memory_profile: () => view.getByText('Waktu: 2026-10-25T10:00:00Z', { exact: true }),
        info: () => view.getByRole('heading', { name: 'Profil performa server', exact: true }),
        security_hardening: () => view.getByPlaceholder('Cari kontrol ASVS berdasarkan ID, judul, atau deskripsi...'),
        network: () => view.getByLabel('IPv4 publik', { exact: true }),
        connectivity: () => view.getByText(/Belum ada alamat koneksi yang diatur/),
        server_backup: () => view.getByText('Belum ada arsip backup migrasi.', { exact: true }),
        updates: () => view.getByText('v0.1.0', { exact: true }).first(),
        reliability: () => view.getByText('Socket runtime', { exact: true }),
      }[section]();
      await target.waitFor();
      if (section === 'connectivity') {
        await view.getByText('Belum ada sertifikat IP panel.', { exact: true }).waitFor();
        await view.getByText('Email belum menyertakan tautan absolut sampai URL diaktifkan.', { exact: true }).waitFor();
      }
    }
    async function layout(label) {
      check(await page.locator('html').evaluate(el => el.scrollWidth <= el.clientWidth + 1) && await root().evaluate(el => el.scrollWidth <= el.clientWidth + 1), `${label}: page and settings content do not overflow horizontally`);
      const escaped = await root().locator('button:visible, select:visible, input:not([type="radio"]):not([type="checkbox"]):visible').evaluateAll(elements => elements.filter(el => {
        const r = el.getBoundingClientRect();
        return r.height < 44 || r.left < -1 || r.right > innerWidth + 1;
      }).map(el => el.textContent || el.placeholder || el.title));
      check(escaped.length === 0, `${label}: controls fit and are at least 44px high (${escaped.join(', ')})`);
    }
    await page.goto(`${base}/settings?section=optimizer&retain=fixture`);
    await content().getByText('Debian GNU/Linux 12 (bookworm)', { exact: true }).waitFor();
    for (const theme of ['light', 'dark']) {
      await page.evaluate(value => { localStorage.setItem('nusa_theme', value); document.documentElement.classList.toggle('dark', value === 'dark'); }, theme);
      for (const width of [320, 360, 390, 768, 1024, 1440]) {
        await page.setViewportSize({ width, height: 900 });
        const mobile = await picker().isVisible();
        check(mobile === await root().evaluate(el => el.clientWidth < 760), `${theme} ${width}px: uses ${mobile ? 'compact picker' : 'grouped navigation'}`);
        for (const section of sections) {
          if (mobile) await picker().selectOption(section);
          else await root().getByRole('navigation', { name: 'Bagian pengaturan server' }).locator(`button[aria-controls="server-settings-content"]`).nth(sections.indexOf(section)).click();
          await page.waitForFunction(value => new URL(location.href).searchParams.get('section') === value, section);
          await ready(section);
          check(mobile ? await picker().inputValue() === section : await root().locator('.settings-desktop-navigation button[aria-pressed="true"]').count() === 1 && await root().locator('.settings-desktop-navigation button').nth(sections.indexOf(section)).getAttribute('aria-pressed') === 'true', `${theme} ${width}px ${section}: selected section is explicit`);
          await layout(`${theme} ${width}px ${section}`);
          check(new URL(page.url()).searchParams.get('retain') === 'fixture', `${theme} ${width}px ${section}: unrelated URL parameters survive`);
          if (width === 320) {
            const tabs = content().locator('.settings-section-tabs > button');
            for (let i = 0; i < await tabs.count(); i++) {
              const tab = tabs.nth(i);
              const name = await tab.innerText();
              await tab.click();
              await layout(`${theme} ${width}px ${section} / ${name}`);
            }
            if (await tabs.count()) {
              await tabs.first().scrollIntoViewIfNeeded();
              await page.screenshot({ path: path.join(output, `${theme}-${section}-submenus-320.png`), fullPage: true });
            }
          }
          if (width === 320 || (width === 1440 && section === 'optimizer')) {
            await root().getByRole('heading', { level: 1 }).scrollIntoViewIfNeeded();
            await page.screenshot({ path: path.join(output, `${theme}-${section}-${width}.png`), fullPage: true });
          }
        }
      }
    }
    await page.setViewportSize({ width: 390, height: 800 });
    await picker().selectOption('info');
    await page.reload();
    await picker().waitFor();
    check(await picker().inputValue() === 'info', 'deep link survives reload');
    await page.goBack();
    check(await picker().inputValue() === 'reliability', 'browser back restores menu selection');
    await page.goForward();
    check(await picker().inputValue() === 'info', 'browser forward restores menu selection');
    await picker().focus();
    check(await picker().evaluate(el => getComputedStyle(el).outlineStyle !== 'none'), 'mobile picker has visible keyboard focus');
    await page.evaluate(() => { document.documentElement.style.fontSize = '200%'; });
    await layout('390px with 200% text');
    await page.evaluate(() => { document.documentElement.style.fontSize = ''; });
    failOptimizer = true;
    await picker().selectOption('optimizer');
    await content().getByRole('alert').filter({ hasText: 'Status server sementara tidak tersedia' }).waitFor();
    await layout('optimizer error state');
    check(writes.length === 0, `no mutation requests (${writes.join(', ')})`);
    check(missing.length === 0, `all API fixtures are defined (${missing.join(', ')})`);
    check(external.length === 0, 'all requests remain local');
    check(errors.length === 0, `no browser exceptions (${errors.join(', ')})`);
    console.log(`${checks} server settings responsive checks passed. Screenshots: ${output}`);
  } catch (error) {
    if (errors.length) console.error('Browser exceptions:', errors);
    const page = browser.contexts()[0]?.pages()[0];
    if (page) await page.screenshot({ path: path.join(output, 'failure.png'), fullPage: true });
    throw error;
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
