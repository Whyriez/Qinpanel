/* Built UI + synthetic tenant file APIs only. No actual tenant files or daemons. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5206';
const output = path.join(__dirname, '../artifacts/file-browser-workspace');
(async () => {
  const browser = await chromium.launch({ channel: 'msedge', headless: true });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1050 }, permissions: ['clipboard-read', 'clipboard-write'] });
  const page = await context.newPage(); page.setDefaultTimeout(10000);
  await fs.mkdir(output, { recursive: true });
  const errors = [], missing = [], calls = [], mutations = [], uploads = new Map();
  let role = 'user', fail = false, many = false, delay = false, checks = 0;
  const check = (condition, label) => { assert.ok(condition, label); console.log(`ok ${++checks}: ${label}`); };
  const entry = (name, is_dir = false, parent = '') => ({ name, relative_path: parent ? `${parent}/${name}` : name, is_dir, size_bytes: is_dir ? 4096 : 1536, modified_at: 1793358000, mode: is_dir ? 493 : 420, owner_user: 'p_alice', owner_group: 'panel-tenants', is_hidden: name.startsWith('.') });
  let rows = [entry('sites', true), entry('media', true), entry('empty', true), entry('index.php'), entry('config.json'), entry('photo + image.png'), entry('backup.zip'), entry('.env'), entry('index.js', false, 'sites'), entry('example.txt', false, 'sites')];
  let trash = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/api/v1/**', async route => {
    const req = route.request(), url = new URL(req.url()), p = url.pathname;
    calls.push({ path: p, query: Object.fromEntries(url.searchParams), method: req.method() });
    let body = {}, status = 200;
    if (!['GET', 'HEAD'].includes(req.method())) { assert.equal(req.headers()['x-csrf-token'], 'fixture'); mutations.push({ path: p, data: req.headers()['content-type']?.includes('json') && req.postData() ? req.postDataJSON() : null }); }
    if (p === '/api/v1/auth/me') body = { user: { id: role === 'admin' ? 1 : 2, username: 'alice', role, status: 'active' }, csrf_token: 'fixture' };
    else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
    else if (p === '/api/v1/notifications/unread-count') body = { unread_count: 0 };
    else if (p === '/api/v1/jobs') body = { items: [], total: 0, page: 1, total_pages: 1 };
    else if (p === '/api/v1/tenants') body = { tenants: [{ id: 42, username: 'alice', status: 'active', role: 'user', linux_username: 'p_alice', linux_uid: 10042 }] };
    else if (p === '/api/v1/files/list') {
      const folder = url.searchParams.get('path') || '', offset = Number(url.searchParams.get('offset') || 0);
      if (delay && folder === 'sites') await new Promise(resolve => setTimeout(resolve, 700));
      let list = (many && !folder ? Array.from({ length: 125 }, (_, index) => entry(`item-${index}.txt`)) : rows).filter(row => row.relative_path.split('/').slice(0, -1).join('/') === folder && (url.searchParams.get('show_hidden') === 'true' || !row.is_hidden));
      const field = url.searchParams.get('sort_by'), desc = url.searchParams.get('sort_order') === 'desc';
      list.sort((a, b) => a.is_dir !== b.is_dir ? (a.is_dir ? -1 : 1) : (desc ? -1 : 1) * (field === 'size' ? a.size_bytes - b.size_bytes : a.name.localeCompare(b.name)));
      body = { entries: list.slice(offset, offset + 100), total_count: list.length, has_more: offset + 100 < list.length };
      if (fail) { status = 503; body = { error: 'Fixture directory unavailable' }; }
    } else if (p === '/api/v1/files/mkdir') { const data = req.postDataJSON(), parts = data.path.split('/'); rows.push(entry(parts.pop(), true, parts.join('/'))); body = { success: true }; }
    else if (p === '/api/v1/files/rename') { const data = req.postDataJSON(), row = rows.find(row => row.relative_path === data.source); row.relative_path = data.destination; row.name = data.destination.split('/').at(-1); body = { success: true }; }
    else if (p === '/api/v1/files/copy' || p === '/api/v1/files/move') { const data = req.postDataJSON(), row = rows.find(row => row.relative_path === data.source); rows.push({ ...row, name: data.destination.split('/').at(-1), relative_path: data.destination }); if (p.endsWith('/move')) rows = rows.filter(row => row.relative_path !== data.source); body = { success: true }; }
    else if (p === '/api/v1/files/trash' && req.method() === 'POST') { const data = req.postDataJSON(), row = rows.find(row => row.relative_path === data.path); trash.push({ trash_id: 'trash-1', original_path: row.relative_path, trashed_at: 1793358000, size_bytes: row.size_bytes, is_dir: row.is_dir, row }); rows = rows.filter(row => row.relative_path !== data.path); body = { success: true }; }
    else if (p === '/api/v1/files/trash') body = { items: trash };
    else if (p === '/api/v1/files/trash/restore') { rows.push(trash[0].row); trash = []; body = { success: true }; }
    else if (p.includes('/archive/')) body = { success: true, job_id: 'fixture' };
    else if (p === '/api/v1/files/content' && req.method() === 'GET') body = { content: '<?php echo "fixture"; ?>', size_bytes: 25, modified_at: 1793358000, etag: '"v1"' };
    else if (p === '/api/v1/files/content') body = { size_bytes: req.postDataJSON().content.length, modified_at: 1793358001, etag: '"v2"' };
    else if (p === '/api/v1/files/search') body = { items: [], total_scanned: 7, limit_reached: false, timed_out: false, duration_ms: 1 };
    else if (p.endsWith('/chmod')) body = { success: true, message: 'Permissions updated' };
    else if (p.endsWith('/upload/init')) { const data = req.postDataJSON(), id = `upload-${uploads.size}`; uploads.set(id, { ...data, bytes: 0 }); body = { upload_id: id }; }
    else if (p.includes('/upload/') && p.endsWith('/chunk')) { const upload = uploads.get(p.split('/').at(-2)); await new Promise(resolve => setTimeout(resolve, 150)); upload.bytes += req.postDataBuffer()?.length || 0; body = { current_offset: upload.bytes, completed: upload.bytes >= upload.total_size }; if (body.completed) rows.push(entry(upload.file_name, false, upload.target_path.split('/').slice(0, -1).join('/'))); }
    else if (p === '/api/v1/ssh/connection-info') body = { host: null, port: null, username: 'p_alice', user_id: 2, home_dir: '/home/p_alice', active_path: '/home/p_alice/sites', relative_path: 'sites', shell_policy: 'none', can_shell: false, can_sftp: false, has_ssh_keys: false, ssh_keys_count: 0, endpoint_state: 'missing', endpoint_id: null, endpoint_revision: null, transport: null, host_keys: [], host_keys_status: 'unavailable', sftp_path: '/sites' };
    else { missing.push(`${req.method()} ${p}`); status = 404; body = { error: 'Unhandled fixture' }; }
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
  });
  const item = name => page.locator('[data-file-item]').filter({ has: page.getByText(name, { exact: true }) });
  const listing = page.locator('.fb-viewport');
  const ready = () => item('index.php').waitFor();
  const tools = () => page.getByRole('button', { name: 'Opsi pengelola berkas', exact: true });
  try {
    await page.goto(`${base}/files`); await ready();
    check(await page.getByRole('heading', { name: 'Pengelola Berkas', exact: true }).isVisible(), 'Clear file manager heading');
    check(await page.getByRole('complementary', { name: 'Navigasi folder', exact: true }).isVisible(), 'Folder navigation is available on desktop');
    check(await page.getByRole('button', { name: 'Terminal di sini', exact: true }).count() === 0, 'Advanced tools stay out of the primary toolbar');
    check(await page.locator('.fb-table').isVisible(), 'Default file list has distinct metadata columns');
    await page.screenshot({ path: path.join(output, 'files-desktop-light.png') });
    await page.getByLabel('Saring nama berkas', { exact: true }).fill('no-file');
    await page.getByText('Tidak ada berkas yang cocok', { exact: true }).waitFor();
    check(await page.getByText('Folder ini masih kosong', { exact: true }).count() === 0, 'Search no-match is distinct from an empty directory');
    await page.getByRole('button', { name: 'Hapus pencarian', exact: true }).first().click(); await ready();
    await item('index.php').click();
    await page.getByRole('region', { name: 'Detail berkas', exact: true }).waitFor();
    check(await page.getByText('p_alice', { exact: true }).isVisible(), 'Inspector exposes owner and permissions');
    await page.screenshot({ path: path.join(output, 'file-details-desktop.png') });
    await page.getByRole('region', { name: 'Tindakan berkas terpilih', exact: true }).getByRole('button', { name: 'Salin', exact: true }).click();
    await item('sites').dblclick();
    await item('index.js').waitFor();
    await page.getByRole('button', { name: 'Tempel di sini', exact: true }).click();
    await item('index.php').waitFor();
    check(mutations.some(call => call.path.endsWith('/copy') && call.data.source === 'index.php' && call.data.destination === 'sites/index.php'), 'Copy preserves source and destination paths');
    await page.getByRole('button', { name: 'Folder baru', exact: true }).click();
    const folder = page.getByRole('dialog', { name: 'Buat Folder Baru', exact: true });
    await folder.getByLabel('Nama Folder', { exact: true }).fill('assets + baru');
    await folder.getByRole('button', { name: 'Buat Folder', exact: true }).click();
    await item('assets + baru').waitFor(); check(rows.some(row => row.relative_path === 'sites/assets + baru'), 'New folders keep exact names and active path');
    await item('index.js').click();
    await page.getByRole('region', { name: 'Tindakan berkas terpilih', exact: true }).getByRole('button', { name: 'Ganti nama', exact: true }).click();
    const rename = page.getByRole('dialog', { name: 'Ganti Nama Berkas', exact: true });
    await rename.getByLabel('Nama Baru', { exact: true }).fill('app.js');
    await rename.getByRole('button', { name: 'Simpan Nama', exact: true }).click();
    await item('app.js').waitFor(); check(true, 'Rename uses the existing confirmation flow');
    await item('app.js').click();
    await page.getByRole('region', { name: 'Tindakan berkas terpilih', exact: true }).getByRole('button', { name: 'Arsipkan', exact: true }).click();
    const archive = page.getByRole('dialog', { name: 'Kompresi Menjadi Arsip', exact: true });
    await archive.getByRole('button', { name: 'Mulai Kompresi', exact: true }).click();
    await archive.waitFor({ state: 'hidden' }); check(mutations.some(call => call.path.endsWith('/archive/compress') && call.data.source_paths[0] === 'sites/app.js'), 'Archive keeps the selected full path');
    await item('app.js').click();
    await page.getByRole('region', { name: 'Tindakan berkas terpilih', exact: true }).getByRole('button', { name: 'Hapus', exact: true }).click();
    const remove = page.getByRole('dialog', { name: 'Pindahkan ke tempat sampah', exact: true });
    await page.keyboard.press('Escape'); check(rows.some(row => row.relative_path === 'sites/app.js'), 'Escape cancels deletion without a mutation');
    await page.getByRole('region', { name: 'Tindakan berkas terpilih', exact: true }).getByRole('button', { name: 'Hapus', exact: true }).click();
    await remove.getByRole('button', { name: 'Hapus ke Sampah', exact: true }).click();
    await item('app.js').waitFor({ state: 'hidden' }); check(trash.length === 1, 'Delete moves to trash through the existing API');
    await tools().click(); await page.locator('.fb-tools-menu').getByRole('button', { name: 'Tempat sampah', exact: true }).click();
    const trashDialog = page.getByRole('dialog', { name: 'Tempat Sampah', exact: true });
    await trashDialog.getByRole('button', { name: 'Pulihkan', exact: true }).click();
    await page.keyboard.press('Escape'); await item('app.js').waitFor(); check(true, 'Trash can restore the file');
    await page.getByRole('button', { name: 'Beranda', exact: true }).click(); await ready();
    await page.getByRole('button', { name: 'Tindakan untuk index.php', exact: true }).click();
    const menu = page.getByRole('group', { name: 'Tindakan berkas', exact: true }); await menu.waitFor();
    await page.keyboard.press('ArrowDown'); check(await menu.evaluate(element => element.contains(document.activeElement)), 'Row action menu supports keyboard focus');
    await page.keyboard.press('Escape'); check(await menu.count() === 0, 'Escape closes the file actions');
    await tools().click(); await page.getByLabel('Tampilkan berkas tersembunyi', { exact: true }).check();
    await page.keyboard.press('Escape'); await item('.env').waitFor(); check(true, 'Hidden-file setting uses the listing API');
    await page.getByRole('button', { name: 'Tampilan grid', exact: true }).click(); check(await page.locator('.fb-grid').isVisible(), 'Grid view remains selectable');
    await page.getByRole('button', { name: 'Tampilan rinci', exact: true }).click();
    await page.getByRole('button', { name: 'Pencarian lengkap', exact: true }).click();
    await page.getByRole('heading', { name: 'Pencarian Berkas & Konten', exact: true }).waitFor();
    await page.keyboard.press('Escape'); check(true, 'Advanced content search remains accessible');
    await page.getByRole('button', { name: 'Tindakan untuk index.php', exact: true }).click();
    await menu.getByRole('button', { name: 'Buka di Editor', exact: true }).click();
    await page.locator('.cm-content').waitFor(); check(true, 'File still opens in the existing editor workspace');
    await page.getByRole('button', { name: 'Tutup Editor', exact: true }).click();
    await page.getByLabel('Pilih berkas untuk diunggah', { exact: true }).setInputFiles({ name: 'upload + sample.txt', mimeType: 'text/plain', buffer: Buffer.from('file fixture upload') });
    await item('upload + sample.txt').waitFor(); check([...uploads.values()].some(upload => upload.bytes === upload.total_size), 'Upload sends all chunks and refreshes the file listing');
    const chunkRequest = page.waitForRequest(request => request.url().includes('/chunk?'));
    await page.getByLabel('Pilih berkas untuk diunggah', { exact: true }).setInputFiles([
      { name: 'batch-one.bin', mimeType: 'application/octet-stream', buffer: Buffer.alloc(2 * 1024 * 1024 + 23, 7) },
      { name: 'batch-two.txt', mimeType: 'text/plain', buffer: Buffer.from('second queued file') },
    ]);
    await chunkRequest; await item('sites').dblclick(); await item('example.txt').waitFor();
    await page.getByText('Selesai', { exact: true }).nth(2).waitFor();
    check([...uploads.values()].every(upload => upload.bytes === upload.total_size) && uploads.size === 3, 'Sequential multi-chunk uploads survive directory navigation');
    check([...uploads.values()].every(upload => !upload.target_path.includes('/')), 'Queued uploads retain their original destination');
    check(await item('batch-one.bin').count() === 0 && await item('example.txt').isVisible(), 'Upload completion refreshes the current directory without replacing its path');
    await page.getByRole('button', { name: 'Beranda', exact: true }).click(); await ready();
    await item('batch-one.bin').waitFor(); await page.getByRole('button', { name: 'Tutup antrean unggah', exact: true }).click();
    for (const theme of ['light', 'dark']) {
      await page.evaluate(dark => document.documentElement.classList.toggle('dark', dark), theme === 'dark');
      for (const width of [320, 375, 768, 1440]) {
        await page.setViewportSize({ width, height: 950 });
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `File manager fits ${width}px ${theme}`);
      }
    }
    await page.setViewportSize({ width: 375, height: 950 });
    await item('index.php').click();
    await page.getByRole('region', { name: 'Tindakan berkas terpilih', exact: true }).getByRole('button', { name: 'Detail', exact: true }).click();
    const detail = page.getByRole('dialog', { name: 'Detail berkas', exact: true }); await detail.waitFor();
    await detail.evaluate(element => Promise.all(element.getAnimations({ subtree: true }).map(animation => animation.finished)));
    check(await detail.locator('.fb-detail-sheet').boundingBox().then(box => box.width <= 375 && box.x >= 0), 'File detail sheet fits mobile');
    await page.screenshot({ path: path.join(output, 'file-details-mobile-dark.png') });
    check(await page.locator('#root').evaluate(element => element.inert), 'Mobile file details make background controls inert');
    await detail.getByRole('button', { name: 'Atur izin', exact: true }).focus(); await page.keyboard.press('Tab');
    check(await detail.getByRole('button', { name: 'Tutup detail berkas', exact: true }).evaluate(element => document.activeElement === element), 'Mobile file details keep keyboard focus inside the sheet');
    await page.keyboard.press('Escape');
    check(await page.getByRole('region', { name: 'Tindakan berkas terpilih', exact: true }).getByRole('button', { name: 'Detail', exact: true }).evaluate(element => document.activeElement === element), 'Closing details returns focus to the selected-file action');
    await page.getByRole('button', { name: 'Tampilkan navigasi folder', exact: true }).click();
    await page.getByRole('complementary', { name: 'Navigasi folder', exact: true }).getByRole('button', { name: 'media', exact: true }).click();
    await page.getByText('Folder ini masih kosong', { exact: true }).waitFor(); check(true, 'Mobile folder navigation opens the selected directory');
    await page.getByRole('button', { name: 'Beranda', exact: true }).click(); await ready();
    await page.locator('main').evaluate(element => { element.scrollTop = 0; });
    await page.screenshot({ path: path.join(output, 'files-mobile-dark.png') });
    fail = true; await page.getByRole('button', { name: 'Segarkan berkas', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: 'Fixture directory unavailable' }).waitFor(); check(await page.getByText('Folder ini masih kosong', { exact: true }).count() === 0, 'Failed directory listing is not displayed as empty');
    fail = false; await page.getByRole('button', { name: 'Coba lagi', exact: true }).click(); await ready();
    many = true; await page.getByRole('button', { name: 'Segarkan berkas', exact: true }).click();
    await item('item-0.txt').waitFor();
    await page.getByRole('button', { name: 'Muat lebih banyak', exact: true }).click();
    await page.waitForFunction(() => document.querySelectorAll('[data-file-item]').length === 125); check(true, 'Large directories support explicit pagination');
    many = false; await page.setViewportSize({ width: 1440, height: 1050 }); await page.getByRole('button', { name: 'Segarkan berkas', exact: true }).click(); await ready();
    delay = true; await item('sites').dblclick(); await page.getByRole('button', { name: 'Beranda', exact: true }).click(); await ready();
    await new Promise(resolve => setTimeout(resolve, 800)); check(await item('config.json').isVisible(), 'Late directory response cannot replace the current path');
    delay = false; role = 'admin'; await page.goto(`${base}/files?tenant=42`); await ready();
    check(calls.filter(call => call.path === '/api/v1/files/list').at(-1).query.tenant === '42', 'Admin file requests preserve the selected tenant');
    await page.getByRole('button', { name: 'Ganti bahasa', exact: true }).click();
    await page.getByRole('heading', { name: 'File manager', exact: true }).waitFor(); check(true, 'File manager provides English workspace labels');
    check(errors.length === 0, `No browser errors: ${errors.join(', ')}`);
    check(missing.length === 0, `All fixture endpoints handled: ${missing.join(', ')}`);
    console.log(`${checks} file browser checks passed`);
  } catch (error) { await page.screenshot({ path: path.join(output, 'failure.png') }); console.error({ uploads: [...uploads.values()], calls: calls.slice(-5), errors, missing }); throw error; }
  finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });

