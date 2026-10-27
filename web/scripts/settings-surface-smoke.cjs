/* Browser checks with API fixtures only. No firewall, repair, backup or restore is executed. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
const doctor = { timestamp: 1, healthy: false, overall_status: 'warn', total_passed: 1, total_warnings: 1, total_failures: 0,
  checks: [{ id: 'fixture', name: 'Socket runtime', category: 'socket', status: 'warn', message: 'Pemeriksaan fixture', remediation_hint: 'Fixture saja', fixable: true }] };
const reconcile = { timestamp: 1, total_resources_checked: 2, is_clean: false, drifts: [{ id: 'fixture', drift_type: 'missing_nginx_config', resource_type: 'site', resource_id: '1', description: 'Konfigurasi fixture', desired_state: 'Ada', actual_state: 'Belum ada', remediation_action: 'Terapkan ulang' }] };
const backup = { filename: 'panel-fixture.tar.gz', filepath: '/fixture/panel-fixture.tar.gz', created_at: 1, size_bytes: 4096, checksum_sha256: 'a'.repeat(64), files_count: 2, verified: true };
async function fixture(browser, theme, width) {
  const page = await browser.newPage({ viewport: { width, height: 900 } });
  page.setDefaultTimeout(15000);
  const errors = [], mutations = [];
  await page.addInitScript(value => localStorage.setItem('nusa_theme', value), theme);
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/v1/**', async route => {
    const request = route.request(), name = new URL(request.url()).pathname;
    if (request.method() === 'POST') mutations.push(name);
    let body = {}, status = 200;
    if (name === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
    else if (name === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (name === '/api/v1/system/optimizer') { status = 503; body = { error: 'Fixture optimizer tidak diaktifkan' }; }
    else if (name === '/api/v1/system/security-hardening') body = {
      asvs_audit: { total_controls: 1, passed_controls: 1, failed_controls: 0, compliance_percentage: 100,
        requirements: [{ id: 'V1.1.1', chapter: 'Arsitektur', title: 'Isolasi runtime', description: 'Pemeriksaan fixture', implementation_evidence: 'Konfigurasi fixture', status: 'Pass' }] },
      nftables_status: { enabled: true, table_name: 'panel_loopback_guard', active_rules_count: 2 },
      systemd_security: [{ service_name: 'panel-api.service', exposure_score: 1.2, is_root: false, directives: [] }], secret_key_version: 1,
    };
    else if (name.endsWith('/nftables/sync')) body = { success: true, rules_count: 2, message: 'Sinkronisasi fixture selesai' };
    else if (name === '/api/v1/system/doctor' || name === '/api/v1/system/doctor/fix') body = doctor;
    else if (name === '/api/v1/system/reconcile') body = reconcile;
    else if (name === '/api/v1/system/backups') body = request.method() === 'POST' ? backup : [backup];
    else if (name === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 0, page: 1, per_page: 1 };
    else if (name.includes('unread-count')) body = { count: 0, unread_count: 0 };
    else if (name.includes('notifications')) body = { notifications: [], total: 0 };
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  await page.goto(base + '/settings', { waitUntil: 'domcontentloaded' });
  return { page, errors, mutations };
}
async function surface(locator) {
  return locator.evaluate(element => {
    const css = getComputedStyle(element);
    return { border: css.borderTopColor, width: css.borderTopWidth, radius: css.borderTopLeftRadius, bg: css.backgroundColor };
  });
}
async function hoverCheck(page, button, label, primary = false) {
  await button.waitFor(); await button.scrollIntoViewIfNeeded();
  await page.mouse.move(1, 1); await page.waitForTimeout(160);
  const before = await button.evaluate(element => { const css = getComputedStyle(element); return { bg: css.backgroundColor, color: css.color }; });
  await button.hover(); await page.waitForTimeout(180);
  const after = await button.evaluate((element, primary) => {
    const css = getComputedStyle(element), probe = document.createElement('span');
    probe.style.color = `var(${primary ? '--primary-hover' : element.classList.contains('button-secondary') ? '--raised' : '--surface-sunken'})`;
    document.body.append(probe); const expected = getComputedStyle(probe).color; probe.remove();
    const luminance = color => {
      const channels = color.match(/[\d.]+/g).slice(0, 3).map(Number).map(value => value / 255).map(value => value <= .04045 ? value / 12.92 : ((value + .055) / 1.055) ** 2.4);
      return .2126 * channels[0] + .7152 * channels[1] + .0722 * channels[2];
    };
    const text = luminance(css.color), bg = luminance(css.backgroundColor);
    return { bg: css.backgroundColor, color: css.color, expected, contrast: (Math.max(text, bg) + .05) / (Math.min(text, bg) + .05) };
  }, primary);
  check(after.bg !== before.bg && after.bg === after.expected, `${label}: hover uses the intended theme background (${before.bg} -> ${after.bg}, expected ${after.expected})`);
  check(after.contrast >= 4.5 && after.color === before.color, `${label}: hover text remains readable (${after.contrast.toFixed(2)}:1)`);
}
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  try {
    await fs.mkdir(path.join(__dirname, '../artifacts/settings-surface'), { recursive: true });
    for (const theme of ['light', 'dark']) {
      const f = await fixture(browser, theme, 1280), p = f.page;
      const baseline = await surface(p.getByTestId('server-optimizer-card'));
      await p.getByRole('button', { name: /^Hardening & Firewall/ }).click();
      const hardening = p.getByTestId('security-hardening-card');
      await hardening.getByPlaceholder('Cari kontrol ASVS berdasarkan ID, judul, atau deskripsi...').waitFor();
      check(JSON.stringify(await surface(hardening)) === JSON.stringify(baseline), `${theme}: hardening outer border, radius and surface match other settings cards`);
      await hoverCheck(p, hardening.getByRole('button', { name: 'Muat Ulang', exact: true }), `${theme} hardening refresh`);
      await hardening.getByRole('button', { name: 'Sinkronkan nftables', exact: true }).click();
      await hardening.getByText('Sinkronisasi fixture selesai', { exact: true }).waitFor();
      check(f.mutations.includes('/api/v1/system/security-hardening/nftables/sync'), `${theme}: sync action still uses the existing typed endpoint`);
      await hardening.getByRole('button', { name: /Isolasi Loopback/ }).click();
      await hoverCheck(p, hardening.getByRole('button', { name: 'Terapkan & Sinkronkan Ulang Aturan nftables', exact: true }), `${theme} hardening primary`, true);
      await p.screenshot({ path: path.join(__dirname, `../artifacts/settings-surface/hardening-${theme}.png`) });
      await p.getByTestId('tab-reliability').click();
      const reliability = p.getByTestId('system-reliability-card');
      await reliability.getByTestId('doctor-fix-button').waitFor();
      check(JSON.stringify(await surface(reliability)) === JSON.stringify(baseline), `${theme}: reliability outer border, radius and surface match other settings cards`);
      await hoverCheck(p, reliability.getByRole('button', { name: 'Segarkan', exact: true }), `${theme} reliability refresh`);
      await hoverCheck(p, reliability.getByTestId('doctor-fix-button'), `${theme} doctor repair`, true);
      await reliability.getByTestId('doctor-fix-button').click();
      await reliability.getByText('Perbaikan sistem otomatis berhasil dijalankan.', { exact: true }).waitFor();
      await reliability.getByRole('button', { name: 'Rekonsiliasi Drift', exact: true }).click();
      await hoverCheck(p, reliability.getByRole('button', { name: 'Deteksi Deviasi (Dry Run)', exact: true }), `${theme} reconciliation preview`);
      await hoverCheck(p, reliability.getByTestId('reconcile-apply-button'), `${theme} reconciliation apply`, true);
      await reliability.getByRole('button', { name: 'Cadangan Panel', exact: true }).click();
      await hoverCheck(p, reliability.getByTestId('create-backup-button'), `${theme} backup create`, true);
      await hoverCheck(p, reliability.getByRole('button', { name: 'Pulihkan', exact: true }), `${theme} backup restore`);
      await reliability.getByTestId('create-backup-button').click();
      await reliability.getByText('Cadangan sistem berhasil dibuat: panel-fixture.tar.gz', { exact: true }).waitFor();
      check(!f.mutations.includes('/api/v1/system/backups/restore'), `${theme}: hovering Restore never submits a restore request`);
      await p.screenshot({ path: path.join(__dirname, `../artifacts/settings-surface/reliability-${theme}.png`) });
      await p.setViewportSize({ width: 390, height: 844 });
      await p.getByLabel('Bagian pengaturan', { exact: true }).selectOption('reliability');
      check(!await p.evaluate(() => document.documentElement.scrollWidth > innerWidth), `${theme}: reliability fits mobile viewport`);
      await p.getByLabel('Bagian pengaturan', { exact: true }).selectOption('security_hardening');
      await hardening.getByPlaceholder('Cari kontrol ASVS berdasarkan ID, judul, atau deskripsi...').waitFor();
      check(!await p.evaluate(() => document.documentElement.scrollWidth > innerWidth), `${theme}: hardening fits mobile viewport`);
      await p.screenshot({ path: path.join(__dirname, `../artifacts/settings-surface/mobile-${theme}.png`) });
      check(f.errors.length === 0, `${theme}: no browser exceptions (${f.errors.join(', ')})`);
      await p.close();
    }
    console.log(`${checks} settings surface checks passed (API fixtures only).`);
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
