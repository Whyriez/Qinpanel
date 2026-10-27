/* Read-only email API fixtures; no mailbox, delivery, runtime or provider changes. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5198';
const output = path.join(__dirname, '../artifacts/email-hover');
const id = '01234567-89ab-4cde-8fab-0123456789ab';
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
const rgb = value => value.trim().startsWith('#') ? value.trim().slice(1).match(/../g).map(c => parseInt(c, 16)) : value.match(/[\d.]+/g).slice(0, 3).map(Number);
const luminance = color => rgb(color).map(value => { const c = value / 255; return c <= .04045 ? c / 12.92 : ((c + .055) / 1.055) ** 2.4; }).reduce((sum, value, index) => sum + value * [.2126, .7152, .0722][index], 0);
const contrast = style => { const a = luminance(style.color), b = luminance(style.background); return (Math.max(a, b) + .05) / (Math.min(a, b) + .05); };
const sample = locator => locator.evaluate(element => {
  const css = getComputedStyle(element);
  return { background: css.backgroundColor, color: css.color, border: css.borderColor, outline: css.outlineStyle, outlineWidth: parseFloat(css.outlineWidth), primary: css.getPropertyValue('--primary'), hover: css.getPropertyValue('--primary-hover'), sunken: css.getPropertyValue('--surface-sunken') };
});
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge', headless: true });
  const errors = [], writes = [], external = [];
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1050 }, reducedMotion: 'reduce' });
    page.on('pageerror', error => errors.push(error.message));
    page.on('request', request => { if (new URL(request.url()).origin !== new URL(base).origin) external.push(request.url()); });
    await page.route('**/api/v1/**', async route => {
      const request = route.request(), p = new URL(request.url()).pathname;
      if (request.method() !== 'GET') { writes.push(p); await route.fulfill({ status: 405, body: '{}' }); return; }
      let status = 200, body = {};
      if (p === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (p === '/api/v1/email') body = {
        mailboxes: [{ id, domain_id: 1, address: 'admin@example.com', quota_mb: 1024, enabled: true, state: 'active', operation: 'apply', job_id: null, error: null }],
        domains: [{ id: 1, name: 'example.com' }], certificates: [{ id: 1, hostname: 'mail.example.com', san_domains: [] }],
        server: { configured: true, server: { hostname: 'mail.example.com', certificate_id: 1 }, services: ['postfix', 'dovecot', 'opendkim'].map(name => ({ name, running: true })) }, server_error: null,
      };
      else if (p === '/api/v1/email/templates') body = { panel_name: 'Qinpanel', subject: '[{panel_name}] {title}', body: '{message}' };
      else if (p.endsWith('/messages')) body = { items: [], total: 0, unseen: 0, uid_validity: 7, before: null };
      else if (p === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 0, page: 1, per_page: 100 };
      else if (p.includes('notifications')) body = { notifications: [], unread_count: 0, count: 0, total: 0 };
      else { status = 404; body = { error: 'No fixture' }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const root = () => page.getByTestId('email-page');
    const tabs = () => root().getByRole('navigation', { name: 'Bagian email' });
    async function checkPrimary(button, label) {
      await page.mouse.move(0, 0);
      const idle = await sample(button);
      check(contrast(idle) >= 4.5, `${label}: normal text contrast meets AA`);
      await button.hover();
      const hovered = await sample(button);
      await page.screenshot({ path: path.join(output, `${label.replace(/[^a-z0-9-]/gi, '-')}.png`), fullPage: true });
      check(contrast(hovered) >= 4.5, `${label}: hovered text contrast meets AA (${contrast(hovered).toFixed(2)}:1)`);
      check(JSON.stringify(rgb(hovered.background)) === JSON.stringify(rgb(hovered.hover)) && hovered.color === idle.color, `${label}: hover uses the primary hover token and preserves foreground`);
    }
    await page.goto(`${base}/email`);
    for (const theme of ['light', 'dark']) {
      await page.evaluate(value => localStorage.setItem('nusa_theme', value), theme);
      await page.reload();
      await root().getByRole('button', { name: 'Buat akun', exact: true }).waitFor();
      check(await page.locator('html').evaluate(el => el.classList.contains('dark')) === (theme === 'dark'), `${theme}: expected theme is active`);
      for (const name of ['Akun dan kotak masuk', 'Notifikasi panel', 'Server email']) {
        const button = tabs().getByRole('button', { name, exact: true });
        await button.click();
        check(await button.getAttribute('aria-pressed') === 'true', `${theme} ${name}: selection remains explicit`);
        await checkPrimary(button, `${theme}-${name}`);
      }
      await tabs().getByRole('button', { name: 'Akun dan kotak masuk', exact: true }).click();
      const inactive = tabs().getByRole('button', { name: 'Notifikasi panel', exact: true });
      await inactive.hover();
      const neutral = await sample(inactive);
      check(JSON.stringify(rgb(neutral.background)) === JSON.stringify(rgb(neutral.sunken)) && contrast(neutral) >= 4.5, `${theme}: inactive tab hover remains neutral and readable`);
      await root().getByRole('button', { name: 'Buat akun', exact: true }).click();
      const create = root().getByRole('button', { name: 'Buat akun email', exact: true });
      check(await create.isDisabled(), `${theme}: create stays disabled without a domain`);
      const disabled = await sample(create);
      await create.hover();
      check((await sample(create)).background === disabled.background, `${theme}: disabled primary hover does not change background`);
      await root().getByLabel('Domain', { exact: true }).selectOption('1');
      await checkPrimary(create, `${theme}-create-mailbox`);
      await root().getByRole('button', { name: 'Buat akun', exact: true }).click();
      await root().getByRole('button', { name: /admin@example.com/ }).click();
      const inbox = root().getByRole('region', { name: 'Kotak masuk email', exact: true });
      for (const name of ['Kotak masuk', 'Terkirim', 'Sampah']) {
        const button = inbox.getByRole('button', { name, exact: true });
        await button.click();
        check(await button.getAttribute('aria-pressed') === 'true', `${theme} ${name}: active folder is selected`);
        await checkPrimary(button, `${theme}-${name}`);
      }
      await checkPrimary(inbox.getByRole('button', { name: 'Tulis email', exact: true }), `${theme}-compose`);
      const selected = inbox.getByRole('button', { name: 'Sampah', exact: true });
      await page.keyboard.press('Tab'); await selected.focus();
      const focus = await sample(selected);
      check(focus.outline !== 'none' && focus.outlineWidth >= 2, `${theme}: keyboard focus remains visible`);
    }
    await page.setViewportSize({ width: 320, height: 800 });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'email controls fit a 320px viewport');
    await page.screenshot({ path: path.join(output, 'mobile-dark.png'), fullPage: true });
    check(writes.length === 0 && external.length === 0, 'hover and selection checks make no mutations or external requests');
    check(errors.length === 0, 'no browser runtime errors');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, writes, errors, external }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
