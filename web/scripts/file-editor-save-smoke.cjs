/* Real FileManager/CodeMirror UI with intercepted API fixtures; no server files or daemons. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5206';
const output = path.join(__dirname, '../artifacts/file-editor-save');
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge' });
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
    page.setDefaultTimeout(10000);
    const errors = [], writes = [], reads = [], unexpectedWrites = [];
    let content = 'Original file', mtime = 1700000000, etag = '"initial"';
    let mode = 'normal', releaseSave, pendingSave, readOnly = false, binary = false;
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url());
      let status = 200, body = {};
      if (url.pathname === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (url.pathname === '/api/v1/tenants') body = { tenants: [{ id: 42, username: 'customer', role: 'user', status: 'active', linux_username: 'p_customer', linux_uid: 200042 }] };
      else if (url.pathname === '/api/v1/files/list') body = { entries: [{ name: 'example.txt', relative_path: 'example.txt', is_dir: false, size_bytes: Buffer.byteLength(content), modified_at: mtime, mode: 420 }], total_count: 1, has_more: false };
      else if (url.pathname === '/api/v1/files/content' && req.method() === 'GET') {
        reads.push({ path: url.searchParams.get('path'), tenant: url.searchParams.get('tenant') });
        body = { path: 'example.txt', content, size_bytes: Buffer.byteLength(content), modified_at: mtime, etag, read_only: readOnly, is_binary: binary };
      } else if (url.pathname === '/api/v1/files/content' && req.method() === 'PUT') {
        const payload = req.postDataJSON();
        writes.push(payload);
        if (mode === 'delayed') { pendingSave = true; await new Promise(resolve => { releaseSave = resolve; }); pendingSave = false; }
        if (mode === 'error') { status = 503; body = { error: 'Fixture network failure' }; }
        else if (!payload.force && (payload.expected_etag !== etag || payload.expected_mtime !== mtime)) {
          status = 409; body = { conflict: true, error: 'Konflik penyimpanan: Berkas telah dimodifikasi oleh proses lain di server sejak Anda membukanya.' };
        } else {
          content = payload.content; mtime += 1; etag = `"revision-${mtime}"`;
          body = mode === 'missing-revision' ? { message: 'saved' } : { path: payload.path, size_bytes: Buffer.byteLength(content), modified_at: mtime, etag, message: 'saved' };
        }
      } else if (url.pathname.includes('notifications')) body = { notifications: [], count: 0, unread_count: 0, total: 0 };
      else if (url.pathname === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 0 };
      else { status = 404; body = { error: 'No fixture' }; }
      if (!['GET', 'HEAD'].includes(req.method()) && url.pathname !== '/api/v1/files/content') unexpectedWrites.push(url.pathname);
      await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const modal = page.getByRole('dialog');
    const editor = modal.locator('.cm-content');
    const save = modal.getByRole('button', { name: /^Simpan(?: Ctrl\+S)?$/ });
    const dirty = modal.getByText('Belum disimpan', { exact: true });
    const success = modal.getByText('Berkas berhasil disimpan', { exact: true });
    async function open() {
      await page.locator('[data-file-item]').filter({ hasText: 'example.txt' }).dblclick();
      await editor.waitFor();
    }
    async function type(text) {
      await editor.click();
      await page.keyboard.press('Control+a');
      await page.keyboard.insertText(text);
      await dirty.waitFor();
    }
    async function close(accept = true) {
      if (await dirty.count()) page.once('dialog', dialog => accept ? dialog.accept() : dialog.dismiss());
      await modal.getByRole('button', { name: 'Tutup Editor' }).click();
    }
    await page.goto(`${base}/files?tenant=42`, { waitUntil: 'domcontentloaded' });
    await open();
    await type('First save');
    await save.click(); await success.waitFor();
    check(writes.length === 1 && writes[0].expected_etag === '"initial"', 'first save uses the opened revision');
    const firstRevision = { mtime, etag };
    check(await save.isDisabled() && await dirty.count() === 0, 'successful save marks the submitted draft clean');
    await type('Second save with different length');
    await page.keyboard.press('Control+s'); await success.waitFor();
    check(writes.length === 2 && writes[1].expected_etag === firstRevision.etag && writes[1].expected_mtime === firstRevision.mtime, 'second save uses the revision returned by the first save');
    check(content === 'Second save with different length' && await modal.getByTestId('conflict-banner').count() === 0, 'repeated save succeeds without a false conflict');
    await type('Third save — unicode'); await save.click(); await success.waitFor();
    check(content === 'Third save — unicode' && writes.length === 3, 'third edit/save also succeeds');
    const countClean = writes.length;
    await page.keyboard.press('Control+s');
    check(writes.length === countClean, 'Ctrl+S on an unchanged file does not submit another write');

    mode = 'delayed';
    await type('Submitted snapshot');
    await page.keyboard.press('Control+s');
    await page.waitForFunction(() => document.querySelector('[role="dialog"]')?.textContent.includes('Menyimpan...'));
    check(pendingSave, 'fixture holds a real in-flight save');
    await page.keyboard.press('Control+s');
    await type('Newer unsaved draft');
    check(writes.length === 4, 'repeated shortcut while saving starts only one request');
    releaseSave(); await success.waitFor();
    check(content === 'Submitted snapshot' && (await editor.innerText()) === 'Newer unsaved draft' && await dirty.count() === 1 && await save.isEnabled(), 'typing during save remains visible and dirty after acknowledgement');
    mode = 'normal';
    await save.click(); await success.waitFor();
    check(content === 'Newer unsaved draft' && writes.length === 5, 'remaining draft saves with the acknowledged revision');

    await type('Keep local draft');
    content = 'External modification'; mtime += 1; etag = '"external"';
    await save.click(); await modal.getByTestId('conflict-banner').waitFor();
    check(content === 'External modification' && await editor.innerText() === 'Keep local draft' && await dirty.count() === 1, 'a genuine external edit still returns 409 and preserves the local draft');
    check(writes.slice(0, 6).every(write => write.force === false), 'ordinary saves never bypass the conflict guard');
    await modal.getByRole('button', { name: 'Simpan Paksa (Overwrite)' }).click(); await success.waitFor();
    check(writes[6].force === true && content === 'Keep local draft', 'overwrite requires the explicit conflict action');
    await type('After explicit overwrite'); await save.click(); await success.waitFor();
    check(writes[7].force === false && content === 'After explicit overwrite', 'ordinary save after overwrite uses its new revision without force');

    mode = 'error'; await type('Retained on error'); await save.click();
    await modal.getByText(/Gagal menyimpan: Fixture network failure/).waitFor();
    check(await modal.getByRole('button', { name: 'Simpan Paksa (Overwrite)' }).count() === 0 && await dirty.count() === 1, 'generic failure retains the draft and does not offer forced overwrite');
    mode = 'normal'; await save.click(); await success.waitFor();
    check(content === 'Retained on error', 'retry after a failed request keeps the last acknowledged revision');
    await type('Discard confirmation draft');
    await close(false);
    check(await modal.count() === 1 && await editor.innerText() === 'Discard confirmation draft', 'close button respects cancel on unsaved changes');
    page.once('dialog', dialog => dialog.dismiss());
    await page.keyboard.press('Escape');
    check(await modal.count() === 1, 'Escape uses the same unsaved-change confirmation');
    await close(true); await modal.waitFor({ state: 'detached' });
    await open();
    check(await editor.innerText() === content && await dirty.count() === 0, 'reopening reads the server content and resets the editor baseline');

    mode = 'missing-revision'; await type('Saved with missing metadata'); await save.click();
    await modal.getByText(/versi terbaru tidak diterima/).waitFor();
    await type('Draft after missing metadata');
    check(await save.isDisabled(), 'missing success metadata blocks subsequent writes instead of dropping guards');
    const missingCount = writes.length;
    await page.keyboard.press('Control+s');
    check(writes.length === missingCount, 'keyboard save cannot bypass missing-revision protection');
    await close(); await modal.waitFor({ state: 'detached' });
    mode = 'normal'; readOnly = true; await open();
    check(await modal.getByText('Hanya Baca', { exact: true }).count() === 1 && await editor.getAttribute('contenteditable') === 'false', 'server read-only flag is respected');
    await close(); await modal.waitFor({ state: 'detached' });
    readOnly = false; binary = true; await open();
    check(await modal.getByText('Hanya Baca', { exact: true }).count() === 1 && await editor.getAttribute('contenteditable') === 'false', 'binary response cannot be saved as an empty text file');
    check(reads.every(read => read.tenant === '42' && read.path === 'example.txt') && writes.every(write => write.tenant === '42' && write.path === 'example.txt'), 'all reads and writes retain the selected tenant and relative path');
    check(unexpectedWrites.length === 0, 'editor shortcuts never trigger background file-manager mutations');
    check(errors.length === 0, 'no browser runtime errors');
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, errors, reads, writes, unexpectedWrites }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
