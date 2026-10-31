/* Local production UI with intercepted APIs only; no agent or service changes. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5206';
const output = path.join(__dirname, '../artifacts/tenant-navigation');
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    await fs.mkdir(output, { recursive: true });
    const context = await browser.newContext({ viewport: { width: 1440, height: 1050 }, permissions: ['clipboard-read', 'clipboard-write'] });
    const page = await context.newPage();
    let role = 'user', fail = false, empty = false, paginate = false, running = false, failSites = false;
    const errors = [], missing = [], mutations = [], scopes = [], sshQueries = [];
    let checks = 0;
    const check = (ok, description) => { assert.ok(ok, description); console.log(`ok ${++checks}: ${description}`); };
    page.on('pageerror', error => errors.push(error.message));
    const sites = [7, 8].map(id => ({ id, user_id: 2, primary_domain: `project${id}.example.test`, kind: 'static', docroot: `/home/alice/sites/${id}`, status: 'active', domains: [], spa_fallback: false, force_https: true, created_at: 1, updated_at: 1 }));
    const endpoint = (id, index = 0) => ({ id: `endpoint-${id}-${index}`, revision: 1, scope: { kind: 'project', site_id: id }, service: 'ssh', host: index ? `ssh-${id}-${index}.example.test` : '2001:db8::7', port: 2222, transport: 'direct', tls_mode: 'ssh', certificate_id: null, enabled: true, primary: index === 0, client_requirement: 'none', created_at: 1, updated_at: 1 });
    await page.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url()), p = url.pathname;
      let body = {}, status = 200;
      if (req.method() !== 'GET') mutations.push(`${req.method()} ${p}`);
      if (p === '/api/v1/auth/me') body = { user: { id: role === 'admin' ? 1 : 2, username: role === 'admin' ? 'admin' : 'alice', role, status: 'active' }, csrf_token: 'fixture' };
      else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (p === '/api/v1/notifications/unread-count') body = { unread_count: 0 };
      else if (p === '/api/v1/connectivity/panel-url') body = { revision: 1, canonical_url: null, current_url: base, endpoint_id: null, endpoint_revision: null, aliases: [] };
      else if (p === '/api/v1/connectivity/panel-tls') body = { certificates: [], profile: 'default', challenge_port: 80 };
      else if (p === '/api/v1/sites') { body = empty ? [] : sites; if (failSites) { status = 503; body = { error: 'Fixture project load failed' }; } }
      else if (/^\/api\/v1\/sites\/\d+$/.test(p)) body = sites.find(s => s.id === Number(p.split('/').at(-1)));
      else if (p.endsWith('/ssl')) body = { certificate: null, force_https: true };
      else if (p.endsWith('/analytics/timeseries')) body = { points: [] };
      else if (p === '/api/v1/jobs') {
        const items = running ? [{ id: 'fixture-job', user_id: 2, name: 'backup.create', status: 'running', progress: 25, payload_json: '{}', result_json: null, error_message: null, retry_count: 0, max_retries: 0, created_at: 1, updated_at: 1 }] : [];
        body = { items, page: 1, per_page: 15, total: items.length, total_pages: 1 };
      } else if (p.endsWith('/events')) { await route.fulfill({ contentType: 'text/event-stream', body: '' }); return; }
      else if (p === '/api/v1/connectivity') {
        const id = Number(url.searchParams.get('site_id')), pageNo = Number(url.searchParams.get('page') || 1);
        if (role === 'user') assert.ok([7, 8].includes(id), 'Tenant endpoint reads must be project-scoped');
        scopes.push(id);
        const rows = empty ? [] : Array.from({ length: paginate ? 26 : 1 }, (_, index) => ({ endpoint: endpoint(id || 7, index), evidence: [] }));
        body = { can_manage: role === 'admin', endpoints: rows.slice((pageNo - 1) * 25, pageNo * 25), page: pageNo, per_page: 25, total: rows.length, total_pages: Math.ceil(rows.length / 25), service_counts: rows.length ? { ssh: rows.length } : {}, projects: id ? sites.filter(site => site.id === id).map(site => ({ id: site.id, name: site.primary_domain })) : [], certificates: [] };
        if (fail) { status = 503; body = { error: 'Fixture connection load failed' }; }
      } else if (p === '/api/v1/ssh/connection-info') {
        sshQueries.push(Object.fromEntries(url.searchParams));
        body = { host: 'ssh.example.test', port: 2222, username: 'alice', user_id: 2, home_dir: '/home/alice', active_path: '/home/alice/sites/7', relative_path: 'sites/7', shell_policy: 'full', can_shell: true, can_sftp: true, has_ssh_keys: true, ssh_keys_count: 1, ssh_command: 'ssh -p 2222 alice@ssh.example.test', sftp_command: 'sftp -P 2222 alice@ssh.example.test', sftp_cd_command: 'cd sites/7', sftp_url: 'sftp://alice@ssh.example.test:2222', ssh_config_snippet: 'Host project\n  HostName ssh.example.test', site_domain: sites[0].primary_domain, endpoint_state: 'configured', endpoint_id: 'registered-ssh', endpoint_revision: 1, transport: 'direct', host_keys: [], host_keys_status: 'unavailable', sftp_path: '/home/alice/sites/7' };
      } else { missing.push(p); status = 404; body = { error: 'Unhandled fixture' }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    await page.goto(`${base}/jobs`);
    await page.getByRole('heading', { name: 'Aktivitas', exact: true }).waitFor();
    check(await page.getByRole('link', { name: 'Aktivitas', exact: true }).count() === 1, 'Tenant sidebar uses Aktivitas');
    check(await page.getByRole('link', { name: 'Konektivitas', exact: true }).count() === 0, 'Tenant has no global connectivity navigation');
    await page.getByText('Belum ada aktivitas', { exact: true }).waitFor();
    check(await page.getByText(/backup, impor dan ekspor database/).count() === 1, 'Activity explains tenant operations');
    await page.keyboard.press('Control+k');
    await page.getByRole('dialog', { name: 'Pencarian cepat' }).waitFor();
    check(await page.getByRole('option').filter({ hasText: 'Aktivitas' }).count() === 1, 'Command palette uses activity');
    check(await page.getByRole('option').filter({ hasText: 'Konektivitas' }).count() === 0, 'Command palette hides global tenant connectivity');
    await page.keyboard.press('Escape');
    running = true; await page.reload();
    await page.getByRole('button', { name: /Aktivitas \(1 aktif\)/ }).click();
    check(await page.getByText('Aktivitas terbaru', { exact: true }).isVisible(), 'Tray uses tenant activity labels');
    check(await page.getByText('Lihat semua aktivitas', { exact: true }).isVisible(), 'Tray opens activity history');
    running = false;
    await page.goto(`${base}/connectivity`);
    await page.getByRole('heading', { name: 'Informasi koneksi', exact: true }).waitFor();
    await page.getByRole('link', { name: 'project7.example.test', exact: true }).click();
    await page.getByRole('heading', { name: 'Alamat layanan terdaftar' }).waitFor();
    await page.getByText('[2001:db8::7]:2222', { exact: true }).waitFor();
    check(page.url().endsWith('/sites/project7.example.test/connections'), 'Legacy link leads to project connection tab');
    check(await page.getByText('Belum diatur', { exact: true }).count() === 0, 'Project info avoids unrelated unconfigured service sections');
    check(await page.getByRole('link', { name: 'Database project', exact: true }).getAttribute('href') === '/sites/project7.example.test/database', 'Database guide stays project-scoped');
    await page.getByRole('button', { name: 'Salin alamat 2001:db8::7', exact: true }).click();
    check(await page.evaluate(() => navigator.clipboard.readText()) === '[2001:db8::7]:2222', 'Copy preserves IPv6 and registered port');
    await page.getByRole('button', { name: 'Panduan SSH / SFTP', exact: true }).click();
    await page.getByText('ssh -p 2222 alice@ssh.example.test', { exact: true }).waitFor();
    check(sshQueries.at(-1).site_id === '7' && sshQueries.at(-1).path === '/home/alice/sites/7', 'SSH guide reuses authorized project/path API');
    await page.keyboard.press('Escape');
    await page.screenshot({ path: path.join(output, 'project-connections-desktop.png') });
    paginate = true;
    await page.getByRole('button', { name: 'Segarkan', exact: true }).click();
    await page.getByRole('button', { name: 'Berikutnya', exact: true }).click();
    await page.getByText('ssh-7-25.example.test:2222', { exact: true }).waitFor();
    check(await page.getByText('2 / 2', { exact: true }).isVisible(), 'Project endpoint pagination reaches final address');
    paginate = false; fail = true;
    await page.getByRole('button', { name: 'Segarkan', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: 'Fixture connection load failed' }).waitFor();
    check(await page.getByText('ssh-7-25.example.test:2222', { exact: true }).count() === 0, 'Load failure removes stale addresses');
    fail = false; empty = true;
    await page.getByRole('button', { name: 'Segarkan', exact: true }).click();
    await page.getByText(/Belum ada alamat layanan yang didaftarkan/).waitFor();
    check(await page.getByRole('button', { name: 'Panduan SSH / SFTP', exact: true }).isVisible(), 'Empty address list keeps connection guides available');
    empty = false;
    await page.goto(`${base}/sites/project8.example.test/connections`);
    await page.getByText('[2001:db8::7]:2222', { exact: true }).waitFor();
    check(scopes.at(-1) === 8, 'Changing project requests that project only');
    for (const width of [320, 375, 1440]) {
      await page.setViewportSize({ width, height: 950 });
      await page.evaluate(() => document.documentElement.classList.add('dark'));
      check(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), `Connection info fits ${width}px dark viewport`);
    }
    await page.setViewportSize({ width: 320, height: 950 });
    await page.screenshot({ path: path.join(output, 'project-connections-mobile-dark.png') });
    await page.goto(`${base}/connectivity`);
    await page.getByRole('link', { name: 'project7.example.test', exact: true }).waitFor();
    await page.getByRole('banner').getByRole('button', { name: 'Buka navigasi', exact: true }).click();
    check(await page.getByRole('link', { name: 'Aktivitas', exact: true }).isVisible(), 'Mobile navigation uses activity');
    check(await page.getByRole('link', { name: 'Konektivitas', exact: true }).count() === 0, 'Mobile navigation hides global connectivity');
    await page.keyboard.press('Escape');
    empty = true; await page.reload();
    await page.getByText(/Belum ada project\./).waitFor(); check(true, 'Old connectivity route handles no projects');
    empty = false; failSites = true; await page.reload();
    await page.getByRole('alert').filter({ hasText: 'Fixture project load failed' }).waitFor();
    failSites = false; await page.getByRole('button', { name: 'Segarkan', exact: true }).click();
    await page.getByRole('link', { name: 'project7.example.test', exact: true }).waitFor(); check(true, 'Project chooser recovers after retry');
    role = 'admin'; await page.setViewportSize({ width: 1440, height: 1050 }); await page.goto(`${base}/jobs`);
    await page.getByRole('heading', { name: 'Antrean Tugas Sistem', exact: true }).waitFor();
    check(await page.getByRole('link', { name: 'Tugas', exact: true }).isVisible(), 'Admin retains task navigation');
    await page.goto(`${base}/connectivity`);
    await page.getByRole('heading', { name: 'Konektivitas', exact: true }).waitFor();
    await page.getByRole('button', { name: 'Tambah alamat', exact: true }).waitFor(); check(true, 'Admin retains endpoint management');
    check(mutations.length === 0, 'Read-only connection flow sends no mutations');
    check(errors.length === 0, `No runtime errors: ${errors.join(', ')}`);
    check(missing.length === 0, `All API calls covered: ${missing.join(', ')}`);
    console.log(`${checks} tenant navigation checks passed`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
