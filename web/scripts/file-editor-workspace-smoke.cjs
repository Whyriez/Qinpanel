/* Static frontend + intercepted file APIs only: no daemons, tenant files, or secrets. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5206';
const output = path.join(__dirname, '../artifacts/file-editor-workspace');
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
const entry = (name, directory = false, parent = '') => ({ name, relative_path: parent ? `${parent}/${name}` : name, is_dir: directory, size_bytes: 50, modified_at: 1700000000, mode: 420 });
async function fixture(browser, width = 1440, theme = 'dark') {
  const page = await browser.newPage({ viewport: { width, height: width < 640 ? 800 : 1000 }, reducedMotion: 'reduce' });
  page.setDefaultTimeout(10000);
  await page.addInitScript(theme => localStorage.setItem('nusa_theme', theme), theme);
  const files = new Map();
  for (const name of ['alpha.txt', 'beta.txt', 'locked.txt', 'slow.txt', 'later.txt', 'src/app.js', '.env', ...Array.from({ length: 105 }, (_, i) => `file-${String(i).padStart(3, '0')}.txt`)]) files.set(name, { content: `Original ${name}`, etag: `"${name}-v1"`, mtime: 1700000000 });
  const root = [entry('src', true), entry('retry-folder', true), ...[...files.keys()].filter(name => !name.includes('/')).map(name => entry(name)), { ...entry('link.txt'), is_symlink: true }];
  const state = { deny: true, folderError: true, slowRead: false, releaseRead: null, delayedPath: '', releaseSave: null, reads: [], writes: [], lists: [], errors: [], unexpectedWrites: [] };
  page.on('pageerror', error => state.errors.push(error.message));
  await page.route('**/api/v1/**', async route => {
    const req = route.request(), url = new URL(req.url()), endpoint = url.pathname;
    let status = 200, body = {};
    if (endpoint === '/api/v1/auth/me') body = { user: { id: 1, username: 'admin', role: 'admin' }, csrf_token: 'fixture' };
    else if (endpoint === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (endpoint === '/api/v1/tenants') body = { tenants: [{ id: 42, username: 'customer', role: 'user', status: 'active', linux_username: 'p_customer', linux_uid: 20042 }] };
    else if (endpoint === '/api/v1/files/list') {
      const folder = url.searchParams.get('path') || '', offset = Number(url.searchParams.get('offset') || 0), limit = Number(url.searchParams.get('limit') || 100);
      state.lists.push({ folder, offset, tenant: url.searchParams.get('tenant') });
      if (folder === 'retry-folder' && state.folderError) { status = 503; body = { error: 'Fixture folder unavailable' }; }
      else {
        const rows = (folder === '' ? root : folder === 'src' ? [entry('app.js', false, 'src'), entry('empty', true, 'src')] : []).filter(row => url.searchParams.get('show_hidden') === 'true' || !row.name.startsWith('.'));
        body = { entries: rows.slice(offset, offset + limit), total_count: rows.length, has_more: offset + limit < rows.length };
      }
    } else if (endpoint === '/api/v1/files/content' && req.method() === 'GET') {
      const name = url.searchParams.get('path'), file = files.get(name);
      state.reads.push({ path: name, tenant: url.searchParams.get('tenant') });
      if (name === 'slow.txt' && state.slowRead) await new Promise(resolve => { state.releaseRead = resolve; });
      if (name === 'locked.txt' && state.deny) { status = 403; body = { error: 'Fixture permission denied' }; }
      else if (!file) { status = 404; body = { error: 'Fixture missing file' }; }
      else body = { content: file.content, etag: file.etag, modified_at: file.mtime, size_bytes: Buffer.byteLength(file.content), read_only: name === '.env', is_binary: false };
    } else if (endpoint === '/api/v1/files/content' && req.method() === 'PUT') {
      const payload = req.postDataJSON(), file = files.get(payload.path);
      state.writes.push({ ...payload, csrf: req.headers()['x-csrf-token'] });
      if (state.delayedPath === payload.path) await new Promise(resolve => { state.releaseSave = resolve; });
      if (!payload.force && (payload.expected_etag !== file.etag || payload.expected_mtime !== file.mtime)) { status = 409; body = { error: 'Fixture real conflict', conflict: true }; }
      else { file.content = payload.content; file.mtime++; file.etag = `"${payload.path}-${file.mtime}"`; body = { etag: file.etag, modified_at: file.mtime, size_bytes: Buffer.byteLength(file.content) }; }
    } else if (endpoint.includes('notifications')) body = { notifications: [], count: 0, unread_count: 0, total: 0 };
    else if (endpoint === '/api/v1/jobs') body = { items: [], total: 0, total_pages: 0 };
    else { status = 404; body = { error: 'No fixture' }; }
    if (!['GET', 'HEAD'].includes(req.method()) && endpoint !== '/api/v1/files/content') state.unexpectedWrites.push(endpoint);
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  const modal = page.getByRole('dialog'), editor = modal.getByRole('tabpanel').locator('.cm-content');
  const sidebar = modal.getByRole('complementary', { name: 'Daftar berkas' });
  const save = modal.getByRole('button', { name: /^Simpan(?: Ctrl\+S)?$/ });
  const success = modal.getByRole('tabpanel').getByText('Berkas berhasil disimpan', { exact: true });
  const tab = name => modal.getByRole('tab', { name: new RegExp(`^${name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}(?: Belum disimpan| Menyimpan)?$`) });
  const open = async name => { await sidebar.getByRole('button', { name, exact: true }).click(); await tab(name).waitFor(); };
  const type = async text => { await editor.click(); await page.keyboard.press('Control+a'); await page.keyboard.insertText(text); await modal.getByRole('tabpanel').getByText('Belum disimpan', { exact: true }).waitFor(); };
  await page.goto(`${base}/files?tenant=42`, { waitUntil: 'domcontentloaded' });
  await page.locator('[data-file-item]').getByText('alpha.txt', { exact: true }).dblclick();
  try { await editor.waitFor(); }
  catch (error) {
    await page.screenshot({ path: path.join(output, `open-failure-${width}.png`) });
    console.error({ width, reads: state.reads, errors: state.errors, dialogs: await modal.count(), editors: await page.locator('.cm-content').count() });
    throw error;
  }
  return { page, modal, editor, sidebar, save, success, tab, open, type, state, files };
}
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge' });
  try {
    const { page, modal, editor, sidebar, save, success, tab, open, type, state, files } = await fixture(browser);
    check(await modal.count() === 1 && await sidebar.isVisible(), 'one workspace dialog contains both the explorer and editor');
    await type('Unsaved alpha'); await open('beta.txt'); await type('Unsaved beta');
    await tab('alpha.txt').click();
    check(await editor.innerText() === 'Unsaved alpha', 'switching tabs preserves independent unsaved drafts');
    check(state.reads.filter(read => read.path === 'alpha.txt').length === 1, 'reopening a tab does not refetch and discard its draft');
    await editor.click(); await page.keyboard.press('Control+z');
    check(await editor.innerText() === 'Original alpha.txt', 'undo history remains available after switching files');
    await page.keyboard.press('Control+y');
    check(await editor.innerText() === 'Unsaved alpha', 'redo restores the unsaved draft after a tab switch');
    await page.keyboard.press('Control+s'); await success.waitFor();
    check(state.writes.length === 1 && state.writes[0].path === 'alpha.txt' && files.get('beta.txt').content === 'Original beta.txt', 'Ctrl+S saves only the active file');
    const revision = files.get('alpha.txt').etag;
    await type('Alpha saved twice'); await save.click(); await success.waitFor();
    check(state.writes[1].expected_etag === revision, 'per-file save retains the repeated-save revision fix');
    await tab('beta.txt').click();
    files.get('beta.txt').content = 'External beta'; files.get('beta.txt').mtime++; files.get('beta.txt').etag = '"external"';
    await save.click(); await modal.getByText('Fixture real conflict', { exact: true }).waitFor();
    await tab('alpha.txt').click(); await tab('beta.txt').click();
    check(await editor.innerText() === 'Unsaved beta' && await modal.getByText('Fixture real conflict', { exact: true }).isVisible(), 'real conflict and draft stay attached to their file across tab switches');
    await modal.getByRole('button', { name: 'Simpan Paksa (Overwrite)' }).click(); await success.waitFor();
    check(files.get('beta.txt').content === 'Unsaved beta' && state.writes.at(-1).force, 'only explicit overwrite replaces the conflicting server version');
    await tab('alpha.txt').click(); await type('Alpha pending'); state.delayedPath = 'alpha.txt';
    await page.keyboard.press('Control+s'); await modal.getByRole('tabpanel').getByText('Menyimpan...', { exact: true }).waitFor();
    await open('later.txt'); await type('Later independent'); await save.click(); await success.waitFor();
    check(files.get('later.txt').content === 'Later independent' && files.get('alpha.txt').content === 'Alpha saved twice', 'another file can be edited and saved while an earlier save is pending');
    check(await modal.getByRole('button', { name: 'Tutup Editor', exact: true }).isDisabled(), 'workspace cannot close while any file is saving');
    await tab('alpha.txt').click(); await type('Alpha newer draft'); state.releaseSave(); await success.waitFor(); state.delayedPath = '';
    check(await editor.innerText() === 'Alpha newer draft' && await save.isEnabled(), 'switching away during save does not clear newer draft changes');
    await save.click(); await success.waitFor();
    await type('Alpha retained draft');
    await sidebar.getByRole('button', { name: 'locked.txt', exact: true }).click(); await modal.getByText('Fixture permission denied', { exact: true }).waitFor();
    check(await tab('locked.txt').count() === 0 && await editor.innerText() === 'Alpha retained draft', 'permission failure leaves the active draft intact');
    state.deny = false; await open('locked.txt');
    check(await editor.innerText() === 'Original locked.txt', 'failed open can be retried successfully');
    state.slowRead = true; await sidebar.getByRole('button', { name: 'slow.txt', exact: true }).click();
    await modal.getByRole('status').filter({ hasText: 'Membuka slow.txt' }).waitFor();
    await open('alpha.txt'); state.releaseRead(); state.slowRead = false;
    check(await editor.innerText() === 'Alpha retained draft' && await tab('slow.txt').count() === 0, 'cached file selection cancels a slower open without replacing the active draft');
    await sidebar.getByRole('button', { name: 'retry-folder', exact: true }).click(); await sidebar.getByText('Fixture folder unavailable', { exact: true }).waitFor();
    state.folderError = false; await sidebar.getByRole('button', { name: 'Coba lagi', exact: true }).click(); await sidebar.getByText('Folder kosong', { exact: true }).waitFor();
    check(true, 'directory errors expose a working retry and empty-folder state');
    await sidebar.getByRole('button', { name: 'src', exact: true }).click(); await open('app.js'); await type('console.log("draft")');
    await modal.getByRole('button', { name: 'Muat ulang daftar berkas' }).click();
    await sidebar.getByRole('button', { name: 'src', exact: true }).waitFor();
    check(await editor.innerText() === 'console.log("draft")', 'refreshing the directory tree preserves open drafts');
    await save.click(); await success.waitFor();
    check(state.writes.at(-1).path === 'src/app.js', 'nested file saves keep their complete tenant-relative path');
    await sidebar.getByRole('checkbox', { name: 'Berkas tersembunyi' }).check(); await open('.env');
    check(await modal.getByRole('tabpanel').getByText('Hanya Baca', { exact: true }).isVisible() && await editor.getAttribute('contenteditable') === 'false', 'hidden files can be browsed and retain server read-only restrictions');
    await sidebar.getByRole('button', { name: 'Muat lebih banyak', exact: true }).click(); await sidebar.getByRole('button', { name: 'file-104.txt', exact: true }).waitFor();
    check(state.lists.some(list => list.offset === 100), 'large folders remain browsable with explicit pagination');
    check(await sidebar.getByRole('button', { name: 'link.txt Tautan', exact: true }).isDisabled(), 'symbolic links are not followed by the sidebar');
    await open('file-000.txt'); await open('file-001.txt');
    check(await modal.getByRole('tab').count() === 8, 'workspace retains up to eight independent open files');
    const beforeLimit = state.reads.length;
    await sidebar.getByRole('button', { name: 'file-002.txt', exact: true }).click(); await modal.getByRole('alert').filter({ hasText: 'Maksimal 8 berkas' }).waitFor();
    check(state.reads.length === beforeLimit && await tab('alpha.txt').count() === 1, 'tab limit never evicts an unsaved file or sends an unnecessary read');
    await tab('alpha.txt').click(); page.once('dialog', dialog => dialog.dismiss());
    await modal.getByRole('button', { name: 'Tutup alpha.txt', exact: true }).click();
    check(await tab('alpha.txt').count() === 1 && await editor.innerText() === 'Alpha retained draft', 'closing a dirty tab can be cancelled');
    page.once('dialog', dialog => dialog.dismiss()); await modal.getByRole('button', { name: 'Tutup Editor', exact: true }).click();
    check(await modal.count() === 1, 'closing the workspace also protects drafts in every tab');
    await tab('beta.txt').focus(); await page.keyboard.press('ArrowRight');
    check(await tab('later.txt').getAttribute('aria-selected') === 'true', 'tabs support keyboard navigation');
    check(state.reads.every(read => read.tenant === '42') && state.lists.every(list => list.tenant === '42') && state.writes.every(write => write.tenant === '42' && write.csrf === 'fixture'), 'explorer, reads, and writes preserve explicit tenant scope and CSRF');
    check(state.errors.length === 0 && state.unexpectedWrites.length === 0, 'workspace triggers no browser errors or background file mutations');
    await tab('alpha.txt').click(); await page.screenshot({ path: path.join(output, 'desktop-dark.png') });
    page.once('dialog', dialog => dialog.accept()); await modal.getByRole('button', { name: 'Tutup alpha.txt', exact: true }).click();
    check(await tab('alpha.txt').count() === 0 && await modal.count() === 1, 'confirmed discard closes just that tab');
    for (const name of ['beta.txt', 'later.txt', 'locked.txt', 'src/app.js', '.env', 'file-000.txt', 'file-001.txt']) await modal.getByRole('button', { name: `Tutup ${name}`, exact: true }).click();
    await modal.getByText('Pilih berkas dari sidebar untuk mulai menyunting.', { exact: true }).waitFor();
    check(await modal.getByRole('tab').count() === 0 && await sidebar.isVisible(), 'closing the last tab keeps the workspace explorer usable');
    await open('alpha.txt');
    check(await editor.innerText() === files.get('alpha.txt').content, 'reopening a discarded tab reads its saved content again');
    await modal.getByRole('button', { name: 'Tutup Editor', exact: true }).click(); await modal.waitFor({ state: 'detached' });
    await page.close();
    for (const [width, theme] of [[320, 'light'], [390, 'dark']]) {
      const mobile = await fixture(browser, width, theme);
      const { page, modal, editor, sidebar, type, tab, open } = mobile;
      check(!await sidebar.isVisible(), `${width}/${theme}: sidebar starts collapsed on mobile`);
      await type('Mobile retained draft'); await modal.getByRole('button', { name: 'Tampilkan daftar berkas' }).click();
      await sidebar.getByRole('button', { name: 'src', exact: true }).click();
      await page.screenshot({ path: path.join(output, `mobile-${width}-${theme}-sidebar.png`) });
      await open('app.js');
      check(!await sidebar.isVisible() && await editor.innerText() === 'Original src/app.js', `${width}/${theme}: choosing a file collapses the sidebar and opens its editor`);
      await tab('alpha.txt').click();
      check(await editor.innerText() === 'Mobile retained draft', `${width}/${theme}: tab switch retains mobile drafts`);
      await page.screenshot({ path: path.join(output, `mobile-${width}-${theme}-editor.png`) });
      const dimensions = await modal.evaluate(el => { const rect = el.getBoundingClientRect(); return { left: rect.left, right: rect.right, viewport: innerWidth, scroll: document.documentElement.scrollWidth, panelWidth: el.querySelector('.editor-workspace').clientWidth, panelScroll: el.querySelector('.editor-workspace').scrollWidth }; });
      const fits = dimensions.left >= 0 && dimensions.right <= dimensions.viewport + 1 && dimensions.panelScroll <= dimensions.panelWidth;
      if (!fits) console.error(dimensions);
      check(fits, `${width}/${theme}: modal fits viewport without internal horizontal overflow`);
      check(mobile.state.errors.length === 0, `${width}/${theme}: no browser runtime errors`);
      page.once('dialog', dialog => dialog.accept()); await modal.getByRole('button', { name: 'Tutup Editor', exact: true }).click(); await page.close();
    }
    await fs.writeFile(path.join(output, 'results.json'), JSON.stringify({ checks, reads: state.reads, writes: state.writes, lists: state.lists, errors: state.errors }, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
