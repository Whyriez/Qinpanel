/* Local production UI; intercepted APIs plus a loopback-only download fixture. */
const { chromium } = require('playwright');
const http = require('node:http');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5206';
const output = path.join(__dirname, '../artifacts/storage-workspace');
let png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aTmcAAAAASUVORK5CYII=', 'base64');
(async () => {
  await fs.mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge', headless: true });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, permissions: ['clipboard-read', 'clipboard-write'] });
  const page = await context.newPage();
  const thumbnail = await page.evaluate(() => {
    const canvas = document.createElement('canvas'); canvas.width=640; canvas.height=420;
    const ctx=canvas.getContext('2d');
    const sky=ctx.createLinearGradient(0,0,0,420); sky.addColorStop(0,'#bfd7dd'); sky.addColorStop(1,'#edf1e7'); ctx.fillStyle=sky; ctx.fillRect(0,0,640,420);
    ctx.fillStyle='#f5eee0'; ctx.beginPath();ctx.arc(452,104,38,0,Math.PI*2);ctx.fill();
    for (const [color,points] of [['#89aaa2',[[0,280],[135,120],[275,280],[415,168],[640,300]]],['#446d68',[[0,350],[180,220],[330,320],[520,210],[640,300]]],['#234c49',[[0,360],[190,300],[390,365],[640,315]]]]) {
      ctx.fillStyle=color;ctx.beginPath();ctx.moveTo(0,420);for(const [x,y] of points)ctx.lineTo(x,y);ctx.lineTo(640,420);ctx.closePath();ctx.fill();
    }
    return canvas.toDataURL('image/png').split(',')[1];
  });
  png=Buffer.from(thumbnail,'base64');
  const errors = [], missing = [], mutations = [], listingRequests = [];
  let checks = 0, failAccess = false, failDelete = false, role = 'user', impersonated = false;
  const check = (value, label) => { assert.ok(value, label); console.log(`ok ${++checks}: ${label}`); };
  let policy = { revision: 1, quota_bytes: 1073741824, used_bytes: 12400, object_count: 4, measured_at: 1, rules: { read: true, write: true, delete: true, max_file_bytes: 33554432, extensions: [] }, enforcement: 'panel', visibility: 'owner' };
  const connection = { id: '7c03851f-60dd-49d1-a3aa-a2100320e7c9', owner_id: 2, site_id: 1, label: 'Media project', provider: 'compatible', endpoint: 'https://storage.example.test', region: 'us-east-1', addressing: 'path', bucket: 'alice-media', revision: 1, verified_at: 1, has_credentials: true, managed: true, managed_state: 'ready', application_key_generation: 1, application_key_state: 'ready', created_at: 1, updated_at: 1 };
  let access = { revision: 1, visibility: 'private', desired_visibility: 'private', state: 'ready', request_id: null, can_change: true };
  const objects = [{ key: 'photo + café.png', size: png.length, modified_at: '2026-10-30T00:00:00Z' }, { key: 'archive.zip', size: 4200, modified_at: '2026-10-30T00:00:00Z' }, { key: 'report.pdf', size: 8200, modified_at: '2026-10-30T00:00:00Z' }];
  // Chromium downloads can be reissued outside Playwright interception. Serve
  // only the exact fixture object through Vite's existing loopback API proxy.
  const downloadFixture = http.createServer((req, res) => {
    const url = new URL(req.url, 'http://127.0.0.1');
    if (req.method !== 'GET' || url.pathname !== `/api/v1/storage/connections/${connection.id}/download` || url.searchParams.get('key') !== objects[0].key || url.searchParams.get('expected_revision') !== '1') {
      missing.push(`download fixture: ${req.method} ${url.pathname}`); res.writeHead(404).end(); return;
    }
    res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Length': png.length, 'Content-Disposition': 'attachment; filename="photo.png"', 'Cache-Control': 'no-store' }); res.end(png);
  });
  page.on('pageerror', error => errors.push(error.message));
  await context.route('https://storage.example.test/**', route => route.fulfill({ status: 200, contentType: 'image/png', body: png }));
  await context.route('**/api/v1/**', async route => {
    const req = route.request(), url = new URL(req.url()), p = url.pathname, method = req.method();
    let body = {}, code = 200;
    if (method === 'POST') {
      assert.equal(req.headers()['x-csrf-token'], 'fixture-csrf');
      if (p.endsWith('/upload')) {
        const key = url.searchParams.get('key');
        objects.push({ key, size: (req.postDataBuffer()?.length ?? 0), modified_at: '2026-10-30T01:00:00Z' });
        mutations.push({ upload: key });
      } else if (p.endsWith('/policy')) {
        const data = req.postDataJSON(); assert.equal(data.expected_revision, policy.revision); mutations.push(data); policy = { ...policy, rules: data.rules, revision: policy.revision + 1 }; body = policy;
      } else if (p.endsWith('/test')) { body = { success: true, revision: 1, verified_at: 1 };
      } else if (p.endsWith('/access')) {
        const data = req.postDataJSON(); mutations.push(data);
        if (access.state === 'unknown') assert.equal(data.request_id, access.request_id);
        else { assert.equal(data.expected_revision, access.revision); access.revision++; }
        access = { ...access, request_id: data.request_id, desired_visibility: data.visibility, state: failAccess ? 'unknown' : 'ready' };
        if (!failAccess) access.visibility = data.visibility;
        body = failAccess ? { error: 'Hasil perubahan akses belum dapat dipastikan.' } : access; code = failAccess ? 503 : 200;
      } else if (p.endsWith('/link')) {
        const data = req.postDataJSON(); mutations.push(data);
        body = { kind: 'signed', url: `https://storage.example.test/alice-media/${data.key.split('/').map(encodeURIComponent).join('/')}?X-Amz-Signature=fixture`, expires_at: Math.floor(Date.now()/1000)+data.expires_in };
      } else if (p.endsWith('/delete')) {
        const data = req.postDataJSON(); mutations.push({ delete: data.key });
        assert.equal(data.confirmation, data.key);
        const index = objects.findIndex(object => object.key === data.key);
        assert.ok(index >= 0, 'Delete must address the exact listed key');
        if (failDelete) { code = 503; body = { error: 'Provider menerima permintaan hapus, tetapi file belum dipastikan hilang. Muat ulang daftar sebelum mencoba kembali.' }; }
        else { objects.splice(index, 1); body = { success: true }; }
      } else if (p.endsWith('/download')) {
        const data = req.postDataJSON(), expires_at = Math.floor(Date.now()/1000)+60;
        body = { url: `${p}?expected_revision=1&key=${encodeURIComponent(data.key)}&expires_at=${expires_at}`, expires_at };
      } else { missing.push(`${method} ${p}`); code = 405; }
    } else if (p === '/api/v1/auth/me') body = { user: { id: 2, username: 'alice', role, status: 'active', impersonated_by: impersonated ? 'admin' : null }, csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture-csrf' };
    else if (p === '/api/v1/storage/context') body = { owners: [{ id: 2, username: 'alice' }], sites: [{ id: 1, owner_id: 2, domain: 'project.example.test' }], upload_limit_bytes: 33554432 };
    else if (p === '/api/v1/storage/connections') body = { connections: [connection], upload_limit_bytes: 33554432 };
    else if (p.endsWith('/access')) body = access;
    else if (p.endsWith('/objects')) {
      const prefix = url.searchParams.get('prefix') ?? ''; listingRequests.push(prefix);
      const matches = objects.filter(object => object.key.startsWith(prefix));
      const files = matches.filter(object => !object.key.slice(prefix.length).includes('/'));
      const prefixes = [...new Set(matches.filter(object => object.key.slice(prefix.length).includes('/')).map(object => prefix + object.key.slice(prefix.length).split('/')[0] + '/'))];
      body = { listing: { objects: files, prefixes, next_token: null } };
    }
    else if (p.endsWith('/policy')) body = policy;
    else if (p.endsWith('/download')) {
      if (!url.searchParams.has('preview')) { await route.continue(); return; }
      await route.fulfill({ contentType: 'image/png', headers: { 'Cache-Control': 'no-store' }, body: png }); return;
    }
    else if (p === '/api/v1/storage/managed') body = { configured: true, running: true, endpoint: connection.endpoint, engine_version: '4.50', data_directory: '/var/lib/qonpanel-storage', bucket_count: 1, quota_enforcement: 'none' };
    else if (p === '/api/v1/storage/managed/tls') body = { certificate: { revision: 1, state: 'active', auto_renew: false, method: 'tunnel' }, dns: { available: false, configured: false, zone_name: null } };
    else if (p.includes('notifications')) body = { notifications: [], total: 0, unread_count: 0 };
    else if (p === '/api/v1/jobs') body = { items: [], total: 0, page: 1, per_page: 100, total_pages: 0 };
    else { missing.push(`${method} ${p}`); code = 404; }
    await route.fulfill({ status: code, contentType: 'application/json', body: JSON.stringify(body) });
  });
  try {
    await new Promise((resolve, reject) => { downloadFixture.once('error', reject); downloadFixture.listen(7700, '127.0.0.1', resolve); });
    await page.goto(`${base}/storage`);
    await page.getByRole('region', { name: 'File browser', exact: true }).waitFor();
    await page.getByRole('button', { name: /archive.zip/ }).waitFor();
    check(await page.getByRole('button', { name: 'Tampilan grid', exact: true }).getAttribute('aria-pressed') === 'true', 'workspace opens in image-friendly grid view');
    check(await page.getByRole('button', { name: 'Uji koneksi', exact: true }).count() === 0, 'connection configuration does not crowd the file workspace');
    await page.getByLabel('Cari bucket',{exact:true}).fill('missing-bucket');
    check(await page.getByText('Bucket tidak ditemukan.',{exact:true}).isVisible(), 'bucket search gives an explicit no-match state');
    await page.getByLabel('Cari bucket',{exact:true}).fill('alice');
    check(await page.locator('.storage-bucket-nav-list .storage-connection').count()===1, 'bucket search preserves matching navigation');
    await page.getByLabel('Cari bucket',{exact:true}).fill('');
    await page.getByRole('tab', { name: 'File', exact: true }).focus(); await page.keyboard.press('End');
    check(await page.getByRole('button', { name: 'Uji koneksi', exact: true }).isVisible(), 'keyboard navigation reaches the separate connection tab');
    await page.keyboard.press('Home');
    await page.getByLabel('Jenis file', { exact: true }).selectOption('image');
    check(await page.locator('.storage-file-list > li').count() === 1, 'file type filter shows images only on the active page');
    await page.getByLabel('Jenis file', { exact: true }).selectOption('all');
    await page.getByLabel('Urutkan file', { exact: true }).selectOption('size');
    check((await page.locator('.storage-file-list > li').first().textContent()).includes([...objects].sort((a,b)=>b.size-a.size)[0].key), 'size sort puts the largest file first');
    await page.getByLabel('Urutkan file', { exact: true }).selectOption('name');
    await page.getByRole('button', { name: 'Tampilan daftar', exact: true }).click();
    check(await page.locator('.storage-list-heading').isVisible(), 'list view presents aligned file metadata columns');
    await page.getByRole('button', { name: 'Tampilan grid', exact: true }).click();
    await page.screenshot({ path: path.join(output, 'workspace-overview.png'), fullPage: true });
    await page.getByRole('button', { name: /photo \+ café.png/ }).click();
    const detail = page.getByRole('complementary', { name: 'Detail file', exact: true });
    await detail.locator('.storage-preview img').waitFor();
    check(await detail.getByText('Private', { exact: true }).count() === 1, 'private file has image preview and explicit access');
    await detail.getByLabel(/^Masa berlaku/).selectOption('86400');
    await detail.getByRole('button', { name: 'Dapatkan URL', exact: true }).click();
    await detail.getByRole('link', { name: 'Buka URL', exact: true }).waitFor();
    check((await detail.getByLabel('URL akses sementara', { exact: true }).inputValue()).includes('X-Amz-Signature='), 'private URL is shareable S3 signed URL');
    check(mutations.at(-1).expires_in === 86400, 'selected signed URL duration is submitted');
    await detail.getByRole('button', { name: 'Salin URL', exact: true }).click();
    check((await page.evaluate(() => navigator.clipboard.readText())).includes('X-Amz-Signature='), 'copy URL writes the actual access URL');
    const download = page.waitForEvent('download'); await detail.getByRole('button', { name: 'Unduh file', exact: true }).click();
    const downloaded = await download;
    check(await downloaded.failure() === null, 'download icon completes without a browser download error');
    assert.deepEqual(await fs.readFile(await downloaded.path()), png, 'Downloaded bytes must match the storage object');
    await detail.getByRole('button', { name: 'Tutup detail file', exact: true }).click();
    await page.getByRole('tab', { name: 'Pengaturan bucket', exact: true }).click();
    const editor = page.getByRole('region', { name: 'Akses bucket', exact: true });
    await editor.scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(output, 'bucket-settings.png') });
    await editor.getByRole('radio', { name: /^Public/ }).check();
    check(access.visibility === 'private', 'choosing Public does not publish before saving');
    await editor.getByRole('button', { name: 'Simpan akses bucket', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.storage-workspace-heading .storage-access-badge')?.textContent === 'Public');
    await page.getByRole('tab', { name: 'File', exact: true }).click();
    await page.getByRole('button', { name: 'Tampilan grid', exact: true }).click();
    await page.getByRole('button', { name: /photo \+ café.png/ }).click();
    check(await detail.getByLabel('URL publik', { exact: true }).inputValue() === 'https://storage.example.test/alice-media/photo%20%2B%20caf%C3%A9.png', 'Public URL is canonical, permanent, and has no signature');
    check(await detail.getByRole('button', { name: 'Dapatkan URL', exact: true }).count() === 0, 'public detail exposes URL without minting a download');
    check(await page.locator('.storage-file-thumbnail img').count() > 0, 'grid shows image thumbnails');
    await page.getByRole('region', { name: 'File browser', exact: true }).scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(output, 'file-workspace.png') });
    for (const theme of ['light', 'dark']) {
      await page.evaluate(theme => document.documentElement.classList.toggle('dark', theme === 'dark'), theme);
      for (const [width, height] of [[1440, 1000], [375, 812], [320, 700]]) {
        await page.setViewportSize({ width, height });
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `${theme} ${width}: no page overflow`);
        await detail.scrollIntoViewIfNeeded();
        check((await detail.boundingBox()).width <= width, `${theme} ${width}: file details fit`);
        if (width !== 375) await page.screenshot({ path: path.join(output, `${theme}-${width}.png`), fullPage: true });
          if (theme === 'dark' && width === 320) {
            const drawer=page.getByRole('dialog',{name:'Detail file',exact:true});
            check(await drawer.isVisible(), 'small-screen file details use a focused drawer');
            await page.keyboard.press('Shift+Tab');
            check(await page.evaluate(()=>!!document.activeElement?.closest('.storage-detail-overlay')), 'drawer keeps keyboard focus inside its controls');
            await page.keyboard.press('Escape'); await drawer.waitFor({state:'hidden'});
            check(await page.evaluate(()=>document.activeElement?.classList.contains('storage-file-row')), 'Escape closes details and restores focus to the file');
            await page.getByRole('heading',{name:'Storage S3',exact:true}).scrollIntoViewIfNeeded();
            await page.screenshot({path:path.join(output,'mobile-workspace.png'),fullPage:true});
            await page.getByRole('button',{name:/photo \+ café.png/}).click();
          }
      }
    }
    await page.setViewportSize({ width: 1440, height: 1000 });
    await detail.getByRole('button', { name: 'Tutup detail file', exact: true }).click();
    await page.getByRole('button', { name: 'Upload file', exact: true }).click();
    const uploadDialog=page.getByRole('dialog',{name:'Upload ke bucket',exact:true});
    check(await uploadDialog.isVisible(), 'upload has a focused dialog instead of pushing the file list down');
    const transfer=await page.evaluateHandle(()=>{const data=new DataTransfer();data.items.add(new File(['fixture archive'],'dragged.zip',{type:'application/zip'}));return data;});
    await uploadDialog.locator('.storage-file-picker').dispatchEvent('drop',{dataTransfer:transfer});
    check(await uploadDialog.getByLabel('Object key',{exact:true}).inputValue()==='dragged.zip', 'drop target selects a real file and preserves its name');
    check(await page.locator('input[type=file]').getAttribute('accept') === null, 'upload allows arbitrary file formats');
    await page.locator('input[type=file]').setInputFiles({ name: 'video.mp4', mimeType: 'video/mp4', buffer: Buffer.from('fixture video') });
    await page.waitForFunction(()=>{const panel=document.querySelector('.storage-upload-card');return panel && getComputedStyle(panel).opacity==='1';});
    await page.screenshot({path:path.join(output,'upload-dialog.png')});
    await page.setViewportSize({width:320,height:700});
    check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1), 'upload dialog fits a 320px screen without horizontal overflow');
    const uploadBounds=await uploadDialog.locator('.dialog-panel').boundingBox();
    check(uploadBounds.x>=0 && uploadBounds.x+uploadBounds.width<=321 && uploadBounds.y>=0 && uploadBounds.y+uploadBounds.height<=701, 'upload dialog stays inside the small-screen viewport');
    await page.screenshot({path:path.join(output,'mobile-upload.png')});
    await page.setViewportSize({width:1440,height:1000});
    await page.getByRole('checkbox', { name: /Izinkan menimpa/ }).check();
    await page.getByRole('button', { name: 'Mulai upload', exact: true }).click();
    await page.getByRole('button', { name: /video.mp4/ }).waitFor();
    check(mutations.some(item => item.upload === 'video.mp4'), 'non-image files use the same storage upload flow');
    check(listingRequests.at(-1) === '', 'upload returns to the containing folder instead of a filename prefix');
    check(await page.getByRole('button', { name: 'Ke atas', exact: true }).count() === 0, 'root upload does not create a fake folder level');
    await detail.getByRole('button', { name: 'Tutup detail file', exact: true }).click();
    const uploadedName = 'WhatsApp Image 2026-10-02 at 20.58.22.jpeg';
    const uploadedKey = `folder space/${uploadedName}`;
    await page.getByRole('button', { name: 'Upload file', exact: true }).click();
    await page.locator('input[type=file]').setInputFiles({ name: uploadedName, mimeType: 'image/jpeg', buffer: png });
    await page.getByLabel('Object key', { exact: true }).fill(uploadedKey);
    await page.getByRole('checkbox', { name: /Izinkan menimpa/ }).check();
    await page.getByRole('button', { name: 'Mulai upload', exact: true }).click();
    await detail.getByLabel('URL publik', { exact: true }).waitFor();
    const uploadedUrl = await detail.getByLabel('URL publik', { exact: true }).inputValue();
    check(uploadedUrl.endsWith('/folder%20space/WhatsApp%20Image%202026-10-02%20at%2020.58.22.jpeg'), 'upload detail preserves the exact filename spaces');
    check(listingRequests.at(-1) === 'folder space/', 'nested upload lists its folder, not its filename');
    check(await page.getByRole('alert').count() === 0, 'successful upload and folder refresh do not show Request failed');
    await detail.getByRole('button', { name: 'Tutup detail file', exact: true }).click();
    await page.getByRole('button', { name: 'Ke atas', exact: true }).click();
    await page.getByLabel('Jenis file',{exact:true}).selectOption('image');
    check(await page.getByRole('button', { name: 'folder space Folder', exact: true }).isVisible(), 'image filtering keeps folders available for navigation');
    await page.getByRole('button', { name: 'folder space Folder', exact: true }).click();
    await page.getByRole('button', { name: new RegExp(uploadedName.replaceAll('.', '\\.')) }).click();
    check(await detail.getByLabel('URL publik', { exact: true }).inputValue() === uploadedUrl, 'URL remains identical after navigating up and reopening the listed image');
    failDelete = true;
    await detail.getByRole('button', { name: 'Hapus file', exact: true }).click();
    let danger = page.getByRole('dialog', { name: 'Hapus file?', exact: true });
    await danger.getByRole('textbox').fill(uploadedKey);
    await danger.getByRole('button', { name: 'Hapus file', exact: true }).click();
    await page.getByRole('alert').getByText(/belum dipastikan hilang/).waitFor();
    check(await page.getByText('File berhasil dihapus.', { exact: true }).count() === 0, 'unconfirmed deletion never shows a success message');
    check(objects.some(object => object.key === uploadedKey), 'unconfirmed delete retains the object for readback');
    await danger.getByRole('button', { name: 'Batal', exact: true }).click();
    await page.getByRole('button', { name: 'Muat ulang file', exact: true }).click(); failDelete = false;
    await detail.getByRole('button', { name: 'Hapus file', exact: true }).click();
    danger = page.getByRole('dialog', { name: 'Hapus file?', exact: true });
    await danger.getByRole('textbox').fill(uploadedKey);
    await danger.getByRole('button', { name: 'Hapus file', exact: true }).click();
    await page.getByText('File berhasil dihapus.', { exact: true }).waitFor();
    await page.getByRole('button', { name: new RegExp(uploadedName.replaceAll('.', '\\.')) }).waitFor({ state: 'hidden' });
    check(!objects.some(object => object.key === uploadedKey), 'confirmed deletion removes the exact spaced key from storage and the listing');
    check(mutations.filter(item => item.delete).every(item => item.delete === uploadedKey), 'delete requests never substitute plus signs for spaces');
    await page.getByRole('tab', { name: 'Pengaturan bucket', exact: true }).click();
    await editor.getByRole('radio', { name: /^Private/ }).check(); failAccess = true;
    await editor.getByRole('button', { name: 'Simpan akses bucket', exact: true }).click();
    await editor.getByRole('button', { name: 'Sinkronkan perubahan akses', exact: true }).waitFor();
    check(access.state === 'unknown' && access.visibility === 'public', 'uncertain revoke is not displayed as confirmed Private');
    const requestId = access.request_id; failAccess = false;
    await editor.getByRole('button', { name: 'Sinkronkan perubahan akses', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.storage-workspace-heading .storage-access-badge')?.textContent === 'Private');
    check(access.request_id === requestId && access.visibility === 'private', 'reconciliation resumes the same request');
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.getByRole('tab', { name: 'Koneksi S3', exact: true }).click();
    await page.getByRole('heading', { name: 'Hubungkan aplikasi Anda', exact: true }).waitFor();
    await page.getByRole('button', { name: 'Uji koneksi', exact: true }).click();
    await page.getByRole('heading', { name: 'Hubungkan aplikasi Anda', exact: true }).waitFor();
    check(await page.getByRole('tab', { name: 'Koneksi S3', exact: true }).getAttribute('aria-selected') === 'true', 'testing S3 connection preserves the connection tab');
    await page.getByRole('button', { name: 'Salin Endpoint', exact: true }).click();
    check(await page.evaluate(() => navigator.clipboard.readText()) === connection.endpoint, 'S3 endpoint can be copied directly');
    await page.getByRole('button', { name: 'Salin konfigurasi', exact: true }).click();
    check((await page.evaluate(() => navigator.clipboard.readText())).includes('AWS_BUCKET=alice-media'), 'environment copy contains selected bucket configuration');
    await page.screenshot({ path: path.join(output, 's3-connection-improved.png'), fullPage: true });
    await page.getByRole('tab', { name: 'Pengaturan bucket', exact: true }).click();
    const rules = page.getByRole('form', { name: 'Atur rules bucket', exact: true });
    await rules.getByRole('switch', { name: 'Upload', exact: true }).waitFor();
    check(await rules.getByRole('switch').count() === 3, 'rules expose three explicit operation switches');
    await rules.getByRole('switch', { name: 'Upload', exact: true }).uncheck();
    check(await rules.getByText('Ada perubahan yang belum disimpan.', { exact: true }).isVisible(), 'rules show unsaved changes');
    await rules.getByRole('button', { name: 'Batalkan perubahan', exact: true }).click();
    check(await rules.getByRole('switch', { name: 'Upload', exact: true }).isChecked(), 'discard restores saved permission values');
    await rules.getByRole('button', { name: 'Gambar', exact: true }).click();
    await rules.getByLabel('Maksimum file (MiB)', { exact: true }).fill('8');
    await rules.getByRole('button', { name: 'Simpan rules', exact: true }).click();
    await rules.getByText('Pengaturan sudah tersimpan.', { exact: true }).waitFor();
    check(policy.rules.max_file_bytes === 8388608 && policy.rules.extensions.includes('png'), 'saving rules submits normalized extensions and bytes');
    await rules.scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(output, 'bucket-rules-improved.png'), fullPage: true });
    await page.getByRole('tab', { name: 'File', exact: true }).click();
    await page.getByRole('button', { name: 'Buat folder', exact: true }).click();
    const folderDialog = page.getByRole('dialog', { name: 'Buat folder', exact: true });
    await folderDialog.getByLabel('Nama folder', { exact: true }).fill('Folder + baru');
    await folderDialog.getByLabel(/Izinkan penulisan penanda folder/).check();
    await page.screenshot({ path: path.join(output, 'new-folder-dialog.png') });
    await folderDialog.getByRole('button', { name: 'Buat folder', exact: true }).click();
    await page.getByRole('button', { name: 'Folder + baru Folder', exact: true }).waitFor();
    check(objects.some(object => object.key === 'Folder + baru/' && object.size === 0), 'new folder writes an exact zero-byte S3 marker');
    await page.getByRole('button', { name: 'Folder + baru Folder', exact: true }).click();
    await page.getByText('Simpan file pertama Anda', { exact: true }).waitFor();
    check(await page.locator('.storage-file-list > li').count() === 0, 'folder marker is hidden inside its own empty folder');
    await page.getByRole('button', { name: 'Ke atas', exact: true }).click();
    objects.push({ key: 'nested/search report.txt', size: 10, modified_at: '2026-10-30T01:00:00Z' });
    await page.getByLabel('Cari file dan folder', { exact: true }).fill('search report');
    await page.getByText('Tidak ada file yang cocok', { exact: true }).waitFor();
    await page.getByLabel('Cakupan pencarian', { exact: true }).selectOption('recursive');
    await page.getByRole('button', { name: /search report.txt/ }).waitFor();
    check(listingRequests.includes('nested/'), 'search can include nested folders beyond the active page');
    await page.getByRole('button', { name: /search report.txt/ }).click();
    check(await detail.getByText('nested/search report.txt', { exact: true }).isVisible(), 'search result opens details using its full exact key');
    await detail.getByRole('button', { name: 'Tutup detail file', exact: true }).click();
    await page.getByRole('button', { name: 'Hapus pencarian', exact: true }).click();
    check(await page.getByLabel('Cari file dan folder', { exact: true }).inputValue() === '', 'search clear resets the query and returns to folder listing');
    for (const width of [320, 375, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      for (const tab of ['File', 'Pengaturan bucket', 'Koneksi S3']) {
        await page.getByRole('tab', { name: tab, exact: true }).click();
        check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `improved ${tab} fits ${width}px`);
      }
    }
    impersonated = true; await page.reload();
    await page.getByRole('heading', { name: 'Workspace file milik tenant', exact: true }).waitFor();
    check(await page.getByRole('region', { name: 'File browser', exact: true }).count() === 0, 'impersonated session has no file workspace');
    impersonated = false; role = 'admin'; await page.reload();
    await page.getByRole('heading', { name: 'Workspace file milik tenant', exact: true }).waitFor();
    check(await page.getByRole('region', { name: 'File browser', exact: true }).count() === 0, 'admin retains only allocation and metadata controls');
    check(errors.length === 0, `no browser errors: ${errors.join(', ')}`);
    check(missing.length === 0, `all requests handled by fixtures: ${missing.join(', ')}`);
    console.log(`${checks} storage workspace UI checks passed.`);
  } catch (error) { await page.screenshot({ path: path.join(output, 'failure.png'), fullPage: true }); throw error; }
  finally { await browser.close(); if (downloadFixture.listening) await new Promise(resolve => downloadFixture.close(resolve)); }
})().catch(error => { console.error(error); process.exitCode = 1; });
