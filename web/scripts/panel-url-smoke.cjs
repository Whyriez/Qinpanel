/* Built UI and intercepted API fixtures only. No probe, proxy or panel daemon. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5199';
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
(async () => {
  const browser = await chromium.launch({ headless: true, channel: 'msedge' });
  try {
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
    const page = await context.newPage(); page.setDefaultTimeout(15000);
    const errors = [], calls = []; page.on('pageerror', e => errors.push(e.message));
    let failLoad = false, failActivate = false, release;
    let gate = null;
    let policy = { revision: 0, canonical_url: null, current_url: base, endpoint_id: null, endpoint_revision: null, aliases: [] };
    const endpoint = { id: '00000000-0000-4000-8000-000000000001', revision: 1, service: 'panel', scope: { kind: 'host' }, transport: 'direct', host: '2001:db8::7', port: 19443, tls_mode: 'tls', certificate_id: null, enabled: true, primary: true, client_requirement: 'none', created_at: 1, updated_at: 1 };
    await context.route('**/api/v1/**', async route => {
      const req = route.request(), p = new URL(req.url()).pathname, method = req.method();
      let body = {}, status = 200;
      if (p === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (p === '/api/v1/connectivity') body = { can_manage: true, page: 1, per_page: 25, total: 1, total_pages: 1, service_counts: { panel: 1 }, endpoints: [{ endpoint, evidence: [] }], projects: [], certificates: [] };
      else if (p === '/api/v1/connectivity/panel-tls' && method === 'GET') body = { certificates: [], profile: 'shortlived', challenge_port: 80 };
    else if (p === '/api/v1/connectivity/panel-url' && method === 'GET') {
        if (failLoad) { status = 503; body = { detail: 'Fixture: pengaturan belum tersedia.' }; } else body = policy;
      } else if (p === '/api/v1/connectivity/panel-url/activate') {
        const payload = req.postDataJSON(); calls.push({ p, payload }); assert.equal(req.headers()['x-csrf-token'], 'fixture');
        assert.deepEqual(payload, { endpoint_id: endpoint.id, expected_endpoint_revision: 1, expected_revision: policy.revision, reachable_confirmed: true });
        if (gate) await gate;
        if (failActivate) { status = 409; body = { detail: 'Fixture: revisi berubah, muat ulang pengaturan.' }; }
        else body = policy = { ...policy, revision: policy.revision + 1, canonical_url: 'https://[2001:db8::7]:19443', endpoint_id: endpoint.id, endpoint_revision: 1, aliases: [base, 'https://old.panel.test:8443'] };
      } else if (p === '/api/v1/connectivity/panel-url/revoke') {
        const payload = req.postDataJSON(); calls.push({ p, payload }); assert.equal(req.headers()['x-csrf-token'], 'fixture');
        assert.equal(payload.expected_revision, policy.revision); assert.equal(payload.origin, 'https://old.panel.test:8443');
        body = policy = { ...policy, revision: policy.revision + 1, aliases: [base] };
      } else if (p === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 1, total_pages: 0 };
      else if (p.includes('notifications')) body = { items: [], notifications: [], unread_count: 0 };
      else { status = 503; body = { detail: 'Outside fixture' }; assert.equal(method, 'GET'); }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const card = () => page.getByRole('region', { name: 'URL utama panel', exact: true });
    const dialog = () => page.getByRole('dialog');
    const open = () => card().getByRole('button', { name: /Aktifkan URL utama/ }).click();
    await page.goto(`${base}/settings?section=connectivity`, { waitUntil: 'domcontentloaded' });
    await card().getByText('Belum diaktifkan', { exact: true }).waitFor();
    check(calls.length === 0, 'opening settings never activates a draft');
    await open();
    check(await dialog().getByRole('button', { name: 'Aktifkan URL', exact: true }).isDisabled(), 'explicit access acknowledgement is required');
    const link = dialog().getByRole('link');
    check(await link.getAttribute('href') === 'https://[2001:db8::7]:19443/connectivity/verify' && (await link.getAttribute('rel')).includes('noopener'), 'precheck preserves IPv6 and custom port and isolates the new tab');
    await page.keyboard.press('Escape'); await dialog().waitFor({ state: 'hidden' });
    check(await card().getByRole('button', { name: /Aktifkan URL utama/ }).evaluate(el => el === document.activeElement), 'Escape restores opener focus');
    await open(); await dialog().getByRole('checkbox').check();
    gate = new Promise(r => { release = r; });
    await dialog().getByRole('button', { name: 'Aktifkan URL', exact: true }).click();
    await page.waitForFunction(() => [...document.querySelectorAll('[role="dialog"] button')].every(b => b.disabled));
    await page.keyboard.press('Escape');
    check(await dialog().count() === 1 && calls.length === 1, 'pending activation prevents dismissal and duplicate submission');
    release(); gate = null;
    await card().getByText('https://[2001:db8::7]:19443', { exact: true }).waitFor();
    check(await card().getByRole('button', { name: `Cabut akses alamat ${base}`, exact: true }).isDisabled(), 'the current address cannot be revoked');
    await page.reload({ waitUntil: 'domcontentloaded' });
    await card().getByRole('code').filter({ hasText: 'https://old.panel.test:8443' }).waitFor();
    check(await card().getByRole('button', { name: /Aktifkan URL utama/ }).isDisabled(), 'activated snapshot persists and cannot be submitted unchanged');
    await card().getByRole('button', { name: 'Cabut akses alamat https://old.panel.test:8443', exact: true }).click();
    check(await dialog().getByRole('button', { name: 'Cabut akses', exact: true }).isDisabled(), 'revocation requires typing the exact transition address');
    await dialog().getByRole('textbox').fill('https://old.panel.test:8443');
    await dialog().getByRole('button', { name: 'Cabut akses', exact: true }).click();
    await dialog().waitFor({ state: 'hidden' });
    check(policy.aliases.length === 1 && calls.length === 2, 'revocation sends the policy revision and retains the current address');
    policy = { ...policy, endpoint_revision: 0 }; failActivate = true;
    await page.reload({ waitUntil: 'domcontentloaded' }); await open(); await dialog().getByRole('checkbox').check();
    await dialog().getByRole('button', { name: 'Aktifkan URL', exact: true }).click();
    await dialog().getByRole('alert').waitFor();
    check((await dialog().getByRole('alert').innerText()).includes('revisi berubah'), 'revision conflict is visible without losing the preview');
    await dialog().getByRole('button', { name: 'Batal', exact: true }).click();
    await dialog().waitFor({ state: 'hidden' });
    failLoad = true; await card().getByRole('button', { name: 'Muat ulang pengaturan URL' }).click();
    await card().getByRole('alert').waitFor();
    check(await card().getByRole('button', { name: /Aktifkan URL utama/ }).count() === 0, 'failed reload removes stale activation controls');
    failLoad = false; failActivate = false;
    await card().getByRole('button', { name: 'Muat ulang pengaturan URL' }).click(); await open();
    await page.setViewportSize({ width: 320, height: 568 });
    const box = await dialog().locator('.dialog-panel').boundingBox();
    check(box.x >= 0 && box.x + box.width <= 321 && box.y >= 0 && box.y + box.height <= 569, 'mobile dialog fits the visual viewport');
    await dialog().getByRole('checkbox').scrollIntoViewIfNeeded();
    check(await dialog().getByRole('checkbox').isVisible() && await dialog().getByRole('button', { name: 'Batal', exact: true }).isVisible(), 'mobile body scroll keeps acknowledgement and actions reachable');
    await fs.mkdir(path.join(__dirname, '../artifacts/panel-url'), { recursive: true });
    await page.screenshot({ path: path.join(__dirname, '../artifacts/panel-url/mobile-dialog.png') });
    await page.keyboard.press('Escape'); await page.setViewportSize({ width: 1440, height: 1000 });
    await page.screenshot({ path: path.join(__dirname, '../artifacts/panel-url/desktop.png'), fullPage: true });
    await page.evaluate(() => document.documentElement.classList.add('dark'));
    await page.screenshot({ path: path.join(__dirname, '../artifacts/panel-url/desktop-dark.png'), fullPage: true });
    check(errors.length === 0, `no browser errors: ${errors.join(', ')}`);
    console.log(`${checks} checks passed`);
  } finally { await browser.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
