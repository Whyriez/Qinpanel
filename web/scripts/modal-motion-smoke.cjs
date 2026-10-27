/* Browser QA with intercepted API fixtures. Never connects to panel-api or panel-agent. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5179';
const output = path.resolve(__dirname, '../artifacts/modal-motion');
fs.mkdirSync(output, { recursive: true });
const plans = Array.from({ length: 24 }, (_, index) => ({
  id: index + 1, name: `Paket uji ${index + 1}`, description: 'Data uji untuk memeriksa daftar panjang dan batas scroll modal.',
  cpu_quota_percent: 100, memory_limit_bytes: 1073741824, disk_limit_bytes: 10737418240,
  tasks_max: 100, io_weight: 100, inode_limit: 50000, max_sites: 5, max_domains: 5,
  max_databases: 2, max_cron_jobs: 3, max_terminal_sessions: 2, max_upload_bytes: 10485760,
  shell_policy: 'full', allow_external_upstream: false, created_at: 1, updated_at: 1,
}));
const tenant = { id: 42, username: 'customer', linux_username: 'p_customer', linux_uid: 10042, role: 'user', status: 'active', plan_id: 1, plan_name: plans[0].name, created_at: 1 };
let assertions = 0;
const errors = [];
const submissions = [];
function check(condition, message) { assert.ok(condition, message); assertions++; }

async function fixture(page, theme) {
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript((value) => localStorage.setItem('nusa_theme', value), theme);
  await page.route('**/api/v1/**', async (route) => {
    const url = new URL(route.request().url());
    let body = {}, status = 200;
    if (url.pathname.endsWith('/auth/me')) body = { user: { id: 1, username: 'admin', role: 'admin', email: 'admin@example.test', totp_enabled: false } };
    else if (url.pathname.endsWith('/auth/csrf')) body = { csrf_token: 'ui-fixture' };
    else if (url.pathname.endsWith('/system/info')) body = { api_version: '0.1.1', agent_version: '0.1.1', proto_version: 1, agent_connected: true, os: 'linux', arch: 'x86_64', environment: 'testing' };
    else if (url.pathname === '/api/v1/system/hardening') body = { cgroup_v2: { ok: true }, hidepid: { ok: true }, tenant_group: { ok: true }, quota: { ok: true }, acl: { ok: true } };
    else if (url.pathname === '/api/v1/plans') body = { plans };
    else if (url.pathname === '/api/v1/tenants' && route.request().method() === 'POST') {
      submissions.push(route.request().postDataJSON()); status = 400;
      body = { error: 'Pesan error fixture untuk memeriksa scroll isi modal. '.repeat(16) };
    } else if (url.pathname === '/api/v1/tenants') body = { tenants: [tenant] };
    else if (url.pathname === '/api/v1/sites') body = [];
    else if (url.pathname.includes('/php/versions')) body = { ppa_configured: true, versions: [] };
    else if (url.pathname === '/api/v1/runtimes/catalog') body = { enabled: true, interval_hours: 24, sources: [] };
    else if (url.pathname === '/api/v1/runtimes/node/versions') body = { versions: [{ version: '24.1.12', is_lts: true, lts_codename: 'Fixture', is_installed: false, support_status: 'lts' }], installed_count: 0 };
    else if (url.pathname === '/api/v1/runtimes/python/versions') body = { catalog_ready: true, versions: [{ version: '3.14.8', major_minor: '3.14', is_default: true, is_lts_or_stable: true, is_installed: false }] };
    else if (url.pathname.includes('unread-count')) body = { count: 0, unread_count: 0 };
    else if (url.pathname.includes('/jobs')) body = { items: [], total: 0 };
    else if (url.pathname.includes('/notifications')) body = { notifications: [], total: 0 };
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
}

async function geometry(page, label, requireScroll = false) {
  const rects = await page.locator('.dialog-overlay').last().evaluate((overlay) => {
    const panel = overlay.querySelector('.dialog-panel');
    const body = panel.querySelector('.dialog-body');
    const header = panel.querySelector('.dialog-header');
    const footer = panel.querySelector('.dialog-footer');
    const rect = (element) => { const r = element.getBoundingClientRect(); return { x: r.x, y: r.y, right: r.right, bottom: r.bottom, height: r.height }; };
    return { panel: rect(panel), body: rect(body), header: rect(header), footer: rect(footer), scrollHeight: body.scrollHeight, clientHeight: body.clientHeight, panelOverflow: panel.scrollHeight - panel.clientHeight, horizontalOverflow: panel.scrollWidth - panel.clientWidth, panelScrollLeft: panel.scrollLeft, overlayOverflow: overlay.scrollHeight - overlay.clientHeight, bodyWidth: body.clientWidth, bodyScrollWidth: body.scrollWidth, width: innerWidth, height: innerHeight, rootInert: document.getElementById('root').inert };
  });
  check(rects.panel.x >= 11 && rects.panel.right <= rects.width - 11, `${label}: horizontal viewport margins`);
  check(rects.panel.y >= 11 && rects.panel.bottom <= rects.height - 11, `${label}: vertical viewport margins ${JSON.stringify(rects)}`);
  check(rects.footer.bottom <= rects.panel.bottom + 1 && rects.header.y >= rects.panel.y - 1, `${label}: header and actions stay visible`);
  check(rects.panelOverflow <= 1 && rects.overlayOverflow <= 1, `${label}: panel and backdrop do not scroll`);
  check(rects.bodyScrollWidth <= rects.bodyWidth + 1, `${label}: no horizontal body overflow`);
  check(rects.horizontalOverflow <= 1 && rects.panelScrollLeft === 0, `${label}: no hidden horizontal clipping`);
  check(rects.header.x >= rects.panel.x && rects.header.right <= rects.panel.right + 1, `${label}: header stays inside the panel`);
  check(rects.rootInert, `${label}: background is inert`);
  if (requireScroll) check(rects.scrollHeight > rects.clientHeight + 10, `${label}: long body scrolls`);
  const before = rects;
  await page.locator('.dialog-overlay').last().locator('.dialog-body').evaluate((body) => { body.scrollTop = body.scrollHeight; });
  const after = await page.locator('.dialog-overlay').last().evaluate((overlay) => {
    const header = overlay.querySelector('.dialog-header').getBoundingClientRect();
    const footer = overlay.querySelector('.dialog-footer').getBoundingClientRect();
    return { headerY: header.y, footerY: footer.y };
  });
  check(Math.abs(after.headerY - before.header.y) < 1 && Math.abs(after.footerY - before.footer.y) < 1, `${label}: only the body moves on scroll`);
}

async function fieldClearance(page, label, scope = page.locator('.dialog-overlay').last()) {
  const fields = scope.locator('input:not([type="checkbox"]):not([type="radio"]):not([type="hidden"]):enabled, select:enabled, textarea:enabled');
  check(await fields.count() > 0, `${label}: has form fields to inspect`);
  for (const field of await fields.all()) {
    // Inspect the focused field away from the page viewport edge as well as local clipping.
    await field.evaluate((control) => control.blur());
    await field.evaluate((control) => control.scrollIntoView({ block: 'center', inline: 'nearest' }));
    await field.focus();
    const result = await field.evaluate((control) => {
      const rect = control.getBoundingClientRect();
      const style = getComputedStyle(control);
      const halo = Math.max(1, parseFloat(style.outlineWidth) + parseFloat(style.outlineOffset));
      const clippedBy = [];
      for (let parent = control.parentElement; parent; parent = parent.parentElement) {
        const css = getComputedStyle(parent);
        const bounds = parent.getBoundingClientRect();
        const left = bounds.left + parent.clientLeft;
        const top = bounds.top + parent.clientTop;
        if (css.overflowX !== 'visible' && (rect.left - halo < left - 1 || rect.right + halo > left + parent.clientWidth + 1)) clippedBy.push(`${parent.className}: horizontal`);
        if (css.overflowY !== 'visible' && (rect.top - halo < top - 1 || rect.bottom + halo > top + parent.clientHeight + 1)) clippedBy.push(`${parent.className}: vertical (field=${rect.top}..${rect.bottom}, container=${top}..${top + parent.clientHeight})`);
        if (parent.classList.contains('dialog-panel')) break;
      }
      return { field: control.getAttribute('placeholder') || control.id || control.tagName, clippedBy };
    });
    check(result.clippedBy.length === 0, `${label}: field border and focus outline stay visible ${JSON.stringify(result)}`);
  }
  await fields.first().focus();
}

(async () => {
  const browser = await chromium.launch({ headless: true, executablePath: process.env.NUSA_BROWSER_PATH || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe' });
  try {
    for (const [name, width, height, theme] of [
      ['desktop-light', 1440, 900, 'light'], ['desktop-dark', 1280, 720, 'dark'],
      ['mobile', 390, 700, 'light'], ['narrow-short', 320, 400, 'light'],
    ]) {
      const page = await browser.newPage({ viewport: { width, height } });
      await fixture(page, theme);
      await page.goto(`${base}/tenants`, { waitUntil: 'domcontentloaded' });
      const opener = page.getByRole('button', { name: 'Tambah Tenant', exact: true });
      await opener.click();
      await page.getByRole('heading', { name: 'Tambah Tenant Baru' }).waitFor();
      await page.waitForTimeout(240);
      await fieldClearance(page, `${name} tenant identity`);
      await page.getByPlaceholder('contoh: tokokami atau klien-a').fill('demo');
      await page.getByPlaceholder('Minimal 8 karakter kuat').fill('FixturePass123!');
      await page.getByRole('button', { name: 'Lanjut Pilih Paket' }).click();
      await page.getByText('Paket uji 24', { exact: true }).waitFor({ state: 'attached' });
      await page.waitForTimeout(240);
      await geometry(page, `${name} tenant plans`, true);
      await page.screenshot({ path: path.join(output, `tenant-${name}.png`) });
      if (name === 'mobile') {
        await page.setViewportSize({ width, height: 360 });
        await page.waitForTimeout(100);
        await geometry(page, 'Mobile after viewport shrink', true);
        await page.screenshot({ path: path.join(output, 'tenant-short-viewport.png') });
        await page.setViewportSize({ width, height });
        await page.waitForTimeout(100);
      }
      const motion = await page.locator('.dialog-panel').evaluate((panel) => ({ name: getComputedStyle(panel).animationName, duration: getComputedStyle(panel).animationDuration }));
      check(motion.name === 'dialog-panel-in' && motion.duration === '0.2s', `${name}: dialog animation is defined`);

      const controls = page.locator('.dialog-overlay button:not([disabled]), .dialog-overlay input:not([disabled])');
      await controls.last().focus(); await page.keyboard.press('Tab');
      check(await controls.first().evaluate((element) => element === document.activeElement), `${name}: Tab stays inside dialog`);
      await page.keyboard.press('Shift+Tab');
      check(await controls.last().evaluate((element) => element === document.activeElement), `${name}: Shift+Tab stays inside dialog`);
      await page.keyboard.press('Escape');
      await page.locator('.dialog-overlay').waitFor({ state: 'detached' });
      check(await opener.evaluate((element) => element === document.activeElement), `${name}: focus returns to opener`);
      check(await page.locator('#root').evaluate((root) => !root.inert), `${name}: background interaction restored`);

      await page.goto(`${base}/plans`, { waitUntil: 'domcontentloaded' });
      await page.getByRole('button', { name: 'Buat Paket Baru', exact: true }).click();
      await page.waitForTimeout(240);
      await geometry(page, `${name} plan form`, height <= 500);
      await fieldClearance(page, `${name} plan form`);
      await page.screenshot({ path: path.join(output, `plan-${name}.png`) });
      check(await page.locator('.dialog-form .dialog-footer button[type="submit"]').count() === 1, `${name}: form footer keeps native submission`);
      await page.keyboard.press('Escape');
      await page.locator('.dialog-overlay').waitFor({ state: 'detached' });

      await page.keyboard.press('Control+k');
      await page.getByRole('combobox').waitFor();
      await page.keyboard.press('Escape');
      await page.locator('.dialog-overlay').waitFor({ state: 'detached' });
      check(true, `${name}: command palette still opens and closes`);

      await page.goto(`${base}/sites`, { waitUntil: 'domcontentloaded' });
      await page.getByRole('button', { name: 'Situs baru', exact: true }).click();
      await page.waitForTimeout(240);
      await geometry(page, `${name} site wizard`);
      await page.getByRole('button', { name: 'Lanjutkan', exact: true }).click();
      await page.waitForTimeout(240);
      await fieldClearance(page, `${name} site domain`);
      await page.keyboard.press('Escape');
      await page.locator('.dialog-overlay').waitFor({ state: 'detached' });

      await page.goto(`${base}/runtime?tab=python`, { waitUntil: 'domcontentloaded' });
      await page.getByText('Pasang versi Python lainnya', { exact: true }).click();
      await page.getByLabel('Nomor versi lengkap', { exact: true }).waitFor();
      await fieldClearance(page, `${name} Python inline manual version`, page.locator('details[open]'));
      check(await page.getByRole('dialog').count() === 0, `${name}: Python installation stays inline`);
      await page.screenshot({ path: path.join(output, `python-${name}.png`) });
      await page.goto(`${base}/runtime?tab=node`, { waitUntil: 'domcontentloaded' });
      await page.getByText('Pengaturan pemasangan Node.js', { exact: true }).click();
      await page.getByLabel('Mirror unduhan (opsional)', { exact: true }).waitFor();
      await fieldClearance(page, `${name} Node inline options`, page.locator('details[open]'));
      check(await page.getByRole('dialog').count() === 0, `${name}: Node installation stays inline`);
      await page.screenshot({ path: path.join(output, `node-${name}.png`) });
      await page.close();
    }
    const reduced = await browser.newPage({ viewport: { width: 390, height: 600 }, reducedMotion: 'reduce' });
    await fixture(reduced, 'light');
    await reduced.goto(`${base}/tenants`, { waitUntil: 'domcontentloaded' });
    await reduced.getByRole('button', { name: 'Tambah Tenant', exact: true }).click();
    const animations = await reduced.locator('.dialog-panel').evaluate((panel) => ({ panel: getComputedStyle(panel).animationName, backdrop: getComputedStyle(panel.parentElement).animationName }));
    check(animations.panel === 'none' && animations.backdrop === 'none', 'Reduced motion disables dialog and backdrop animations');
    await reduced.getByPlaceholder('contoh: tokokami atau klien-a').fill('demo');
    await reduced.getByPlaceholder('Minimal 8 karakter kuat').fill('FixturePass123!');
    await reduced.getByRole('button', { name: 'Lanjut Pilih Paket' }).click();
    await reduced.getByRole('button', { name: 'Tinjau Ringkasan' }).click();
    await reduced.getByRole('button', { name: 'Konfirmasi & Buat Tenant' }).click();
    await reduced.getByText(/Pesan error fixture/).waitFor();
    await geometry(reduced, 'Long server error', true);
    check(submissions.length === 1 && submissions[0].username_slug === 'demo' && submissions[0].plan_id === 1, 'Wizard keeps the existing tenant request contract');
    await reduced.screenshot({ path: path.join(output, 'tenant-error-reduced-motion.png') });
    await reduced.close();
    check(errors.length === 0, `No browser errors: ${errors.join('; ')}`);
    fs.writeFileSync(path.join(output, 'results.json'), JSON.stringify({ assertions, errors, note: 'Mock API only. No live provisioning or agent execution.' }, null, 2));
    console.log(`${assertions} browser assertions passed. Screenshots: ${output}`);
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
