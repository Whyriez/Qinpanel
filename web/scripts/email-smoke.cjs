// Every panel API is intercepted. Never starts panel-api, panel-agent, or a mail service.
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs/promises');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5194';
let checks = 0;
function check(value, label) { assert.ok(value, label); console.log(`ok ${++checks}: ${label}`); }
const mailboxId = '01234567-89ab-4cde-8fab-0123456789ab';
const job = (name, payload) => ({ id: `job-${Date.now()}-${Math.random()}`, user_id: 1, name, payload_json: JSON.stringify(payload), status: 'queued', progress: 0, retry_count: 0, max_retries: 0, result_json: null, error_message: null, created_at: 1, updated_at: 1 });
(async () => {
  const browser = await chromium.launch({ headless: true, channel: process.env.QPANEL_BROWSER_CHANNEL || 'msedge' });
  const errors = [], requests = [], jobs = new Map(); let role = 'admin'; let denied = false; let validity = 7;
  const overview = { mailboxes: [], domains: [{ id: 1, name: 'example.com' }], server: { configured: true, server: { hostname: 'mail.example.com', certificate_id: 1 }, services: ['postfix', 'dovecot', 'opendkim'].map(name => ({ name, running: true })) }, server_error: null, certificates: [{ id: 1, hostname: 'mail.example.com', san_domains: [] }] };
  let template = { panel_name: 'Qpanel', subject: '[{panel_name}] {title}', body: '{message}\n{action_url}' };
  const settings = { smtp_enabled: false, smtp_host: 'smtp.example.com', smtp_port: 587, smtp_username: 'fixture', smtp_has_password: true, smtp_encryption: 'starttls', from_address: 'panel@example.com', from_name: 'Brand', admin_email: 'admin@example.com', ssl_warning_days: 30, quota_warning_percent: 90, notify_ssl_expiring: true, notify_quota_warning: true, notify_job_failed: true, notify_deploy_failed: true };
  const enqueue = (name, payload) => { const item = job(name, payload); jobs.set(item.id, item); return item; };
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 1050 } }); page.setDefaultTimeout(15000);
    page.on('pageerror', e => errors.push(e.message));
    await page.addInitScript(()=>Object.defineProperty(crypto,'randomUUID',{value:undefined,configurable:true}));
    await page.route('**/api/v1/**', async route => {
      const req = route.request(), url = new URL(req.url()), p = url.pathname; let code = 200, body = {};
      if (p === '/api/v1/auth/me') body = { user: { id: 1, role, username: 'fixture' }, csrf_token: 'fixture' };
      else if (p === '/api/v1/auth/csrf') body = { csrf_token: 'fixture' };
      else if (p === '/api/v1/email') body = denied ? { error: 'Fixture: email belum tersedia' } : overview, code = denied ? 503 : 200;
      else if (p === '/api/v1/email/templates') { if (req.method() === 'PUT') template = req.postDataJSON(); body = template; }
      else if (p === '/api/v1/email/mailboxes') {
        const data = req.postDataJSON(); requests.push({ p, data });
        const item = enqueue('email.mailbox', { id: mailboxId });
        overview.mailboxes.push({ id: mailboxId, address: `${data.local_part}@example.com`, domain_id: 1, quota_mb: data.quota_mb, state: 'pending', enabled: true, operation: 'apply', job_id: item.id, error: null });
        body = { id: mailboxId, address: overview.mailboxes[0].address, password: 'GeneratedFixture123!', job: item }; code = 202;
      } else if (p.endsWith('/messages') && p.startsWith('/api/v1/email/')) {
        if (req.method() === 'GET') body = { items: [{ uid: '42', 'hdr.subject': 'Halo dari pelanggan', 'hdr.from': 'Pelanggan <customer@example.net>', 'hdr.to': 'admin@example.com', flags: '' }], uid_validity: validity, total: 1, unseen: 1, before: null };
        else { const data = req.postDataJSON(); requests.push({ p, data }); body = data.action === 'read' ? { uid: '42', 'hdr.subject': 'Halo dari pelanggan', 'hdr.from': 'customer@example.net', 'hdr.to': 'admin@example.com', 'text.utf8': '<script>window.emailInjected=true</script>\nPesan aman sebagai teks.' } : { moved: true }; }
      } else if (p.endsWith('/send')) { requests.push({ p, data: req.postDataJSON() }); await new Promise(r => setTimeout(r, 250)); body = { job: enqueue('email.send', {}) }; code = 202; }
      else if (p.endsWith('/dns') && p.startsWith('/api/v1/email/')) body = { hostname: 'mail.example.com', ipv4: '203.0.113.10', ipv6: null, records: [{ type: 'MX', name: 'example.com', content: 'mail.example.com', priority: 10 }, { type: 'TXT', name: 'qpanel._domainkey.example.com', content: 'v=DKIM1; p=PUBLIC' }] };
      else if (p === `/api/v1/email/mailboxes/${mailboxId}`) {
        const data = req.postDataJSON(); requests.push({ p, data }); const item = enqueue('email.mailbox', { id: mailboxId }); overview.mailboxes[0].state = 'pending'; overview.mailboxes[0].job_id = item.id;
        body = { address: overview.mailboxes[0].address, password: data.action === 'reset_password' ? 'ResetFixture123!' : null, job: item }; code = 202;
      } else if (p === '/api/v1/email/install') { requests.push({ p, data: req.postDataJSON() }); body = { job: enqueue('email.install', req.postDataJSON()) }; code = 202; }
      else if (p === '/api/v1/jobs') { const items = [...jobs.values()].filter(j => !url.searchParams.get('status') || j.status === url.searchParams.get('status')); body = { items, total: items.length, total_pages: 1, page: 1, per_page: 100 }; }
      else if (/^\/api\/v1\/jobs\/[^/]+$/.test(p)) body = jobs.get(p.split('/').at(-1)) || {};
      else if (p === '/api/v1/notifications/settings') { if (req.method() === 'PUT') Object.assign(settings, req.postDataJSON()); body = settings; }
      else if (p === '/api/v1/notifications/smtp/test') body = { success: true, message: 'Server SMTP menerima email uji coba' };
      else if (p.includes('notifications')) body = { items: [], notifications: [], unread_count: 0 };
      else if (p.endsWith('/system/info')) body = { environment: 'production', agent_connected: true };
      await route.fulfill({ status: code, contentType: 'application/json', body: JSON.stringify(body) });
    });
    const root = () => page.getByTestId('email-page');
    const finish = async () => { for (const j of jobs.values()) if (j.status === 'queued') Object.assign(j, { status: 'succeeded', progress: 100 }); if (overview.mailboxes[0]) overview.mailboxes[0].state = 'active'; await root().getByRole('button', { name: 'Muat ulang email', exact: true }).click(); };
    await page.goto(`${base}/email`, { waitUntil: 'domcontentloaded' }); await root().getByRole('button', { name: 'Buat akun', exact: true }).waitFor();
    check(await page.getByRole('link', { name: 'Email', exact: true }).count() > 0, 'Email appears in the main navigation');
    await root().getByRole('button', { name: 'Buat akun', exact: true }).click(); await root().getByLabel('Nama akun', { exact: true }).fill('admin'); await root().getByLabel('Domain', { exact: true }).selectOption('1'); await root().getByRole('button', { name: 'Buat akun email', exact: true }).click();
    await root().getByRole('region', { name: 'Kredensial email baru' }).waitFor(); check(await root().getByText('GeneratedFixture123!', { exact: true }).isVisible(), 'Generated credentials are shown once');
    await page.evaluate(()=>Object.defineProperty(navigator,'clipboard',{value:undefined,configurable:true})); await root().getByRole('button',{name:'Salin password',exact:true}).click(); await page.getByText(/Fitur salin tidak tersedia/).waitFor(); check(true,'Unavailable clipboard gives a manual-copy fallback without throwing');
    await root().getByRole('button', { name: 'Reset password', exact: true }).waitFor(); check(await root().getByRole('button', { name: 'Reset password', exact: true }).isDisabled(), 'Pending account operations disable duplicate changes');
    check(await root().getByRole('link', { name: 'Lihat log tugas', exact: true }).isVisible(), 'Pending mailbox links to persisted job diagnostics');
    await root().getByRole('button', { name: 'Tutup kredensial' }).click(); await finish(); await root().getByText('Halo dari pelanggan', { exact: true }).click(); await root().getByText(/Pesan aman sebagai teks/).waitFor();
    check(await page.evaluate(() => !window.emailInjected), 'Email HTML is rendered as text and cannot execute scripts');
    check(requests.some(r => r.data.action === 'read' && r.data.uid_validity === 7 && r.data.uid === 42), 'Read operation includes UID validity');
    validity = 8; await page.waitForResponse(response => response.url().includes('/messages?folder=inbox') && response.request().method() === 'GET', { timeout: 20000 });
    await root().getByRole('button', { name: 'Pindahkan ke Sampah', exact: true }).click(); await root().getByText('Halo dari pelanggan', { exact: true }).waitFor(); check(requests.some(r => r.data.action === 'trash' && r.data.uid_validity === 7), 'Deletion retains opened-message UID validity after background folder refresh');
    await root().getByRole('button', { name: 'Tulis email', exact: true }).click(); await root().getByLabel('Kepada', { exact: true }).fill('person@example.net'); await root().getByLabel('Subjek', { exact: true }).fill('Balasan'); await root().getByLabel('Isi pesan', { exact: true }).fill('Isi pesan fixture'); await root().getByRole('button', { name: 'Kirim email', exact: true }).dblclick(); await root().getByLabel('Kepada', { exact: true }).waitFor({ state: 'hidden' });
    check(requests.filter(r => r.p.endsWith('/send')).length === 1, 'Rapid compose submission queues one message');
    check(/^[0-9a-f-]{36}$/.test(requests.find(r => r.p.endsWith('/send')).data.request_id), 'Compose carries a stable idempotency UUID');
    await root().getByRole('button', { name: 'Panduan DNS', exact: true }).click(); await root().getByRole('region', { name: 'Panduan DNS email' }).waitFor(); check(await root().getByText(/DNS only untuk hostname email/).isVisible(), 'DNS guide explains MX, DKIM and DNS-only mail routing');
    await root().getByRole('button', { name: 'Notifikasi panel', exact: true }).click(); await root().getByLabel('Nama panel dalam email').fill('Brand Sendiri'); await root().getByLabel('Subjek template').fill('[{panel_name}] {title}'); await root().getByRole('button', { name: 'Simpan template', exact: true }).click();
    check(await root().getByText('[Brand Sendiri] Sertifikat segera kedaluwarsa', { exact: true }).isVisible(), 'Custom notification template previews in the panel');
    await root().getByRole('button', { name: 'Atur SMTP dan pengirim', exact: true }).click(); await page.getByRole('dialog').waitFor(); check(await page.getByRole('dialog').getByRole('textbox').count() > 0, 'SMTP settings are reachable from Email'); await page.keyboard.press('Escape');
    await root().getByRole('button', { name: 'Server email', exact: true }).click(); check(await root().getByText('postfix', { exact: true }).isVisible() && await root().getByText('dovecot', { exact: true }).isVisible(), 'Server status separates transport and inbox services');
    await root().getByLabel('Hostname server email').fill('mail.example.com'); await root().getByLabel('Sertifikat TLS panel').selectOption('1'); await root().getByRole('button', { name: 'Terapkan konfigurasi server', exact: true }).click(); await root().getByRole('button', { name: 'Pemasangan sedang berjalan', exact: true }).waitFor();
    check(await root().getByRole('button', { name: 'Pemasangan sedang berjalan', exact: true }).isDisabled(), 'Server installation stays disabled while its job is active');
    await page.setViewportSize({ width: 390, height: 844 }); await root().getByRole('button', { name: 'Akun dan kotak masuk', exact: true }).click(); await root().getByRole('button', { name: /admin@example.com/ }).click(); await root().getByText('Halo dari pelanggan', { exact: true }).waitFor();
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Email page fits a narrow mobile viewport');
    await fs.mkdir(path.join(__dirname, '../artifacts/email'), { recursive: true }); await page.screenshot({ path: path.join(__dirname, '../artifacts/email/mobile.png'), fullPage: true });
    const contrast = async () => root().getByRole('button', { name: 'Akun dan kotak masuk', exact: true }).evaluate(async element => {
      void getComputedStyle(element).color; await Promise.all(element.getAnimations().map(a => a.finished));
      const style=getComputedStyle(element), luminance=value=>value.match(/[\d.]+/g).slice(0,3).map(Number).map(n=>{const c=n/255;return c<=0.04045?c/12.92:((c+0.055)/1.055)**2.4;}).reduce((sum,c,i)=>sum+c*[0.2126,0.7152,0.0722][i],0);
      const a=luminance(style.color),b=luminance(style.backgroundColor);return (Math.max(a,b)+0.05)/(Math.min(a,b)+0.05);
    });
    check(await contrast()>=4.5,'Light selected-tab text meets WCAG AA contrast');
    await page.evaluate(()=>document.documentElement.classList.add('dark'));check(await contrast()>=4.5,'Dark selected-tab text meets WCAG AA contrast');
    await root().getByRole('button',{name:'Tulis email',exact:true}).scrollIntoViewIfNeeded();await page.screenshot({path:path.join(__dirname,'../artifacts/email/mobile-inbox-dark.png')});
    check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'Dark inbox stays within the mobile viewport');
    await root().getByRole('button',{name:'Tulis email',exact:true}).focus();await page.keyboard.press('Enter');await root().getByLabel('Kepada',{exact:true}).waitFor();check(true,'Compose opens with keyboard activation');
    role = 'user'; await page.goto(`${base}/email`, { waitUntil: 'domcontentloaded' }); await root().getByRole('button', { name: 'Akun dan kotak masuk', exact: true }).waitFor(); check(await root().getByRole('button', { name: 'Server email', exact: true }).count() === 0 && await root().getByRole('button', { name: 'Notifikasi panel', exact: true }).count() === 0, 'Tenant cannot see administrator email settings');
    denied = true; await root().getByRole('button', { name: 'Muat ulang email', exact: true }).click(); await root().getByRole('alert').getByText(/belum tersedia/).waitFor(); check(true, 'API failures remain visible');
    check(errors.length === 0, `No browser runtime errors: ${errors.join(', ')}`);
    console.log(`${checks} email UI checks passed; mock APIs only.`);
  } finally { await browser.close(); }
})().catch(e => { console.error(e); process.exitCode = 1; });
