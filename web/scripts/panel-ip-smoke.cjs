/* Production bundle plus API fixtures. No ACME request, agent daemon or Nginx. */
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
    let actor = 'admin', rows = [], next = 1, failedLoad = false, malformed = false, conflict = false, gate = null, release, reads = 0;
    const jobs = new Map(), calls = [], errors = [];
    page.on('pageerror', e => errors.push(e.message));
    const endpoint = { id: '00000000-0000-4000-8000-000000000001', revision: 1, service: 'panel', scope: { kind: 'host' }, transport: 'direct', host: '1.1.1.1', port: 19443, tls_mode: 'tls', certificate_id: null, enabled: true, primary: true, client_requirement: 'none', created_at: 1, updated_at: 1 };
    const ipv6 = { ...endpoint, id: '00000000-0000-4000-8000-000000000002', host: '2606:4700:4700::1111', primary: false };
    const certificate = (id, staging, ip) => ({ id, scope: 'panel', ip, port: 19443, staging, state: 'queued', not_before: null, not_after: null, renew_at: null, expired: false, error: null, job_id: `job-${id}` });
    const job = (id, certificateId) => ({ id, user_id: 1, name: 'panel.tls.issue', status: 'queued', progress: 0, payload_json: JSON.stringify({ certificate_id: certificateId }), result_json: null, error_message: null, retry_count: 0, max_retries: 0, created_at: 1, updated_at: 1 });
    const enqueue = (staging, ip) => { const row = certificate(`certificate-${next++}`, staging, ip); rows.unshift(row); jobs.set(row.job_id, job(row.job_id, row.id)); return row; };
    const finish = (row, status) => { const now = Math.floor(Date.now() / 1000); Object.assign(jobs.get(row.job_id), { status, progress: status === 'succeeded' ? 100 : 10, error_message: status === 'failed' ? 'Fixture CA unavailable' : null }); Object.assign(row, status === 'succeeded' ? { state: row.staging ? 'staging' : 'active', not_before: now - 60, not_after: now + 160 * 3600, renew_at: now + 100 * 3600 } : { state: 'failed', error: 'Fixture CA unavailable' }); };
    await context.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url()), p = url.pathname, method = req.method(); let body = {}, status = 200;
      if (p === '/api/v1/auth/me') body = { user: { id: actor === 'admin' ? 1 : 2, username: actor, role: actor }, csrf_token: 'fixture' };
      else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (p === '/api/v1/connectivity') body = { can_manage: actor === 'admin', page: 1, per_page: 25, total: actor === 'admin' ? 2 : 0, total_pages: actor === 'admin' ? 1 : 0, service_counts: actor === 'admin' ? { panel: 2 } : {}, endpoints: actor === 'admin' ? [endpoint, ipv6].map(endpoint => ({ endpoint, evidence: [] })) : [], projects: [], certificates: [] };
      else if (p === '/api/v1/connectivity/panel-url') body = { revision: 0, canonical_url: null, current_url: base, endpoint_id: null, endpoint_revision: null, aliases: [] };
      else if (p === '/api/v1/connectivity/panel-tls' && method === 'GET') {
        reads++; if (failedLoad) { status = 503; body = { detail: 'Fixture: sertifikat belum dapat dimuat.' }; }
        else body = malformed ? {} : { certificates: rows, profile: 'shortlived', challenge_port: 80 };
      } else if (p.startsWith('/api/v1/connectivity/panel-tls/') && method === 'POST') {
        assert.equal(actor, 'admin'); assert.equal(req.headers()['x-csrf-token'], 'fixture'); calls.push({ p, body: req.postData() ? req.postDataJSON() : null });
        if (gate) await gate;
        if (conflict) { status = 409; body = { detail: 'Fixture: revisi endpoint berubah.' }; }
        else if (p.endsWith('/issue')) {
          const payload = req.postDataJSON(); assert.deepEqual(Object.keys(payload).sort(), ['email', 'endpoint_id', 'expected_revision', 'http01_confirmed', 'staging']); assert.equal(payload.http01_confirmed, true); assert.equal(payload.expected_revision, 1);
          const row = enqueue(payload.staging, payload.endpoint_id === ipv6.id ? ipv6.host : endpoint.host); body = { job_id: row.job_id, certificate_id: row.id }; status = 202;
        } else if (p.endsWith('/renew')) { const row = enqueue(false, rows.find(c => c.state === 'active').ip); body = { job_id: row.job_id, certificate_id: row.id }; status = 202; }
        else if (p.endsWith('/discard')) { rows = rows.filter(c => c.id !== p.split('/').at(-2)); body = { discarded: true }; }
        else if (p.endsWith('/retry')) { const row = rows.find(c => c.id === p.split('/').at(-2)); body = { job_id: row.job_id, certificate_id: row.id }; status = 202; }
        else throw new Error(`Unexpected TLS mutation ${p}`);
      } else if (p === '/api/v1/jobs') {
        const items = [...jobs.values()].filter(j => !url.searchParams.get('status') || j.status === url.searchParams.get('status')); body = { items, total: items.length, page: 1, per_page: 100, total_pages: items.length ? 1 : 0 };
      } else if (p.startsWith('/api/v1/jobs/')) body = jobs.get(p.split('/').at(-1)) || [];
      else if (p.includes('notifications')) body = { items: [], notifications: [], unread_count: 0 };
      else { status = 503; body = { detail: 'Outside fixture' }; assert.equal(method, 'GET'); }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const card = () => page.getByRole('region', { name: 'Sertifikat IP panel', exact: true });
    const dialog = () => page.getByRole('dialog');
    const issue = () => card().getByRole('button', { name: 'Terbitkan sertifikat IP', exact: true });
    const queue = () => dialog().getByRole('button', { name: 'Antrekan penerbitan', exact: true });
    await page.goto(`${base}/settings?section=connectivity`, { waitUntil: 'domcontentloaded' });
    await card().getByText('Belum ada sertifikat IP panel.', { exact: true }).waitFor(); await issue().click();
    check(calls.length === 0 && await queue().isDisabled(), 'opening the editor does not issue a certificate and requires acknowledgement');
    check(await dialog().getByRole('checkbox').first().isChecked(), 'staging is selected and explicitly does not install test certificates');
    await dialog().getByLabel('Alamat panel', { exact: true }).selectOption(ipv6.id); await dialog().getByLabel('Email akun CA').fill('ops@example.test');
    await dialog().getByRole('checkbox').last().check();
    await page.keyboard.press('Escape'); await dialog().waitFor({ state: 'hidden' });
    check(await issue().evaluate(el => el === document.activeElement), 'Escape returns keyboard focus to the opener');
    await issue().click(); await dialog().getByLabel('Alamat panel', { exact: true }).selectOption(ipv6.id); await dialog().getByLabel('Email akun CA').fill('ops@example.test'); await dialog().getByRole('checkbox').last().check();
    gate = new Promise(r => { release = r; }); await queue().click(); await page.waitForFunction(() => [...document.querySelectorAll('[role="dialog"] button')].every(b => b.disabled)); await page.keyboard.press('Escape');
    check(calls.length === 1 && await dialog().count() === 1, 'pending issuance blocks duplicate submission and premature dismissal');
    release(); gate = null; await dialog().waitFor({ state: 'hidden' }); await card().getByRole('progressbar').waitFor();
    check(await issue().isDisabled() && calls[0].body.endpoint_id === ipv6.id, 'persisted pending state locks issuance and retains IPv6 by endpoint ID');
    await page.reload({ waitUntil: 'domcontentloaded' }); await card().getByRole('progressbar').waitFor();
    check(calls.length === 1 && await issue().isDisabled(), 'navigation and reload recover the existing job without another mutation');
    const staged = rows[0]; finish(staged, 'succeeded');
    await card().getByText(/Uji staging selesai, tidak dipasang/).waitFor();
    check(await card().getByRole('button', { name: 'Perbarui sertifikat panel' }).count() === 0, 'staging result never becomes an active production certificate');
    await issue().click(); await dialog().getByRole('checkbox').first().uncheck(); await dialog().getByLabel('Email akun CA').fill('ops@example.test'); await dialog().getByRole('checkbox').last().check();
    check((await dialog().innerText()).includes('sertifikat aktif sebelumnya dipertahankan'), 'production preview explains candidate tests and preservation of the previous certificate');
    await queue().click(); await dialog().waitFor({ state: 'hidden' }); const production = rows[0]; finish(production, 'succeeded');
    await card().getByText(/1\.1\.1\.1:19443.*Inventaris aktif/).waitFor();
    check((await card().innerText()).includes('160 jam tersisa') && (await card().innerText()).includes('Jadwal renewal'), 'active inventory shows exact expiry and a lifetime-aware renewal schedule');
    await Promise.all([page.waitForResponse(response => response.url().endsWith('/panel-tls/renew') && response.status() === 202), card().getByRole('button', { name: 'Perbarui sertifikat panel', exact: true }).click()]); const failed = rows[0]; finish(failed, 'failed');
    await card().getByText('Fixture CA unavailable', { exact: true }).waitFor();
    check(rows.some(c => c.id === production.id && c.state === 'active'), 'failed renewal retains the previously active certificate');
    const validUntil = production.not_after; production.not_after = Math.floor(Date.now() / 1000) - 1;
    await card().getByRole('button', { name: 'Muat ulang sertifikat panel' }).click(); await card().getByText(/1\.1\.1\.1:19443.*Kedaluwarsa/).waitFor();
    check((await card().innerText()).includes('0 jam tersisa'), 'expired active inventory is clearly marked instead of presenting a ready certificate');
    production.not_after = validUntil;
    const candidate = certificate('candidate-fixture', false, '1.1.1.1'); candidate.state = 'candidate'; rows.unshift(candidate);
    await card().getByRole('button', { name: 'Muat ulang sertifikat panel' }).click(); await card().getByRole('button', { name: 'Buang kandidat', exact: true }).click();
    check(await dialog().getByRole('button', { name: 'Buang kandidat', exact: true }).isDisabled(), 'discarding an unresolved candidate requires typing its IP');
    await dialog().getByRole('textbox').fill(candidate.ip); await dialog().getByRole('button', { name: 'Buang kandidat', exact: true }).click(); await dialog().waitFor({ state: 'hidden' });
    check(!rows.some(c => c.id === candidate.id), 'discard sends a separate mutation and refreshes inventory');
    failedLoad = true; await card().getByRole('button', { name: 'Muat ulang sertifikat panel' }).click(); await card().getByRole('alert').waitFor();
    check(await issue().count() === 0, 'a failed inventory refresh removes stale issuance and renewal actions');
    failedLoad = false; malformed = true; await card().getByRole('button', { name: 'Muat ulang sertifikat panel' }).click(); await card().getByText('Inventaris sertifikat panel tidak valid.', { exact: true }).waitFor();
    check(await issue().count() === 0, 'malformed inventory is rejected without a browser crash');
    malformed = false; await card().getByRole('button', { name: 'Muat ulang sertifikat panel' }).click(); await issue().click();
    await dialog().getByLabel('Email akun CA').fill('ops@example.test'); await dialog().getByRole('checkbox').last().check(); conflict = true; await queue().click(); await dialog().getByRole('alert').waitFor();
    check((await dialog().getByRole('alert').innerText()).includes('revisi endpoint berubah'), 'endpoint conflicts remain visible with the editable draft');
    await dialog().getByRole('button', { name: 'Batal', exact: true }).click(); conflict = false; await issue().click();
    await page.setViewportSize({ width: 320, height: 568 }); const box = await dialog().locator('.dialog-panel').boundingBox();
    check(box.x >= 0 && box.x + box.width <= 321 && box.y >= 0 && box.y + box.height <= 569, 'the mobile editor fits the viewport with a scrolling body');
    await dialog().getByRole('checkbox').last().scrollIntoViewIfNeeded(); check(await dialog().getByRole('button', { name: 'Batal' }).isVisible(), 'mobile scrolling keeps acknowledgement and footer actions reachable');
    await page.waitForFunction(() => !document.querySelector('[data-sonner-toast]'));
    const artifacts = path.join(__dirname, '../artifacts/panel-ip'); await fs.mkdir(artifacts, { recursive: true }); await page.screenshot({ path: path.join(artifacts, 'mobile-dialog.png') });
    await dialog().getByRole('button', { name: 'Batal', exact: true }).click(); await dialog().waitFor({ state: 'hidden' }); await page.setViewportSize({ width: 1440, height: 1000 }); await page.screenshot({ path: path.join(artifacts, 'desktop.png'), fullPage: true });
    await page.evaluate(() => document.documentElement.classList.add('dark')); await page.screenshot({ path: path.join(artifacts, 'desktop-dark.png'), fullPage: true });
    const previousReads = reads; actor = 'user'; await page.goto(`${base}/connectivity`, { waitUntil: 'domcontentloaded' }); await page.getByTestId('connectivity-settings').waitFor();
    check(await card().count() === 0 && reads === previousReads, 'tenant view never loads the panel certificate inventory or admin controls');
    check(errors.length === 0, `no browser exceptions: ${errors.join(', ')}`); console.log(`${checks} checks passed`);
  } finally { await browser.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
