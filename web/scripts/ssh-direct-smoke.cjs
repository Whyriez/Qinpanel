/* Static production UI and API fixtures only; no SSH client, login or agent. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5199';
let checks = 0;
const check = (ok, label) => { assert.ok(ok, label); console.log(`ok ${++checks}: ${label}`); };
const sample = {
  host: '2001:db8::1', port: 2222, username: 'p_actor1', user_id: 2,
  home_dir: '/home/p_actor1', active_path: '/home/p_actor1/sites/bob\'s "$HOME" [app]', relative_path: 'sites/app',
  sftp_path: '/home/p_actor1/sites/bob\'s "$HOME" [app]', shell_policy: 'full', can_shell: true, can_sftp: true,
  has_ssh_keys: true, ssh_keys_count: 1, endpoint_state: 'configured', endpoint_id: 'ssh-global', endpoint_revision: 3,
  transport: 'direct', host_keys_status: 'local_configuration', host_keys: [{ key_type: 'ssh-ed25519', fingerprint_sha256: 'SHA256:fixturehost' }],
  ssh_command: 'ssh -p 2222 -t p_actor1@2001:db8::1 \'cd -- /home/p_actor1 && exec "$SHELL" -l\'',
  sftp_command: "sftp -P 2222 'p_actor1@[2001:db8::1]'",
  sftp_cd_command: 'cd "/home/p_actor1/sites/bob\'s \\"$HOME\\" [app]"',
  sftp_url: 'sftp://p_actor1@[2001:db8::1]:2222/home/p_actor1/sites/bob%27s%20%22%24HOME%22%20%5Bapp%5D/',
  ssh_config_snippet: 'Host p_actor1-server\n    HostName 2001:db8::1\n    User p_actor1\n    Port 2222\n    StrictHostKeyChecking ask\n', site_domain: 'website-tunnel.example.com',
};
const site = { id: 7, user_id: 2, kind: 'static', runtime: null, version: null, status: 'active', docroot: '/home/p_actor1/sites/app/public', primary_domain: 'website-tunnel.example.com', domains: [], settings_json: null, force_https: false, ssl_enabled: false, created_at: 1, updated_at: 1 };
(async () => {
  const browser = await chromium.launch({ headless: true, channel: 'msedge' });
  try {
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
    await context.addInitScript(() => { Object.defineProperty(navigator, 'clipboard', { value: { writeText: async value => { window.__copied = value; } } }); });
    const page = await context.newPage(); page.setDefaultTimeout(15000);
    const errors = [], calls = []; let info = { ...sample }, fail = false, gate, release;
    page.on('pageerror', e => errors.push(e.message));
    await context.route('**/api/v1/**', async route => {
      const req = route.request(), u = new URL(req.url()), p = u.pathname;
      calls.push({ p, method: req.method(), query: u.search }); assert.equal(req.method(), 'GET');
      let body = {}, status = 200;
      if (p === '/api/v1/auth/me') body = { user: { id: 2, username: 'actor1', role: 'user', status: 'active' }, csrf_token: 'fixture' };
      else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (p === '/api/v1/files/list') body = { entries: [], total_count: 0, has_more: false };
      else if (p === '/api/v1/sites') body = [site];
      else if (p === '/api/v1/sites/7') body = site;
      else if (p === '/api/v1/ssh/connection-info') {
        if (gate) await gate;
        if (fail) { status = 503; body = { error: 'Fixture: endpoint unavailable' }; }
        else if (u.searchParams.has('host')) body = { ...info, host: u.searchParams.get('host'), port: Number(u.searchParams.get('port')), endpoint_state: 'manual_preview', endpoint_id: null, endpoint_revision: null, host_keys: [], host_keys_status: 'untrusted_override', ssh_command: 'ssh fixture-manual', sftp_command: 'sftp fixture-manual' };
        else body = u.searchParams.get('site_id') === '7' ? { ...info, host: 'project-ssh.example.com', port: 2200, endpoint_id: 'ssh-project' } : info;
      }
      else if (p.includes('notifications')) body = { items: [], notifications: [], unread_count: 0 };
      else if (p === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 100, total_pages: 0 };
      else { status = 503; body = { error: 'Outside fixture' }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const dialog = () => page.getByRole('dialog', { name: 'Koneksi SSH & SFTP' });
    const open = async () => { await page.getByRole('button', { name: 'Salin SSH', exact: true }).click(); await dialog().getByText('Username Linux', { exact: true }).waitFor(); };
    const close = async () => { await page.keyboard.press('Escape'); await dialog().waitFor({ state: 'hidden' }); };
    await page.goto(`${base}/files`, { waitUntil: 'domcontentloaded' }); await open();
    check((await dialog().innerText()).includes('2001:db8::1') && (await dialog().innerText()).includes('2222'), 'shared File Manager dialog displays explicit IPv6 host and custom port');
    check(!(await dialog().innerText()).includes('website-tunnel.example.com'), 'website domain never becomes the direct SSH host');
    check((await dialog().innerText()).includes('SHA256:fixturehost') && (await dialog().innerText()).includes('belum membuktikan'), 'local fingerprint is separate from remote reachability');
    await dialog().getByRole('button', { name: 'Salin Perintah SSH', exact: true }).click();
    check(await page.evaluate(() => window.__copied) === sample.ssh_command, 'clipboard receives the server-generated SSH command unchanged');
    check(await dialog().getByRole('button', { name: 'Buka terminal browser', exact: true }).count() === 1, 'browser terminal fallback is available under full shell policy');
    await dialog().getByRole('tab', { name: 'Perintah', exact: true }).focus(); await page.keyboard.press('ArrowRight');
    check(await dialog().getByRole('tab', { name: 'Client SFTP', selected: true }).count() === 1, 'arrow keys move and activate guide tabs');
    await dialog().getByRole('button', { name: 'Salin URL SFTP', exact: true }).click();
    check(await page.evaluate(() => window.__copied) === sample.sftp_url, 'IPv6 and encoded folder URL can be copied');
    await page.keyboard.press('End'); // Focus is now on copy; explicitly focus the tab first.
    await dialog().getByRole('tab', { name: 'Client SFTP', exact: true }).focus(); await page.keyboard.press('End');
    check(await dialog().getByRole('tab', { name: 'Konfigurasi OpenSSH', selected: true }).count() === 1, 'End selects the last tab');
    await dialog().getByRole('button', { name: 'Salin Konfigurasi OpenSSH', exact: true }).click();
    check(await page.evaluate(() => window.__copied) === sample.ssh_config_snippet, 'client config keeps raw IPv6 and actual endpoint port');
    await fs.mkdir(path.resolve('target'), { recursive: true }); await page.screenshot({ path: 'target/ssh-direct-desktop.png' });
    await close(); check(await page.getByRole('button', { name: 'Salin SSH', exact: true }).evaluate(e => document.activeElement === e), 'Escape restores focus to the File Manager trigger');
    info = { ...sample, endpoint_state: 'missing', host: null, port: null, host_keys: [], host_keys_status: 'not_applicable', ssh_command: null, sftp_command: null, sftp_cd_command: null, sftp_url: null, ssh_config_snippet: null };
    await open(); check((await dialog().innerText()).includes('Endpoint SSH belum diatur') && await dialog().getByRole('tab').count() === 0, 'missing endpoint has no fabricated command or default host'); await close();
    info = { ...info, endpoint_state: 'unsupported_transport', transport: 'cloudflare_access' };
    await open(); check((await dialog().innerText()).includes('Cloudflare Access') && await dialog().getByRole('tab').count() === 0, 'Access endpoint is not mislabeled as direct'); await close();
    info = { ...sample, shell_policy: 'sftp', can_shell: false, ssh_command: null, sftp_path: '/p_actor1/sites/app' };
    await open(); check(await dialog().getByRole('button', { name: 'Salin Perintah SSH', exact: true }).count() === 0 && await dialog().getByRole('button', { name: 'Buka terminal browser' }).count() === 0 && (await dialog().innerText()).includes('/p_actor1/sites/app'), 'SFTP-only policy hides interactive shell and shows the chroot path'); await close();
    info = { ...sample, shell_policy: 'none', can_shell: false, can_sftp: false };
    await open(); check(await dialog().getByRole('tab').count() === 0 && await dialog().getByRole('button', { name: 'Buka terminal browser' }).count() === 0 && !(await dialog().innerText()).includes('SHA256:fixturehost'), 'none policy hides every remote guide and fingerprint'); await close();
    info = { ...sample, has_ssh_keys: false, ssh_keys_count: 0, host_keys: [], host_keys_status: 'unavailable' };
    await open(); check((await dialog().innerText()).includes('Belum ada kunci SSH terdaftar') && (await dialog().innerText()).includes('Fingerprint belum tersedia'), 'missing tenant key and host identity have independent guidance'); await close();
    info = { ...sample }; await open(); await dialog().getByText('Pratinjau host dan port manual', { exact: true }).click();
    await dialog().getByLabel('Host manual', { exact: true }).fill('manual.example.com'); await dialog().getByLabel('Port manual', { exact: true }).fill('2200');
    check(calls.filter(c => c.query.includes('manual.example.com')).length === 0, 'typing manual input does not publish or probe an endpoint');
    await dialog().getByRole('button', { name: 'Buat pratinjau', exact: true }).click(); await dialog().getByText(/Pratinjau manual: alamat/).waitFor();
    check(!(await dialog().innerText()).includes('SHA256:fixturehost') && (await dialog().innerText()).includes('manual.example.com'), 'manual preview cannot borrow the registered host fingerprint');
    await dialog().getByRole('button', { name: 'Pakai endpoint terdaftar', exact: true }).click(); await dialog().getByText('SHA256:fixturehost', { exact: true }).waitFor(); check((await dialog().innerText()).includes('2001:db8::1'), 'reset reloads the server-selected endpoint'); await close();
    fail = true; await page.getByRole('button', { name: 'Salin SSH', exact: true }).click(); await dialog().getByRole('alert').waitFor();
    check(await dialog().getByRole('tab').count() === 0 && !(await dialog().innerText()).includes('SHA256:fixturehost'), 'failed reload does not leave stale commands'); fail = false; await dialog().getByRole('button', { name: 'Coba lagi', exact: true }).click(); await dialog().getByText('Username Linux', { exact: true }).waitFor(); await close();
    gate = new Promise(resolve => { release = resolve; }); await page.getByRole('button', { name: 'Salin SSH', exact: true }).click(); await dialog().getByText('Memuat informasi koneksi...', { exact: true }).waitFor();
    check(await dialog().getByRole('tab').count() === 0, 'pending response does not fabricate or retain commands'); await close(); release(); gate = null;
    await page.setViewportSize({ width: 390, height: 844 }); await open();
    await dialog().evaluate(async el => { await Promise.all(el.getAnimations({ subtree: true }).filter(a => a.effect?.getTiming().iterations !== Infinity).map(a => a.finished.catch(() => {}))); });
    const bounds = await dialog().evaluate(el => { const panel = el.querySelector('.dialog-panel'), body = el.querySelector('.dialog-body'), footer = el.querySelector('.dialog-footer'); return { panel: panel.getBoundingClientRect().toJSON(), body: body.getBoundingClientRect().toJSON(), footer: footer.getBoundingClientRect().toJSON(), scroll: body.scrollHeight > body.clientHeight, overflow: document.documentElement.scrollWidth > innerWidth }; });
    check(bounds.panel.x >= 0 && bounds.panel.right <= 390 && bounds.footer.bottom <= 844 && bounds.body.bottom <= bounds.footer.top + 1 && bounds.scroll && !bounds.overflow, 'mobile dialog scrolls its body while keeping footer visible without page overflow');
    await page.screenshot({ path: 'target/ssh-direct-mobile.png' });
    await dialog().getByRole('button', { name: 'Salin Buka folder di SFTP', exact: true }).click();
    check(await page.evaluate(() => window.__copied) === sample.sftp_cd_command, 'special folder command copies intact on mobile');
    await close();
    await page.setViewportSize({ width: 1440, height: 1000 }); await page.goto(`${base}/sites/7/files`, { waitUntil: 'domcontentloaded' });
    await open(); check((await dialog().innerText()).includes('project-ssh.example.com') && calls.filter(c => c.p.endsWith('/ssh/connection-info')).at(-1).query.includes('site_id=7'), 'nested Site File Manager preserves project endpoint scope'); await close();
    await page.getByRole('button', { name: 'SSH / SFTP', exact: true }).click(); await dialog().getByText('Username Linux', { exact: true }).waitFor();
    check((await dialog().innerText()).includes('project-ssh.example.com') && calls.filter(c => c.p.endsWith('/ssh/connection-info')).at(-1).query.includes('site_id=7'), 'Site Detail header uses the same project-scoped SSH endpoint'); await close();
    check(errors.length === 0 && calls.every(c => c.method === 'GET'), 'no browser runtime error or endpoint/provider mutation');
    console.log(`Passed ${checks} local SSH UI checks`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
