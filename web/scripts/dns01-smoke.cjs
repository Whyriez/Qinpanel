/* Production bundle with API fixtures only. No provider, ACME, or panel daemons. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5199';
let checks = 0;
const check = (ok, text) => { assert.ok(ok, text); console.log(`ok ${++checks}: ${text}`); };
(async () => {
  const browser = await chromium.launch({ headless: true, channel: 'msedge' });
  try {
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
    const page = await context.newPage(); page.setDefaultTimeout(15000);
    let actor = 'admin', failed = false, malformed = false, reject = false, gate, release, reads = 0;
    const inventory = { provider: { configured: false, pending: [] }, projects: [{ site_id: 7, hostname: 'app.example.com' }], orders: [], bindings: [] };
    const jobs = new Map(), calls = [], errors = [];
    page.on('pageerror', e => errors.push(e.message));
    const newJob = row => ({ id: row.job_id, user_id: 1, name: 'ssl.dns01.issue', status: 'queued', progress: 0, payload_json: JSON.stringify({ order_id: row.id }), result_json: null, error_message: null, retry_count: 0, max_retries: 0, created_at: 1, updated_at: 1 });
    await context.route('**/api/v1/**', async route => {
      const req = route.request(), u = new URL(req.url()), p = u.pathname, method = req.method(); let body = {}, status = 200;
      if (p === '/api/v1/auth/me') body = { user: { id: actor === 'admin' ? 1 : 2, username: actor, role: actor }, csrf_token: 'fixture' };
      else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (p.startsWith('/api/v1/network/dns01')) {
        if (method === 'GET') { reads++; if (failed) { status = 503; body = { error: 'Fixture: DNS inventory unavailable' }; } else body = malformed ? {} : inventory; }
        else {
          assert.equal(actor, 'admin'); assert.equal(req.headers()['x-csrf-token'], 'fixture');
          const payload = req.postData() ? req.postDataJSON() : null; calls.push({ p, method, payload });
          if (gate) await gate;
          if (reject) { status = 409; body = { error: 'Fixture: CNAME conflict; DNS preserved' }; }
          else if (method === 'PUT') {
            assert.deepEqual(Object.keys(payload).sort(), ['api_token', 'zone_id', 'zone_name']);
            inventory.provider = { configured: true, zone_id: payload.zone_id, zone_name: payload.zone_name, revision: 'fixture-revision', pending: [] }; body = inventory.provider;
          } else if (method === 'DELETE') { inventory.provider = { configured: false, pending: [] }; body = inventory.provider; }
          else if (p.endsWith('/issue')) {
            assert.deepEqual(Object.keys(payload).sort(), ['confirmed', 'email', 'site_id', 'staging']); assert.equal(payload.site_id, 7); assert.equal(payload.confirmed, true);
            const id = `order-${calls.length}`, row = { id, site_id: 7, state: 'queued', staging: payload.staging, error: null, job_id: `job-${id}` };
            inventory.orders.unshift(row); body = newJob(row); jobs.set(body.id, body); status = 202;
          } else if (p.endsWith('/discard')) { assert.equal(payload.hostname, 'app.example.com'); inventory.orders = inventory.orders.filter(o => o.id !== p.split('/').at(-2)); body = { discarded: true }; }
          else if (p.endsWith('/retry')) { const row = inventory.orders.find(o => o.id === p.split('/').at(-2)); body = newJob(row); jobs.set(body.id, body); status = 202; }
          else if (p.endsWith('/cleanup')) { inventory.provider.pending = inventory.provider.pending.filter(c => c.id !== payload.id); body = { cleaned: true }; }
          else throw new Error(`Unexpected DNS mutation ${p}`);
        }
      } else if (p === '/api/v1/network') body = { ipv4: null, ipv6: null, projects: [] };
      else if (p === '/api/v1/network/cloudflare') body = { configured: false, account_id: null, connector_installed: false, managed_tunnels: [] };
      else if (p === '/api/v1/jobs') { const items = [...jobs.values()].filter(j => !u.searchParams.get('status') || j.status === u.searchParams.get('status')); body = { items, total: items.length, page: 1, per_page: 100, total_pages: items.length ? 1 : 0 }; }
      else if (p.startsWith('/api/v1/jobs/')) body = jobs.get(p.split('/').at(-1)) || [];
      else if (p === '/api/v1/connectivity') body = { can_manage: actor === 'admin', page: 1, per_page: 25, total: 0, total_pages: 0, service_counts: {}, endpoints: [], projects: [], certificates: [] };
      else if (p.includes('notifications')) body = { items: [], notifications: [], unread_count: 0 };
      else { status = 503; body = { error: 'Outside fixture' }; assert.equal(method, 'GET'); }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const card = () => page.getByRole('region', { name: 'Sertifikat hostname melalui DNS-01' });
    const dialog = () => page.getByRole('dialog');
    const issue = () => card().getByRole('button', { name: 'Terbitkan atau perbarui hostname', exact: true });
    const queue = () => dialog().getByRole('button', { name: 'Antrekan DNS-01', exact: true });
    const refresh = async () => { await card().getByRole('button', { name: 'Muat ulang DNS-01', exact: true }).click(); };
    await page.goto(`${base}/settings?section=network`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('button', { name: 'Sertifikat DNS-01', exact: true }).click();
    await card().getByText('Belum dihubungkan', { exact: true }).waitFor();
    check(calls.length === 0 && await issue().count() === 0, 'DNS token setup is separate from Tunnel credentials and does not issue automatically');
    await card().getByRole('button', { name: 'Hubungkan token DNS-01' }).click();
    await dialog().getByLabel('Nama zona', { exact: true }).fill('example.com'); await dialog().getByLabel('ID zona', { exact: true }).fill('a'.repeat(32));
    const token = dialog().getByLabel('API token DNS', { exact: true }); await token.fill('fixture_scoped_dns_token_12345');
    check(await token.getAttribute('type') === 'password' && (await dialog().innerText()).includes('Zone DNS Edit'), 'token entry is masked and explains the minimum zone permissions');
    await dialog().getByRole('button', { name: 'Simpan token DNS' }).click(); await dialog().waitFor({ state: 'hidden' }); await issue().waitFor();
    check(!(await card().innerText()).includes('fixture_scoped_dns_token'), 'saved credentials are never displayed in the inventory');
    await issue().click();
    check(await queue().isDisabled() && await dialog().getByRole('checkbox').first().isChecked(), 'issuance defaults to staging and requires project and consent');
    await page.keyboard.press('Escape'); await dialog().waitFor({ state: 'hidden' }); check(await issue().evaluate(e => document.activeElement === e), 'Escape closes the shared modal and restores focus');
    await issue().click(); await dialog().getByLabel('Hostname utama project').selectOption('7'); await dialog().getByLabel('Email akun CA').fill('ops@example.test'); await dialog().getByRole('checkbox').last().check();
    gate = new Promise(r => { release = r; }); await queue().click(); await page.waitForFunction(() => [...document.querySelectorAll('[role="dialog"] button')].every(b => b.disabled));
    await page.keyboard.press('Escape'); check(await dialog().count() === 1 && calls.filter(c => c.p.endsWith('/issue')).length === 1, 'pending submission blocks duplicate issuance and dismissal');
    release(); gate = null; await dialog().waitFor({ state: 'hidden' }); await card().getByRole('progressbar').waitFor();
    await page.reload({ waitUntil: 'domcontentloaded' }); await page.getByRole('button', { name: 'Sertifikat DNS-01', exact: true }).click(); await card().getByRole('progressbar').waitFor();
    check(await issue().isDisabled() && calls.filter(c => c.p.endsWith('/issue')).length === 1, 'navigation restores persisted progress without another certificate order');
    const staged = inventory.orders[0]; staged.state = 'staging'; Object.assign(jobs.get(staged.job_id), { status: 'succeeded', progress: 100 });
    await card().getByText(/app.example.com.*Uji staging selesai, tidak dipasang/).waitFor(); check(inventory.bindings.length === 0, 'staging completion does not install a production certificate');
    await issue().click(); await dialog().getByLabel('Hostname utama project').selectOption('7'); await dialog().getByLabel('Email akun CA').fill('ops@example.test'); await dialog().getByRole('checkbox').first().uncheck(); await dialog().getByRole('checkbox').last().check(); await queue().click(); await dialog().waitFor({ state: 'hidden' });
    const active = inventory.orders[0], now = Math.floor(Date.now() / 1000); active.state = 'active'; Object.assign(jobs.get(active.job_id), { status: 'succeeded', progress: 100 }); inventory.bindings.push({ site_id: 7, certificate_id: 10, not_after: now + 90 * 86400, renew_at: now + 60 * 86400, error: null });
    await card().getByText('app.example.com · Sertifikat produksi', { exact: true }).waitFor(); check((await card().innerText()).includes('Jadwal renewal'), 'production inventory shows expiry and renewal time');
    inventory.orders.unshift({ id: 'failed', site_id: 7, state: 'failed', staging: false, error: 'Fixture renewal failed; active preserved', job_id: 'failed-job' }); await refresh(); await card().getByText('Fixture renewal failed; active preserved', { exact: true }).waitFor();
    check(inventory.bindings[0].certificate_id === 10, 'failed renewal preserves the active binding');
    inventory.provider.pending.push({ id: 'stale-txt', hostname: '_acme-challenge.app.example.com', created_at: now - 2500 }); await refresh(); await card().getByRole('button', { name: 'Bersihkan TXT tertunda', exact: true }).click(); await page.waitForFunction(() => !document.body.innerText.includes('_acme-challenge.app.example.com'));
    check(calls.at(-1).p.endsWith('/cleanup'), 'orphan cleanup is an explicit scoped mutation');
    inventory.orders.unshift({ id: 'candidate', site_id: 7, state: 'candidate', staging: false, error: 'Fixture activation uncertain', job_id: 'candidate-job' }); await refresh(); await card().getByRole('button', { name: 'Buang kandidat', exact: true }).click();
    check(await dialog().getByRole('button', { name: 'Buang kandidat', exact: true }).isDisabled(), 'discard requires the exact project hostname');
    await dialog().getByRole('textbox').fill('app.example.com'); await dialog().getByRole('button', { name: 'Buang kandidat', exact: true }).click(); await dialog().waitFor({ state: 'hidden' }); check(!inventory.orders.some(o => o.id === 'candidate'), 'discard refreshes inventory after restoring the saved origin');
    failed = true; await refresh(); await card().getByRole('alert').waitFor(); check(await issue().count() === 0, 'inventory failure removes stale certificate actions');
    failed = false; malformed = true; await refresh(); await card().getByText(/Inventaris DNS-01 tidak valid/).waitFor(); check(await issue().count() === 0, 'malformed inventory does not expose actions');
    malformed = false; await refresh(); await issue().click(); await dialog().getByLabel('Hostname utama project').selectOption('7'); await dialog().getByLabel('Email akun CA').fill('ops@example.test'); await dialog().getByRole('checkbox').last().check(); reject = true; await queue().click(); await dialog().getByRole('alert').waitFor(); check((await dialog().innerText()).includes('CNAME conflict'), 'DNS conflicts remain visible with the draft intact'); reject = false;
    await page.setViewportSize({ width: 320, height: 568 }); const box = await dialog().locator('.dialog-panel').boundingBox(); check(box.x >= 0 && box.x + box.width <= 321 && box.y >= 0 && box.y + box.height <= 569, 'mobile DNS editor fits the viewport');
    const bodyBox = await dialog().locator('.dialog-body').boundingBox(), footerBox = await dialog().locator('.dialog-footer').boundingBox(); check(bodyBox.height > 0 && bodyBox.y + bodyBox.height <= footerBox.y + 1, 'scroll body stays above the footer without content overlap');
    await dialog().getByRole('checkbox').last().scrollIntoViewIfNeeded(); check(await dialog().getByRole('button', { name: 'Batal', exact: true }).isVisible(), 'mobile scrolling keeps consent and actions reachable'); check(await dialog().locator('.dialog-footer button').evaluateAll(buttons => buttons.every(b => b.scrollWidth <= b.clientWidth + 1)), 'footer labels fit without horizontal clipping');
    await page.waitForFunction(() => !document.querySelector('[data-sonner-toast]')); const artifacts = path.join(__dirname, '../artifacts/dns01'); await fs.mkdir(artifacts, { recursive: true }); await page.screenshot({ path: path.join(artifacts, 'mobile-dialog.png') });
    await dialog().getByRole('button', { name: 'Batal', exact: true }).click(); await page.setViewportSize({ width: 1440, height: 1000 }); await page.screenshot({ path: path.join(artifacts, 'desktop.png'), fullPage: true }); await page.evaluate(() => document.documentElement.classList.add('dark')); await page.screenshot({ path: path.join(artifacts, 'desktop-dark.png'), fullPage: true });
    const before = reads; actor = 'user'; await page.goto(`${base}/connectivity`, { waitUntil: 'domcontentloaded' }); await page.getByTestId('connectivity-settings').waitFor(); check(reads === before && await card().count() === 0, 'tenant view does not read the DNS token or admin inventory');
    check(errors.length === 0, `no browser exceptions: ${errors.join(', ')}`); console.log(`${checks} checks passed`);
  } finally { await browser.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
