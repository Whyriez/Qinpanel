/* Static UI + API fixtures only. No panel daemons, system changes or real file operations. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
const output = path.join(__dirname, '../artifacts/file-marquee');
let checks = 0;
function check(value, message) { assert.ok(value, message); checks++; console.log(`ok ${checks}: ${message}`); }
const near = (a, b) => Math.abs(a - b) < 2;
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || 'msedge' });
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    const errors = [], mutations = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url());
      if (!['GET', 'HEAD'].includes(req.method())) mutations.push({ path: url.pathname, method: req.method(), body: req.postData() });
      let status = 200, body = {};
      if (url.pathname === '/api/v1/auth/me') body = { user: { id: 42, username: 'customer', role: 'user' }, csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/files/list') body = { entries: Array.from({ length: 80 }, (_, i) => ({ name: `file-${String(i).padStart(2, '0')}.txt`, relative_path: `file-${String(i).padStart(2, '0')}.txt`, is_dir: false, size_bytes: 1024, modified_at: 1700000000, mode: 420, owner_user: 'p_customer', owner_group: 'panel-tenants' })), total_count: 80, has_more: false };
      else if (url.pathname.includes('notifications')) body = { notifications: [], count: 0, unread_count: 0, total: 0 };
      else if (url.pathname === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 1 };
      else { status = 404; body = { error: 'No fixture' }; }
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const container = page.locator('div.select-none.overflow-auto');
    const marquee = container.locator('div.absolute.pointer-events-none.border-blue-500');
    const items = container.locator('[data-file-item]');
    const settle = () => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    async function boxMatches(start, end, message) {
      const box = await marquee.boundingBox();
      check(box && near(box.x, Math.min(start.x, end.x)) && near(box.y, Math.min(start.y, end.y)) && near(box.width, Math.abs(end.x - start.x)) && near(box.height, Math.abs(end.y - start.y)), message);
    }
    async function selectionMatches(start, end, message) {
      const result = await items.evaluateAll((elements, { start, end }) => elements.map(el => {
        const r = el.getBoundingClientRect();
        const expected = Math.min(start.x, end.x) < r.right && Math.max(start.x, end.x) > r.left && Math.min(start.y, end.y) < r.bottom && Math.max(start.y, end.y) > r.top;
        return { expected, checked: el.querySelector('input[type=checkbox]').checked };
      }), { start, end });
      check(result.some(r => r.expected) && result.every(r => r.expected === r.checked), message);
    }
    await page.goto(base + '/files', { waitUntil: 'domcontentloaded' });
    await items.first().waitFor();
    await settle();
    let area = await container.boundingBox();
    let start = { x: area.x + 5, y: area.y + 65 }, end = { x: area.x + 220, y: area.y + 175 };
    await page.mouse.move(start.x, start.y); await page.mouse.down();
    await page.mouse.move(end.x, end.y, { steps: 8 });
    try { await boxMatches(start, end, 'selection box starts at the held cursor position and ends at the current cursor'); }
    catch (error) {
      await page.screenshot({ path: path.join(output, 'before.png'), fullPage: true });
      await fs.writeFile(path.join(output, 'before.json'), JSON.stringify({ start, end, container: area, box: await marquee.boundingBox() }, null, 2));
      throw error;
    }
    await selectionMatches(start, end, 'highlighted list files match the visible selection rectangle');
    await page.mouse.up();
    check(await marquee.count() === 0, 'mouse release removes the selection box');

    // Start in reverse direction, then scroll while the held mouse is stationary.
    await container.evaluate(el => { el.scrollTop = 240; }); await settle();
    area = await container.boundingBox();
    start = { x: area.x + 5, y: area.y + 210 }; end = { x: area.x + 240, y: area.y + 90 };
    await page.mouse.move(start.x, start.y); await page.mouse.down();
    await page.mouse.move(end.x, end.y, { steps: 6 });
    await boxMatches(start, end, 'reverse drag after scrolling keeps the anchor under the original cursor');
    await selectionMatches(start, end, 'reverse scrolled selection highlights the intersecting files');
    await container.evaluate(el => { el.scrollTop += 60; }); await settle();
    start.y -= 60;
    await boxMatches(start, end, 'scroll during drag keeps the content anchor fixed and the moving edge under the cursor');
    await selectionMatches(start, end, 'scroll during drag recalculates selected files without another mouse move');
    await page.mouse.move(20, 110, { steps: 5 }); await page.mouse.up();
    await page.mouse.move(end.x, end.y);
    check(await marquee.count() === 0, 'release outside the file panel stops dragging when the cursor returns');

    // Grid uses the same coordinates and hit testing as the list.
    await page.getByTitle('Tampilan Grid', { exact: true }).click();
    await container.evaluate(el => { el.scrollTop = 0; }); await settle();
    area = await container.boundingBox();
    start = { x: area.x + 5, y: area.y + 20 }; end = { x: area.x + 260, y: area.y + 160 };
    await page.mouse.move(start.x, start.y); await page.mouse.down(); await page.mouse.move(end.x, end.y, { steps: 6 });
    await boxMatches(start, end, 'grid drag uses the held cursor as its anchor');
    await selectionMatches(start, end, 'grid selection matches the rectangle');
    await page.screenshot({ path: path.join(output, 'grid-drag.png'), fullPage: true });
    await page.evaluate(() => window.dispatchEvent(new Event('blur'))); await settle();
    check(await marquee.count() === 0, 'window losing focus cancels the active drag');
    await page.mouse.up();

    // Native file-item dragging must still carry the existing file payload.
    const payload = await items.first().evaluate(el => {
      const transfer = new DataTransfer();
      el.dispatchEvent(new DragEvent('dragstart', { bubbles: true, dataTransfer: transfer }));
      return transfer.getData('application/panel-files');
    });
    check(JSON.parse(payload).includes('file-00.txt'), 'native file drag keeps the file transfer payload');
    check(await marquee.count() === 0, 'native file drag does not open a marquee');
    await page.mouse.click(area.x + 5, area.y + 20, { button: 'right' });
    check(await marquee.count() === 0, 'right click does not start marquee selection');
    await page.keyboard.press('Escape');
    check(mutations.length === 0, 'cursor selection never sends a file mutation or upload request');
    check(errors.length === 0, errors.join('\n') || 'no browser errors');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
