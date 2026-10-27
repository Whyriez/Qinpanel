/* Browser fixtures only: no panel services or tenant mutations are performed. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5198';
const output = path.join(__dirname, '../artifacts/tenant-mobile');
let checks = 0;
const check = (ok, message) => { assert.ok(ok, message); console.log(`ok ${++checks}: ${message}`); };
const tenant = {
  id: 41, username: 'pelanggan-dengan-nama-panjang', email: 'administrasi.pelanggan.dengan.alamat.panjang@example.co.id',
  status: 'active', linux_username: 'p_pelanggan_dengan_nama_panjang', linux_uid: 10041,
  plan_id: 1, plan_name: 'Paket hosting bisnis dengan kapasitas penyimpanan tambahan', created_at: 1780000000, updated_at: 1780000000,
};
const second = { ...tenant, id: 42, username: 'pelanggan-kedua', linux_username: null, linux_uid: null, email: null, status: 'suspended', plan_id: null, plan_name: null };
const plan = { id: 1, name: tenant.plan_name, description: 'Paket untuk pelanggan bisnis', cpu_quota_percent: 200, memory_limit_bytes: 2147483648, disk_limit_bytes: 10737418240, inode_limit: 100000, tasks_max: 256, max_sites: 5, max_domains: 10, shell_policy: 'full' };

(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge', headless: true });
  const errors = [], writes = [], external = [], queries = [];
  let listState = 'ready', hardened = false;
  let releaseLoading;
  try {
    const page = await browser.newPage({ viewport: { width: 320, height: 800 }, reducedMotion: 'reduce' });
    page.on('pageerror', error => errors.push(error.message));
    page.on('request', request => { if (new URL(request.url()).origin !== new URL(base).origin) external.push(request.url()); });
    await page.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url()), p = url.pathname;
      if (req.method() !== 'GET') { writes.push(p); await route.fulfill({ status: 405, body: '{}' }); return; }
      let status = 200, body = {};
      if (p === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (p === '/api/v1/tenants') {
        queries.push(url.search);
        if (listState === 'loading') await new Promise(resolve => { releaseLoading = resolve; });
        if (listState === 'error') { status = 500; body = { detail: 'Daftar tenant sementara tidak tersedia' }; }
        else body = { tenants: listState === 'empty' ? [] : [tenant, second].filter(t => (!url.searchParams.get('status') || url.searchParams.get('status') === t.status) && (!url.searchParams.get('search') || t.username.includes(url.searchParams.get('search')))) };
      }
      else if (p === '/api/v1/system/hardening') body = { ...Object.fromEntries(['cgroup_v2', 'hidepid', 'tenant_group', 'quota', 'acl'].map(name => [name, { name, ok: hardened, message: 'Fixture diagnostik' }])), overall_ok: hardened };
      else if (p === '/api/v1/tenants/41') body = { tenant, plan };
      else if (p.endsWith('/shell-policy')) body = { shell_policy: null, effective_policy: 'full', inherited_from_plan: true };
      else if (p.endsWith('/resources')) body = { uid: tenant.linux_uid, linux_username: tenant.linux_username, metrics: { exists: true, memory_current_bytes: 134217728, memory_peak_bytes: 268435456, pids_current: 12, pids_peak: 20, cpu_usage_usec: 1234000, io_read_bytes: 1024, io_write_bytes: 2048 }, quota: { bytes_used: 1024000, bytes_limit: 10737418240, inodes_used: 200, backend: 'ext4' } };
      else if (p === '/api/v1/plans') body = { plans: [plan] };
      else if (p === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 0, page: 1, per_page: 100 };
      else if (p.includes('notifications')) body = { notifications: [], unread_count: 0, count: 0, total: 0 };
      else { status = 404; body = { detail: `Missing fixture: ${p}` }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });

    const root = () => page.getByTestId('tenants-page');
    const mobileList = () => root().locator('.tenant-mobile-list');
    async function noOverflow(label, scope = page.locator('html')) {
      check(await scope.evaluate(el => el.scrollWidth <= el.clientWidth + 1), `${label}: no horizontal overflow`);
      if (await page.locator('.tenant-page').count()) {
        check(await page.locator('.tenant-page').evaluate(el => el.scrollWidth <= el.clientWidth + 1), `${label}: tenant content fits its container`);
      }
      if (await page.locator('.dialog-body').count()) {
        check(await page.locator('.dialog-body').evaluate(el => el.scrollWidth <= el.clientWidth + 1), `${label}: dialog body has no horizontal scroll`);
      }
    }
    async function controlsFit(scope, label) {
      const failures = await scope.locator('button:visible, input:not([type="radio"]):not([type="checkbox"]):visible, select:visible').evaluateAll(elements => elements.filter(el => {
        const r = el.getBoundingClientRect();
        return r.height < 44 || r.left < -1 || r.right > innerWidth + 1;
      }).map(el => el.getAttribute('aria-label') || el.textContent || el.title));
      check(failures.length === 0, `${label}: visible controls fit and are at least 44px high (${failures.join(', ')})`);
    }
    async function buttonContrast(button, label) {
      const ratio = await button.evaluate(el => {
        const luminance = color => color.match(/[\d.]+/g).slice(0, 3).map(Number).map(value => {
          const c = value / 255;
          return c <= .04045 ? c / 12.92 : ((c + .055) / 1.055) ** 2.4;
        }).reduce((sum, value, i) => sum + value * [.2126, .7152, .0722][i], 0);
        const css = getComputedStyle(el), a = luminance(css.color), b = luminance(css.backgroundColor);
        return (Math.max(a, b) + .05) / (Math.min(a, b) + .05);
      });
      check(ratio >= 4.5, `${label}: button text contrast meets AA (${ratio.toFixed(2)}:1)`);
    }
    await page.goto(`${base}/tenants`);
    await mobileList().getByText(tenant.email, { exact: true }).waitFor();
    for (const theme of ['light', 'dark']) {
      await page.setViewportSize({ width: 320, height: 800 });
      await page.evaluate(value => localStorage.setItem('nusa_theme', value), theme);
      await page.reload();
      await mobileList().getByText(tenant.email, { exact: true }).waitFor();
      await buttonContrast(root().getByRole('button', { name: 'Terapkan Penguatan Host', exact: true }), `${theme} hardening`);
      for (const width of [320, 360, 390, 768, 1024, 1440]) {
        await page.setViewportSize({ width, height: 900 });
        const label = `${theme} list ${width}px`;
        await noOverflow(label);
        await controlsFit(root(), label);
        const cards = await mobileList().isVisible();
        check(cards === (width < 1440), `${label}: uses ${cards ? 'cards' : 'table'}`);
        const visible = root().locator(cards ? '.tenant-mobile-list' : '.tenant-desktop-list');
        check(await visible.getByText(tenant.email, { exact: true }).isVisible() && await visible.getByText(tenant.plan_name, { exact: true }).isVisible(), `${label}: long email and plan remain visible`);
        if ([320, 390, 1440].includes(width)) {
          await page.screenshot({ path: path.join(output, `${theme}-list-${width}.png`), fullPage: true });
          if (cards) {
            await mobileList().locator('li').first().scrollIntoViewIfNeeded();
            await page.screenshot({ path: path.join(output, `${theme}-cards-${width}.png`), fullPage: true });
            await root().getByRole('heading', { level: 1 }).scrollIntoViewIfNeeded();
          }
        }
      }
    }

    await page.setViewportSize({ width: 320, height: 800 });
    const filters = root().getByRole('group', { name: 'Status tenant' });
    await filters.getByRole('button', { name: 'Ditangguhkan', exact: true }).click();
    await mobileList().getByRole('heading', { name: second.username, exact: true }).waitFor();
    await page.waitForFunction(() => document.querySelector('.tenant-mobile-list ul')?.children.length === 1);
    check(queries.at(-1).includes('status=suspended'), 'status filter preserves the API query');
    check(await filters.getByRole('button', { name: 'Ditangguhkan', exact: true }).getAttribute('aria-pressed') === 'true', 'selected filter is announced');
    check(await mobileList().getByText('Tanpa paket', { exact: true }).isVisible(), 'tenant without a plan has a readable fallback');
    await filters.getByRole('button', { name: 'Semua', exact: true }).click();
    await root().getByRole('textbox', { name: 'Cari tenant berdasarkan slug atau email' }).fill('tidak-ada');
    await mobileList().getByRole('status').waitFor();
    check(await mobileList().getByText('Tidak ada tenant yang cocok dengan kriteria pencarian.').isVisible(), 'empty search state is visible');
    await root().getByRole('textbox').fill('');
    await mobileList().getByText(tenant.email, { exact: true }).waitFor();
    await root().getByRole('textbox').focus();
    await page.keyboard.press('Tab');
    check(await filters.getByRole('button', { name: 'Semua', exact: true }).evaluate(el => el === document.activeElement && getComputedStyle(el).outlineStyle !== 'none'), 'filter supports keyboard focus');
    await page.evaluate(() => { document.documentElement.style.fontSize = '200%'; });
    await noOverflow('320px with 200% text');
    await page.evaluate(() => { document.documentElement.style.fontSize = ''; });

    await mobileList().getByRole('button', { name: `Hapus tenant ${tenant.username}`, exact: true }).click();
    const confirmation = page.getByRole('dialog');
    await confirmation.waitFor();
    await noOverflow('delete confirmation', confirmation);
    await controlsFit(confirmation, 'delete confirmation');
    check(await confirmation.getByRole('button', { name: 'Hapus Tenant Permanen', exact: true }).isDisabled(), 'delete still requires confirmation');
    await page.keyboard.press('Escape');
    await root().getByRole('button', { name: 'Tambah Tenant', exact: true }).click();
    const wizard = page.getByRole('dialog');
    await wizard.getByRole('heading', { name: 'Tambah Tenant Baru' }).waitFor();
    await wizard.getByPlaceholder('contoh: tokokami atau klien-a').fill('pelanggan-dengan-nama-panjang');
    await wizard.getByPlaceholder('kontak@klien.com').fill(tenant.email);
    await wizard.getByPlaceholder('Minimal 8 karakter kuat').fill('Fixture-only-password-123!');
    await noOverflow('wizard identity', wizard);
    await controlsFit(wizard, 'wizard identity');
    await wizard.getByRole('button', { name: 'Lanjut Pilih Paket', exact: true }).click();
    await wizard.getByText(plan.name, { exact: true }).waitFor();
    await noOverflow('wizard plans', wizard);
    await controlsFit(wizard, 'wizard plans');
    await wizard.getByRole('radio').check();
    await wizard.getByRole('button', { name: 'Tinjau Ringkasan', exact: true }).click();
    await noOverflow('wizard review', wizard);
    await controlsFit(wizard, 'wizard review');
    check(await wizard.evaluate(el => el.contains(document.activeElement)), 'wizard retains keyboard focus after changing steps');
    await page.screenshot({ path: path.join(output, 'dark-wizard-320.png'), fullPage: true });
    await page.keyboard.press('Escape');
    await wizard.waitFor({ state: 'hidden' });

    await mobileList().getByRole('button', { name: `Buka detail tenant ${tenant.username}`, exact: true }).click();
    const detail = page.getByTestId('tenant-detail-page');
    await detail.getByRole('heading', { name: tenant.username, exact: true }).waitFor();
    for (const theme of ['light', 'dark']) {
      await page.evaluate(value => document.documentElement.classList.toggle('dark', value === 'dark'), theme);
      await buttonContrast(detail.getByRole('button', { name: 'Impersonasi', exact: true }), `${theme} impersonation`);
      for (const width of [320, 390, 768, 1440]) {
        await page.setViewportSize({ width, height: 900 });
        for (const tab of ['Ringkasan', 'Penggunaan Resource (Live)', 'Area Berbahaya']) {
          const button = detail.getByRole('navigation', { name: 'Bagian detail tenant' }).getByRole('button', { name: tab, exact: true });
          await button.click();
          check(await button.getAttribute('aria-current') === 'page', `${theme} ${width}px ${tab}: selected tab is announced`);
          await noOverflow(`${theme} ${width}px detail ${tab}`);
          await controlsFit(detail, `${theme} ${width}px detail ${tab}`);
          if (width === 320) await page.screenshot({ path: path.join(output, `${theme}-detail-${tab.split(' ')[0]}-320.png`), fullPage: true });
        }
      }
    }
    await page.setViewportSize({ width: 320, height: 800 });
    hardened = true;
    listState = 'loading';
    await page.goto(`${base}/tenants`, { waitUntil: 'domcontentloaded' });
    await mobileList().getByText('Memuat daftar tenant...').waitFor();
    await noOverflow('loading state');
    listState = 'ready';
    releaseLoading();
    await mobileList().getByText(tenant.email, { exact: true }).waitFor();
    check(await root().getByText('Semua Prasyarat Terpenuhi').isVisible(), 'healthy hardening badge is visible');
    await noOverflow('healthy hardening');
    listState = 'error';
    await root().getByRole('button', { name: 'Muat ulang daftar tenant' }).click();
    await root().getByText('Daftar tenant sementara tidak tersedia', { exact: true }).waitFor();
    await noOverflow('load error');
    check(writes.length === 0, `no mutation requests (${writes.join(', ')})`);
    check(external.length === 0, 'all requests stay local');
    check(errors.length === 0, `no browser exceptions (${errors.join(', ')})`);
    console.log(`${checks} tenant responsive checks passed. Screenshots: ${output}`);
  } catch (error) {
    if (errors.length) console.error('Browser errors:', errors);
    const page = browser.contexts()[0]?.pages()[0];
    if (page) await page.screenshot({ path: path.join(output, 'failure.png'), fullPage: true });
    throw error;
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
