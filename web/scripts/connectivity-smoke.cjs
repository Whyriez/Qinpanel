/* Built UI + intercepted API fixtures only. No provider, probe or panel daemon. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5199';
const artifacts = path.join(__dirname, '../artifacts/connectivity');
let checks = 0;
function check(value, label) { assert.ok(value, label); console.log(`ok ${++checks}: ${label}`); }
function latch() { let resolve; const promise = new Promise(r => { resolve = r; }); return { promise, resolve }; }
(async () => {
  await fs.mkdir(artifacts, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge' });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  let actor = 'admin', failRead = false, malformed = false, failCreate = false, conflict = false, next = 1;
  let readGate = latch(), mutationGate = null, diagnosisGate = null, diagnosisFailure = false, diagnosisCalls = 0;
  const errors = [], mutations = [], keys = new Map(), reports = new Map();
  let endpoints = [];
  const projects = [{ id: 1, name: 'app.example.test' }, { id: 2, name: 'other.example.test' }];
  const certificates = [{ id: 10, site_id: 1, name: 'app.example.test' }, { id: 20, site_id: 2, name: 'other.example.test' }];
  const client = transport => transport === 'cloudflare_access' ? 'cloudflared' : transport === 'private_network' ? 'cloudflare_one' : 'none';
  const generated = config => ({ ...config, id: `00000000-0000-4000-8000-${String(next++).padStart(12, '0')}`, revision: 1, client_requirement: client(config.transport), created_at: 1, updated_at: 1 });
  const attach = page => { page.setDefaultTimeout(15000); page.on('pageerror', error => errors.push(error.message)); };
  await context.route('**/api/v1/**', async route => {
    const req = route.request(), url = new URL(req.url()), p = url.pathname, method = req.method();
    let status = 200, body = {};
    if (p === '/api/v1/auth/me') body = { user: { id: actor === 'admin' ? 1 : 2, username: actor, role: actor === 'admin' ? 'admin' : 'user' }, csrf_token: 'fixture' };
    else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (p === '/api/v1/connectivity' && method === 'GET') {
      if (readGate) await readGate.promise;
      if (failRead) { status = 503; body = { detail: 'Fixture: data konektivitas belum tersedia.' }; }
      else if (malformed) body = {};
      else {
        const authorized = endpoints.filter(e => actor === 'admin' || (e.scope.kind === 'project' && e.scope.site_id === 1));
        const service_counts = {}; for (const e of authorized) service_counts[e.service] = (service_counts[e.service] || 0) + 1;
        const filtered = authorized.filter(e => !url.searchParams.get('service') || e.service === url.searchParams.get('service'));
        const page = Number(url.searchParams.get('page') || 1), per_page = Number(url.searchParams.get('per_page') || 25);
        body = { can_manage: true, page, per_page, total: filtered.length, total_pages: Math.ceil(filtered.length / per_page), service_counts, endpoints: filtered.slice((page - 1) * per_page, page * per_page).map(endpoint => ({ endpoint: { ...endpoint }, evidence: reports.get(endpoint.id) || [] })), projects: actor === 'admin' ? projects : projects.slice(0, 1), certificates: actor === 'admin' ? certificates : [] };
      }
    } else if (p.endsWith('/diagnose') && method === 'POST') {
      diagnosisCalls++;
      assert.equal(actor, 'admin'); assert.equal(req.headers()['x-csrf-token'], 'fixture');
      const payload = req.postDataJSON(); assert.deepEqual(Object.keys(payload), ['expected_revision']);
      const endpoint = endpoints.find(e => e.id === p.split('/').at(-2)); assert.ok(endpoint);
      assert.equal(payload.expected_revision, endpoint.revision);
      if (diagnosisGate) await diagnosisGate.promise;
      if (diagnosisFailure) { diagnosisFailure = false; status = 409; body = { detail: 'Fixture: tunggu 60 detik sebelum memeriksa kembali.' }; }
      else {
        const now = Math.floor(Date.now() / 1000);
        const report = { id: 100, endpoint_id: endpoint.id, service: endpoint.service, endpoint_revision: endpoint.revision, kind: 'dns', observation_point: 'local_server', observer_id: 'server-resolver', address_family: 'ipv4', observed_at: now, expires_at: now + 300, outcome: 'pass', reason: 'verified', current: true, expired: false, revision_matches: true, suggested_action: 'Tidak ada tindakan.' };
        const evidence = [report, { ...report, id: 101, kind: 'route', outcome: 'unknown', reason: 'policy_blocked', suggested_action: 'Gunakan pengujian client.' }, { ...report, id: 102, kind: 'listener', outcome: 'unknown', reason: 'not_tested' }, { ...report, id: 103, kind: 'firewall', address_family: null, outcome: 'unknown', reason: 'local_guard_only' }, { ...report, id: 104, kind: 'tls', outcome: 'fail', reason: 'tls_failure' }];
        reports.set(endpoint.id, evidence); body = { endpoint, evidence };
      }
    } else if (p === '/api/v1/connectivity/panel-tls' && method === 'GET') body = { certificates: [], profile: 'shortlived', challenge_port: 80 };
    else if (p === '/api/v1/connectivity/panel-url' && method === 'GET') { body = { revision: 0, canonical_url: null, current_url: base, endpoint_id: null, endpoint_revision: null, aliases: [] };
    } else if (p.startsWith('/api/v1/connectivity/') && method === 'GET') {
      body = endpoints.find(e => e.id === p.split('/').at(-1) && (actor === 'admin' || (e.scope.kind === 'project' && e.scope.site_id === 1)));
      if (!body) { status = 404; body = { detail: 'Endpoint tidak ditemukan.' }; }
    } else if (p.startsWith('/api/v1/connectivity') && method !== 'GET') {
      const payload = req.postDataJSON(), key = req.headers()['idempotency-key'];
      mutations.push({ method, payload, key });
      assert.equal(req.headers()['x-csrf-token'], 'fixture');
      if (mutationGate) await mutationGate.promise;
      if (method === 'POST') {
        assert.match(key, /^[0-9a-f-]{36}$/);
        const saved = keys.get(key);
        if (saved) { assert.equal(saved.payload, JSON.stringify(payload)); body = endpoints.find(e => e.id === saved.id); if (!body) { status = 409; body = { detail: 'Alamat sudah dihapus.' }; } }
        else { body = generated(payload); endpoints.push(body); keys.set(key, { id: body.id, payload: JSON.stringify(payload) }); status = 201; }
        if (failCreate) { failCreate = false; status = 503; body = { detail: 'Fixture: respons penyimpanan terputus.' }; }
      } else {
        const existing = endpoints.find(e => e.id === p.split('/').at(-1)); assert.ok(existing);
        if (conflict) { conflict = false; existing.revision++; existing.port = 2444; }
        if (existing.revision !== payload.expected_revision) { status = 409; body = { detail: 'Endpoint telah berubah. Muat ulang konfigurasi.' }; }
        else if (method === 'PUT') { assert.ok(!('id' in payload.config)); assert.ok(!('client_requirement' in payload.config)); Object.assign(existing, payload.config, { revision: existing.revision + 1, client_requirement: client(payload.config.transport) }); body = existing; }
        else { endpoints = endpoints.filter(e => e.id !== existing.id); status = 204; }
      }
    } else if (p === '/api/v1/network') body = { ipv4: null, ipv6: null, projects: [] };
    else if (p === '/api/v1/network/cloudflare') body = { configured: false, account_id: null, connector_installed: false, managed_tunnels: [] };
    else if (p === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 1, total_pages: 0 };
    else if (p.includes('notifications')) body = { items: [], notifications: [], unread_count: 0 };
    else if (p.endsWith('/system/info')) body = { environment: 'production', agent_connected: false };
    else { status = 503; body = { error: 'Outside connectivity fixture' }; if (method !== 'GET') throw new Error(`Unexpected mutation ${method} ${p}`); }
    await route.fulfill({ status, contentType: 'application/json', body: status === 204 ? undefined : JSON.stringify(body) });
  });
  try {
    const page = await context.newPage(); attach(page);
    const root = () => page.getByTestId('connectivity-settings');
    const dialog = () => page.getByRole('dialog');
    const field = label => dialog().getByLabel(label, { exact: true });
    const hostname = () => dialog().getByLabel('Hostname atau IP', { exact: false });
    const save = () => dialog().getByRole('button', { name: 'Simpan alamat', exact: true });
    await page.goto(`${base}/settings?section=connectivity`, { waitUntil: 'domcontentloaded' });
    await root().getByText('Memuat konektivitas…').waitFor();
    check(await root().getByRole('button', { name: 'Tambah alamat' }).count() === 0, 'loading does not expose unverified admin controls');
    readGate.resolve(); readGate = null;
    await root().getByText('Belum ada alamat koneksi yang diatur.', { exact: false }).waitFor();
    check(await root().getByText('Belum diatur', { exact: true }).count() === 7, 'empty state represents all seven admin services without fabricated endpoints');
    await root().getByRole('button', { name: 'Tambah alamat' }).click();
    await field('Profil awal').selectOption('tunnel');
    check(await hostname().inputValue() === '' && mutations.length === 0, 'preset changes only the draft and does not infer a hostname or send requests');
    await field('Layanan').selectOption('ssh'); await field('Profil awal').selectOption('mixed');
    check(await field('Jalur koneksi').inputValue() === 'direct', 'mixed preset retains direct SSH independently of web Tunnel');
    await hostname().fill('2001:db8::7'); await field('Port').fill('2222');
    mutationGate = latch();
    await save().evaluate(button => { button.click(); button.click(); });
    await dialog().getByRole('button', { name: 'Menyimpan…' }).waitFor();
    await page.keyboard.press('Escape');
    check(await dialog().isVisible() && await hostname().isDisabled(), 'pending save blocks dismiss and draft changes');
    const pending = mutationGate; mutationGate = null; pending.resolve();
    await dialog().waitFor({ state: 'hidden' });
    check(mutations.length === 1 && endpoints.length === 1, 'double click produces one metadata mutation');
    await root().getByText('[2001:db8::7]:2222', { exact: true }).waitFor();
    check(mutations[0].payload.port === 2222, 'IPv6 authority and custom port survive persistence');
    await page.reload({ waitUntil: 'domcontentloaded' }); await root().getByText('[2001:db8::7]:2222', { exact: true }).waitFor();
    check(new URL(page.url()).searchParams.get('section') === 'connectivity', 'selected settings section survives reload');
    await root().getByRole('button', { name: 'Ubah 2001:db8::7', exact: true }).click();
    await hostname().fill('ssh.example.test'); await field('Profil awal').selectOption('tunnel'); await save().click(); await dialog().waitFor({ state: 'hidden' });
    await root().getByText('Memerlukan cloudflared pada perangkat client.').waitFor();
    check(mutations.at(-1).payload.expected_revision === 1 && mutations.at(-1).payload.config.transport === 'cloudflare_access', 'replacement sends an explicit revision and Access route');
    const checkButton = () => root().getByRole('button', { name: 'Periksa ssh.example.test', exact: true });
    check(await checkButton().isEnabled(), 'admin can check a registered endpoint');
    diagnosisGate = latch();
    await checkButton().focus(); await page.keyboard.press('Enter');
    await root().getByRole('button', { name: 'Memeriksa… ssh.example.test', exact: true }).waitFor();
    check(await root().getByRole('button', { name: 'Memeriksa… ssh.example.test', exact: true }).isDisabled() && await root().getByRole('button', { name: 'Ubah ssh.example.test', exact: true }).isDisabled(), 'pending diagnosis blocks duplicate checks and conflicting edits');
    check(diagnosisCalls === 1, 'keyboard check sends exactly one typed request');
    const gate = diagnosisGate; diagnosisGate = null; gate.resolve();
    await root().getByRole('status').getByText('Pemeriksaan selesai.', { exact: false }).waitFor();
    check(await root().getByText('Belum diuji dari luar', { exact: true }).isVisible(), 'server results never claim external readiness');
    check(await root().getByText('TLS: Perlu tindakan', { exact: true }).isVisible() && await root().getByText('Firewall milik panel: Belum diketahui', { exact: true }).isVisible(), 'TLS failure and firewall unknown stay separate');
    await root().getByText('Bukti pemeriksaan (5)', { exact: true }).click();
    check(await root().getByText('Target dibatasi kebijakan probe', { exact: false }).isVisible(), 'policy-blocked probe result has a clear layer and reason');
    diagnosisFailure = true; await checkButton().click();
    await root().getByRole('alert').getByText('Fixture: tunggu 60 detik sebelum memeriksa kembali.').waitFor();
    check(await checkButton().isEnabled(), 'cooldown error releases the pending control');
    await root().getByRole('button', { name: 'Muat ulang', exact: true }).click();
    await root().getByText('TLS: Perlu tindakan', { exact: true }).waitFor();
    check(await root().getByText('TCP dari server: Belum diketahui', { exact: true }).isVisible(), 'persisted results survive reload without a green route claim');
    await root().getByRole('button', { name: 'Ubah ssh.example.test', exact: true }).click(); await field('Port').fill('2999'); conflict = true;
    await save().click(); await dialog().getByRole('alert').getByText('Endpoint telah berubah. Muat ulang konfigurasi.').waitFor();
    check(await field('Port').inputValue() === '2999', 'conflict preserves the unsaved draft instead of overwriting it');
    await dialog().getByRole('button', { name: 'Muat versi terbaru dan ganti isian' }).click();
    await page.waitForFunction(() => document.querySelector('[role=dialog] input[type=number]')?.value === '2444');
    await field('Port').fill('3001'); await save().click(); await dialog().waitFor({ state: 'hidden' });
    check(mutations.at(-1).payload.expected_revision === 3, 'explicit reload adopts the newest revision before retry');
    await root().getByRole('button', { name: 'Tambah alamat' }).click(); await field('Layanan').selectOption('website');
    check(await field('Sertifikat lokal').locator('option[value="10"]').count() === 1 && await field('Sertifikat lokal').locator('option[value="20"]').count() === 0, 'certificate selector is restricted to the chosen project');
    await field('Project').selectOption('2');
    check(await field('Sertifikat lokal').locator('option[value="20"]').count() === 1, 'changing project updates certificate choices');
    await field('Project').selectOption('1'); await hostname().fill('web.example.test'); await field('Profil awal').selectOption('mixed');
    failCreate = true; const before = mutations.length; await save().click(); await dialog().getByText('Fixture: respons penyimpanan terputus.').waitFor();
    check(await hostname().inputValue() === 'web.example.test', 'failed create retains the draft');
    await save().click(); await dialog().waitFor({ state: 'hidden' });
    check(mutations[before].key === mutations[before + 1].key && endpoints.length === 2, 'retry reuses its create key and does not duplicate an already committed endpoint');
    await root().getByRole('button', { name: 'Hapus web.example.test', exact: true }).click();
    const deleteBefore = mutations.length;
    check(await dialog().getByRole('button', { name: 'Hapus alamat', exact: true }).isDisabled(), 'delete requires typed confirmation');
    await dialog().getByLabel(/Ketik/).fill('web.example.test');
    await dialog().getByRole('button', { name: 'Hapus alamat', exact: true }).click(); await dialog().waitFor({ state: 'hidden' });
    check(mutations.length === deleteBefore + 1 && endpoints.length === 1, 'confirmed delete removes only the selected endpoint');
    const ssh = endpoints[0], now = Math.floor(Date.now() / 1000);
    const report = { id: 1, endpoint_id: ssh.id, service: 'ssh', endpoint_revision: ssh.revision, kind: 'service', observation_point: 'local_server', observer_id: 'server', address_family: null, observed_at: now - 10, expires_at: now + 600, outcome: 'pass', reason: 'verified', current: true, expired: false, revision_matches: true, suggested_action: 'Tidak ada tindakan.' };
    reports.set(ssh.id, [report, { ...report, id: 2, kind: 'installation', expires_at: now - 1, expired: true, current: false }, { ...report, id: 3, kind: 'route', endpoint_revision: ssh.revision - 1, current: false, revision_matches: false }, { ...report, id: 4, kind: 'external_reachability', observation_point: 'user_client', observer_id: 'laptop', address_family: 'ipv4', outcome: 'unknown', reason: 'timeout' }, { ...report, id: 5, kind: 'external_reachability', observation_point: 'external_probe', observer_id: 'probe', address_family: 'ipv6', outcome: 'fail', reason: 'port_blocked' }]);
    await root().getByRole('button', { name: 'Muat ulang', exact: true }).click(); await root().getByText('Layanan lokal: Berjalan', { exact: true }).waitFor();
    await root().getByText('Bukti pemeriksaan (5)', { exact: true }).click();
    check(await root().getByText(/Kedaluwarsa/).count() > 0 && await root().getByText(/Konfigurasi telah berubah/).count() > 0, 'history distinguishes expired evidence from obsolete configuration');
    check(await root().getByText('IPV4 · laptop: Belum diketahui', { exact: true }).count() === 1 && await root().getByText('IPV6 · probe: Perlu tindakan', { exact: true }).count() === 1, 'IPv4 unknown cannot mask IPv6 failure');
    check(await root().getByText('TCP dari server: Belum diperiksa', { exact: true }).count() === 1, 'obsolete route pass is excluded from readiness');
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => { localStorage.setItem('nusa_theme', theme); document.documentElement.classList.toggle('dark', theme === 'dark'); }, theme);
      await page.screenshot({ path: path.join(artifacts, `desktop-${theme}.png`), fullPage: true, animations: 'disabled' });
      const contrast = await root().getByRole('button', { name: 'Tambah alamat', exact: true }).evaluate(button => {
        const style = getComputedStyle(button);
        const luminance = color => color.match(/[\d.]+/g).slice(0, 3).map(Number).map(v => v / 255).map(v => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4).reduce((sum, v, i) => sum + v * [.2126, .7152, .0722][i], 0);
        const a = luminance(style.color), b = luminance(style.backgroundColor);
        return (Math.max(a, b) + .05) / (Math.min(a, b) + .05);
      });
      check(contrast >= 4.5, `${theme} primary action text meets WCAG AA contrast (${contrast.toFixed(2)}:1)`);
    }
    reports.set(ssh.id, [{ ...report, expires_at: Math.floor(Date.now() / 1000) + 2 }]);
    await root().getByRole('button', { name: 'Muat ulang', exact: true }).click(); await root().getByText('Layanan lokal: Berjalan', { exact: true }).waitFor();
    await root().getByText('Layanan lokal: Berjalan', { exact: true }).waitFor({ state: 'hidden' });
    check(true, 'evidence expires while the page stays open');
    failRead = true; await root().getByRole('button', { name: 'Muat ulang', exact: true }).click(); await root().getByRole('alert').waitFor();
    check(await root().getByText('ssh.example.test:3001', { exact: true }).count() === 0, 'failed refresh removes stale summary');
    failRead = false; await root().getByRole('button', { name: 'Coba lagi' }).click(); await root().getByText('ssh.example.test:3001', { exact: true }).waitFor();
    malformed = true; await root().getByRole('button', { name: 'Muat ulang', exact: true }).click(); await root().getByText(/Data konektivitas tidak valid/).waitFor();
    check(true, 'malformed API response produces a recoverable error');
    malformed = false; await root().getByRole('button', { name: 'Coba lagi' }).click(); await root().getByText('ssh.example.test:3001', { exact: true }).waitFor();
    await page.setViewportSize({ width: 320, height: 600 });
    await root().getByRole('button', { name: 'Tambah alamat' }).click(); await field('Layanan').selectOption('imap');
    await field('Profil awal').selectOption('tunnel');
    check(await field('Jalur koneksi').inputValue() === 'provider', 'mail Tunnel preset selects an external provider rather than public SMTP over web Tunnel');
    await field('Layanan').selectOption('smtp_inbound');
    check(await field('Port').inputValue() === '25' && await field('Keamanan koneksi').inputValue() === 'starttls', 'inbound SMTP defaults to STARTTLS on port 25');
    await field('Layanan').selectOption('imap'); await field('Profil awal').selectOption('tunnel');
    await dialog().getByRole('button', { name: 'Tutup', exact: true }).focus(); await page.keyboard.press('Shift+Tab');
    assert.ok(await save().evaluate(button => document.activeElement === button));
    await page.keyboard.press('Tab');
    check(await dialog().getByRole('button', { name: 'Tutup', exact: true }).evaluate(button => document.activeElement === button), 'keyboard focus wraps inside the mobile dialog');
    await field('Port').scrollIntoViewIfNeeded(); await field('Port').focus();
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'mobile form and page have no horizontal overflow');
    await page.screenshot({ path: path.join(artifacts, 'mobile-dialog.png'), fullPage: true });
    await page.keyboard.press('Escape'); await dialog().waitFor({ state: 'hidden' });
    check(true, 'Escape closes an unsaved form on mobile');
    for (let i = 0; i < 27; i++) endpoints.push(generated({ ...ssh, primary: false, host: `paged-${i}.example.test` }));
    await root().getByRole('button', { name: 'Muat ulang', exact: true }).click();
    await root().getByRole('button', { name: 'Berikutnya', exact: true }).click();
    await root().getByText('28 alamat · Halaman 2 dari 2', { exact: true }).waitFor();
    check(await root().getByText('paged-26.example.test:3001', { exact: true }).count() === 1, 'pagination fetches the next bounded page');
    await root().getByLabel('Filter layanan', { exact: true }).selectOption('website');
    await root().getByText('Belum ada alamat untuk layanan ini.', { exact: false }).waitFor();
    check(await root().getByRole('heading', { name: 'SSH / SFTP', exact: true }).count() === 0, 'service filtering resets pagination and excludes unrelated groups');
    await root().getByLabel('Filter layanan', { exact: true }).selectOption('');
    await root().getByRole('button', { name: 'Berikutnya', exact: true }).click();
    await root().getByText('28 alamat · Halaman 2 dari 2', { exact: true }).waitFor();
    endpoints = endpoints.filter(e => !e.host.startsWith('paged-'));
    await root().getByRole('button', { name: 'Muat ulang', exact: true }).click(); await root().getByText('ssh.example.test:3001', { exact: true }).waitFor();
    check(await root().getByRole('button', { name: 'Berikutnya', exact: true }).count() === 0, 'shrinking data returns the UI to a valid page');
    const beforeLink = mutations.length; await root().getByRole('link', { name: 'Kelola DNS dan Cloudflare' }).click(); await page.getByTestId('network-settings').waitFor();
    check(new URL(page.url()).searchParams.get('section') === 'network' && mutations.length === beforeLink, 'DNS link reuses existing settings without a provider mutation');
    await page.close(); actor = 'user';
    endpoints.push(generated({ service: 'website', scope: { kind: 'project', site_id: 1 }, transport: 'cloudflare_http', host: 'mine.example.test', port: 443, tls_mode: 'tls', certificate_id: null, enabled: true, primary: true }));
    endpoints.push(generated({ service: 'website', scope: { kind: 'project', site_id: 2 }, transport: 'direct', host: 'private-other.example.test', port: 443, tls_mode: 'tls', certificate_id: null, enabled: true, primary: true }));
    const tenant = await context.newPage(); attach(tenant); await tenant.setViewportSize({ width: 390, height: 844 });
    await tenant.goto(`${base}/connectivity`, { waitUntil: 'domcontentloaded' }); const view = tenant.getByTestId('connectivity-settings');
    await view.getByText('mine.example.test:443', { exact: true }).waitFor();
    check(!(await tenant.content()).includes('private-other.example.test') && !(await tenant.content()).includes('ssh.example.test:3001'), 'tenant does not see foreign project or host-wide private endpoints');
    check(await view.getByRole('button', { name: /^Periksa/ }).count() === 0, 'tenant cannot initiate network diagnostics');
    check(await view.getByRole('button', { name: 'Tambah alamat' }).count() === 0 && await view.getByRole('button', { name: /^Ubah|^Hapus/ }).count() === 0, 'tenant remains read-only even when a malformed server permission flag says can_manage');
    check(await view.getByRole('link', { name: 'Buka terminal browser' }).count() === 1 && await view.getByRole('link', { name: 'Buka File Manager' }).count() === 1, 'browser access alternatives remain available without native endpoints');
    check(await tenant.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'tenant mobile summary has no horizontal overflow');
    await tenant.screenshot({ path: path.join(artifacts, 'tenant-mobile.png'), fullPage: true });
    check(errors.length === 0, `no browser exceptions: ${errors.join('; ')}`);
    console.log(`${checks} connectivity browser checks passed; all APIs were fixtures.`);
  } catch (error) {
    try { await context.pages().at(-1)?.screenshot({ path: path.join(artifacts, 'failure.png'), fullPage: true }); } catch { /* Page may already be closed. */ }
    throw error;
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
