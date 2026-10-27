/* Browser + API/WebSocket fixtures only. No shell commands or panel daemons. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
const output = path.join(__dirname, '../artifacts/file-terminal-floating');
let checks = 0;
const check = (value, message) => { assert.ok(value, message); checks++; console.log(`ok ${checks}: ${message}`); };
const near = (a, b) => Math.abs(a - b) < 2;
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || 'msedge' });
  try {
    const page = await browser.newPage({ viewport: { width: 1366, height: 900 } });
    const errors = [], calls = [], sockets = [], messages = [];
    page.on('pageerror', error => errors.push(error.message));
    const mode = { policy: 'full' };
    let sessionCount = 0;
    await page.routeWebSocket('**/ws/terminal/**', socket => {
      sockets.push(socket);
      socket.onMessage(message => {
        const text = typeof message === 'string' ? message : message.toString('utf8');
        messages.push(text);
      });
      socket.send('\r\nFLOATING_SESSION_READY\r\nfixture $ ');
    });
    await page.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url());
      calls.push({ path: url.pathname, method: req.method(), body: req.postDataJSON() });
      let status = 200, body = {};
      if (url.pathname === '/api/v1/auth/me') body = { user: { id: 42, username: 'customer', role: 'user' }, csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/tenants/42/shell-policy') body = { user_id: 42, username: 'customer', effective_policy: mode.policy, shell_policy: mode.policy, inherited_from_plan: false, plan_policy: mode.policy };
      else if (url.pathname === '/api/v1/files/list') {
        const current = url.searchParams.get('path') || '';
        body = { entries: current ? [] : [{ name: 'sites', relative_path: 'sites', path: 'sites', is_dir: true, size_bytes: 4096, modified_at: 1, mode: 493, owner_user: 'p_customer', owner_group: 'panel-tenants' }], total_count: current ? 0 : 1, has_more: false };
      } else if (url.pathname === '/api/v1/terminal/sessions' && req.method() === 'POST') {
        sessionCount++;
        body = { session_id: `floating-${sessionCount}`, username: 'p_customer', pid: 1, cwd: req.postDataJSON().cwd || '', cols: 80, rows: 24, created_at: 1, ws_url: `/ws/terminal/floating-${sessionCount}`, closed: false };
      } else if (url.pathname.startsWith('/api/v1/terminal/sessions/') && req.method() === 'DELETE') body = { session_id: url.pathname.split('/').at(-1), closed: true, message: 'Closed' };
      else if (url.pathname.includes('notifications')) body = { notifications: [], count: 0, unread_count: 0, total: 0 };
      else if (url.pathname === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 1 };
      else { status = 404; body = { error: 'No fixture' }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const created = () => calls.filter(call => call.path === '/api/v1/terminal/sessions' && call.method === 'POST');
    const closed = () => calls.filter(call => call.path.startsWith('/api/v1/terminal/sessions/') && call.method === 'DELETE');
    const frame = page.locator('[data-files-terminal]');
    const state = () => frame.getAttribute('data-window-state');
    const waitState = async value => { await page.waitForFunction(value => document.querySelector('[data-files-terminal]')?.getAttribute('data-window-state') === value, value); };
    await page.goto(base + '/files', { waitUntil: 'domcontentloaded' });
    await page.getByRole('row').filter({ hasText: 'sites' }).dblclick();
    await page.getByText('Direktori ini masih kosong', { exact: true }).waitFor();
    await page.getByRole('button', { name: 'Terminal di sini', exact: true }).click();
    await frame.locator('.xterm-screen').waitFor();
    await page.waitForFunction(() => document.querySelector('.xterm-rows')?.textContent.includes('FLOATING_SESSION_READY'));
    check(await state() === 'floating', 'file terminal opens as a floating window');
    check(await frame.getAttribute('aria-modal') === 'false', 'floating terminal does not block the file manager');
    const initial = await frame.boundingBox();
    check(initial.width <= 840 && initial.height <= 440 && initial.x > 12 && initial.y > 12, 'initial terminal uses a moderate size away from screen edges');
    check(created().length === 1 && created()[0].body.cwd === 'sites', 'terminal preserves the selected tenant-relative folder');
    check(calls.findIndex(call => call.path.endsWith('/shell-policy')) < calls.findIndex(call => call.method === 'POST'), 'shell policy is checked before session creation');

    await page.getByRole('button', { name: 'Folder Baru', exact: true }).first().click();
    await page.locator('input[placeholder="nama-folder"]').waitFor();
    check(true, 'file manager remains interactive while terminal is open');
    await page.getByRole('button', { name: 'Batal', exact: true }).click();
    const handle = frame.getByRole('button', { name: 'Geser terminal', exact: true });
    const handleBox = await handle.boundingBox();
    await page.mouse.move(handleBox.x + 60, handleBox.y + 15);
    await page.mouse.down();
    await page.mouse.move(handleBox.x - 120, handleBox.y - 125, { steps: 8 });
    await page.mouse.up();
    const dragged = await frame.boundingBox();
    check(dragged.x < initial.x - 100 && dragged.y < initial.y - 100, 'title bar drag moves the floating window');
    await handle.focus();
    await page.keyboard.press('ArrowLeft');
    const keyed = await frame.boundingBox();
    check(near(keyed.x, dragged.x - 10), 'keyboard arrows can move the window');
    await frame.getByRole('button', { name: 'Atur posisi terminal', exact: true }).click();
    await frame.getByRole('button', { name: 'Geser ke kanan', exact: true }).click();
    const clicked = await frame.boundingBox();
    check(near(clicked.x, keyed.x + 40), 'single-click positioning is available as an alternative to drag');
    await page.keyboard.press('Escape');

    await frame.getByRole('button', { name: 'Minimalkan terminal', exact: true }).click();
    await waitState('minimized');
    check((await frame.boundingBox()).height <= 44, 'minimize collapses terminal to a small bar');
    check(await page.evaluate(() => document.activeElement.getAttribute('aria-label') === 'Pulihkan terminal'), 'minimize keeps keyboard focus on the restore control');
    check(created().length === 1 && closed().length === 0 && sockets.length === 1, 'minimize keeps the same session and WebSocket');
    sockets[0].send('\r\nOUTPUT_WHILE_MINIMIZED\r\n');
    await frame.getByRole('button', { name: 'Pulihkan terminal', exact: true }).click();
    await waitState('floating');
    await page.waitForFunction(() => document.querySelector('.xterm-rows')?.textContent.includes('OUTPUT_WHILE_MINIMIZED'));
    check(true, 'restoring minimized terminal preserves streamed output');
    await frame.getByRole('button', { name: 'Minimalkan terminal', exact: true }).click();
    await page.getByRole('button', { name: 'Terminal di sini', exact: true }).click();
    await waitState('floating');
    check(created().length === 1, 'file manager terminal button restores the existing minimized session');

    const beforeMax = await frame.boundingBox();
    await frame.getByRole('button', { name: 'Maksimalkan terminal', exact: true }).click();
    await waitState('maximized');
    const max = await frame.boundingBox();
    check(near(max.x, 12) && near(max.y, 12) && near(max.width, 1342) && near(max.height, 876), 'maximize fills viewport while preserving visible window controls');
    await frame.getByRole('button', { name: 'Pulihkan ukuran terminal', exact: true }).click();
    await waitState('floating');
    const restored = await frame.boundingBox();
    check(near(restored.x, beforeMax.x) && near(restored.y, beforeMax.y) && near(restored.width, beforeMax.width), 'restore returns to the previous floating position and size');
    await frame.getByRole('button', { name: 'Maksimalkan terminal', exact: true }).click();
    await frame.getByRole('button', { name: 'Minimalkan terminal', exact: true }).click();
    await frame.getByRole('button', { name: 'Maksimalkan terminal', exact: true }).click();
    await waitState('maximized');
    check(true, 'maximize from a minimized maximized window restores maximized state');
    await frame.getByRole('button', { name: 'Pulihkan ukuran terminal', exact: true }).click();
    await frame.locator('.xterm-helper-textarea').focus();
    await page.keyboard.type('floating-input');
    await page.waitForTimeout(100);
    check(messages.join('').includes('floating-input'), 'terminal keyboard input still reaches the same WebSocket fixture');
    await page.screenshot({ path: path.join(output, 'floating-desktop.png'), fullPage: true });

    await frame.getByRole('button', { name: 'Tab Baru', exact: true }).click();
    await page.waitForFunction(() => document.querySelectorAll('[data-files-terminal] .xterm-screen').length === 2);
    check(created().length === 2 && sockets.length === 2, 'floating terminal supports multiple session tabs');
    await page.setViewportSize({ width: 390, height: 844 });
    const mobile = await frame.boundingBox();
    check(mobile.x >= 12 && mobile.y >= 12 && mobile.x + mobile.width <= 378 && mobile.y + mobile.height <= 832, 'resize clamps the window inside a mobile viewport');
    await frame.getByRole('button', { name: 'Maksimalkan terminal', exact: true }).click();
    await waitState('maximized');
    check((await frame.boundingBox()).width <= 366, 'mobile maximized window fits the viewport');
    await page.screenshot({ path: path.join(output, 'floating-mobile.png'), fullPage: true });
    const closeResponses = Promise.all(['floating-1', 'floating-2'].map(id => page.waitForResponse(res => res.request().method() === 'DELETE' && res.url().endsWith(`/${id}`))));
    await frame.getByRole('button', { name: 'Tutup terminal berkas', exact: true }).click();
    await frame.waitFor({ state: 'detached' });
    await closeResponses;
    check(closed().length === 2, 'closing the window closes all terminal sessions');
    check(await page.getByRole('button', { name: 'Terminal di sini', exact: true }).evaluate(el => el === document.activeElement), 'close returns keyboard focus to the file manager trigger');
    await page.goto(base + '/terminal', { waitUntil: 'domcontentloaded' });
    await page.locator('.xterm-screen').waitFor();
    check(await frame.count() === 0 && await page.getByRole('button', { name: 'Minimalkan terminal' }).count() === 0, 'standalone Terminal menu keeps its existing layout');

    mode.policy = 'sftp';
    await page.goto(base + '/files', { waitUntil: 'domcontentloaded' });
    const beforeBlocked = created().length;
    await page.getByRole('button', { name: 'Terminal di sini', exact: true }).click();
    await frame.getByText('Terminal belum diaktifkan', { exact: true }).waitFor();
    check(created().length === beforeBlocked, 'floating window does not bypass SFTP-only shell restrictions');
    check(errors.length === 0, errors.join('\n') || 'no browser render errors');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors, calls, websocketCount: sockets.length }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
