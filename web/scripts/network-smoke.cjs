/* Production UI with intercepted API fixtures. No live DNS, Cloudflare, or daemons. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.QPANEL_UI_URL || 'http://127.0.0.1:5194';
const artifacts = path.join(__dirname, '../artifacts/network');
let checks = 0;
const check = (value, label) => { assert.ok(value, label); console.log(`ok ${++checks}: ${label}`); };
const account = 'a'.repeat(32), zone = 'b'.repeat(32), tunnel = '01234567-89ab-4cde-8fab-0123456789ab';
const token = 'fixture_cloudflare_api_token_123456789';
const job = (id, name, payload) => ({ id, name, status:'queued', progress:0, user_id:1, payload_json:JSON.stringify(payload), result_json:null, error_message:null, retry_count:0, max_retries:0, created_at:1, updated_at:1 });
(async () => {
  await fs.mkdir(artifacts, { recursive:true });
  const browser = await chromium.launch({ headless:true, channel:process.env.QPANEL_BROWSER_CHANNEL || 'msedge' });
  const errors = [], commands = [], jobs = new Map();
  const network = { ipv4:null, ipv6:null, projects:[{domain_id:1,site_id:1,hostname:'app.example.com',kind:'static',status:'active',tls:false}] };
  const status = { configured:false,account_id:null,connector_installed:false,managed_tunnels:[] };
  let records=[], tunnels=[], zonePage=1, denyRecords=false;
  const zoneItem = { id:zone,name:'example.com',status:'active',name_servers:['one.ns.cloudflare.com','two.ns.cloudflare.com'] };
  try {
    const page=await browser.newPage({ viewport:{width:1440,height:1000} });page.setDefaultTimeout(15000);
    page.on('pageerror', error=>errors.push(error.message));
    await page.route('**/api/v1/**', async route=>{
      const req=route.request(), url=new URL(req.url()), pathname=url.pathname;
      let code=200, body={};
      if(pathname==='/api/v1/auth/me') body={user:{id:1,username:'admin',role:'admin'},csrf_token:'fixture'};
      else if(pathname==='/api/v1/auth/csrf') body={csrf_token:'fixture'};
      else if(pathname==='/api/v1/network') {
        if(req.method()==='PUT') Object.assign(network,req.postDataJSON());
        body=network;
      } else if(pathname==='/api/v1/network/dns-check') body={hostname:'app.example.com',addresses:['203.0.113.10'],expected:['203.0.113.10'],matches:true};
      else if(pathname==='/api/v1/network/cloudflare/install') {body=job(`job-${jobs.size}`,'cloudflare.install',{});jobs.set(body.id,body);code=202;}
      else if(pathname==='/api/v1/network/cloudflare' && req.method()==='GET') body=status;
      else if(pathname==='/api/v1/network/cloudflare') {
        const cmd=req.postDataJSON();commands.push(cmd);
        switch(cmd.action){
          case 'configure': status.configured=true;status.account_id=cmd.account_id;body={configured:true};break;
          case 'zones': zonePage=cmd.page;body={items:[zoneItem],pagination:{page:zonePage,total_pages:2,total_count:2}};break;
          case 'create_zone': body={...zoneItem,id:'d'.repeat(32),name:cmd.name,status:'pending'};break;
          case 'records':
            if(denyRecords){code=502;body={error:'Fixture: token tidak memiliki DNS Read'};}
            else body={items:records.map(r=>({...r})),pagination:{page:1,total_pages:1,total_count:records.length}};
            break;
          case 'save_record': {
            if(cmd.record.content==='reject-fixture'){code=502;body={error:'Fixture: record ditolak'};break;}
            const value={...cmd.record,id:cmd.record_id || 'c'.repeat(32),type:cmd.record.kind};
            records=records.filter(r=>r.id!==value.id).concat(value);body=value;break;
          }
          case 'delete_record': records=records.filter(r=>r.id!==cmd.record_id);body={deleted:true};break;
          case 'tunnels':body={items:tunnels,pagination:{page:1,total_pages:1,total_count:tunnels.length}};break;
          default:body=job(`job-${jobs.size}`,'cloudflare.change',cmd);jobs.set(body.id,body);code=202;
        }
      } else if(pathname==='/api/v1/jobs') {
        const filter=url.searchParams.get('status');const items=[...jobs.values()].filter(j=>!filter || j.status===filter).map(j=>({...j}));
        body={items,total:items.length,page:1,per_page:100,total_pages:1};
      } else if(/^\/api\/v1\/jobs\/[^/]+$/.test(pathname)) body=jobs.get(pathname.split('/').at(-1)) || {};
      else if(pathname.includes('notifications')) body={notifications:[],items:[],unread_count:0};
      else if(pathname.includes('optimizer')) { code=503; body={error:'Optimizer outside this fixture'}; }
      else if(pathname.endsWith('/system/info')) body={environment:'production',agent_connected:true};
      await route.fulfill({status:code,contentType:'application/json',body:JSON.stringify(body)});
    });
    const root=()=>page.getByTestId('network-settings');
    const open=async()=>{await page.goto(`${base}/settings`,{waitUntil:'domcontentloaded'});await page.getByRole('button',{name:'DNS dan Cloudflare',exact:true}).click();await root().waitFor();};
    const finish=async(update)=>{const active=[...jobs.values()].filter(j=>j.status==='queued');assert.equal(active.length,1);update?.();Object.assign(active[0],{status:'succeeded',progress:100,updated_at:2});await root().getByText(/Dalam antrean|Sedang berjalan|Memverifikasi status/).waitFor({state:'hidden'});};
    await open();await root().getByLabel('IPv4 publik',{exact:true}).fill('203.0.113.10');
    await root().getByRole('button',{name:'Simpan IP publik'}).click();await root().getByText('IP publik disimpan.',{exact:true}).waitFor();
    check(await root().getByText('203.0.113.10',{exact:true}).count()>0,'public IPv4 persists and produces project A guidance');
    await root().getByRole('button',{name:'Periksa DNS'}).click();await root().getByText(/Sesuai dengan IP server/).waitFor();check(true,'DNS check renders resolver result for the selected project');
    await root().getByRole('button',{name:'Cloudflare DNS',exact:true}).click();
    await root().getByLabel('Account ID',{exact:true}).fill(account);await root().getByLabel('API token',{exact:true}).fill(token);
    await root().getByRole('button',{name:'Hubungkan akun',exact:true}).click();await root().getByText(/Akun Cloudflare tersambung/).waitFor();
    check(await root().getByLabel('API token baru').inputValue()==='','token input clears after save');
    check(!(await page.evaluate(()=>JSON.stringify(localStorage))).includes(token),'Cloudflare token is not stored in localStorage');
    await root().getByLabel('Pilih domain',{exact:true}).selectOption(zone);await root().getByRole('heading',{name:'Tambah record',exact:true}).waitFor();
    check(await root().getByText(/one.ns.cloudflare.com/).count()>0,'nameservers and domain status are shown');
    const kind=root().getByLabel('Jenis record',{exact:true}), content=root().getByLabel('Isi record',{exact:true});
    await kind.selectOption('TXT');await root().getByLabel('Nama lengkap atau @').fill('_dmarc.example.com');await content.fill('v=DMARC1; p=none');
    await root().getByRole('button',{name:'Simpan record',exact:true}).click();await root().getByText(/Record disimpan/).waitFor();
    check(commands.some(c=>c.action==='save_record' && c.record.kind==='TXT' && c.record.proxied===false),'TXT with underscore sends a non-proxied typed record');
    await root().getByRole('button',{name:'Ubah _dmarc.example.com',exact:true}).click();await content.fill('reject-fixture');await root().getByRole('button',{name:'Simpan record',exact:true}).click();await root().getByRole('alert').getByText(/record ditolak/).waitFor();
    check(await content.inputValue()==='reject-fixture','failed DNS save retains the draft');
    await content.fill('v=DMARC1; p=reject');await root().getByRole('button',{name:'Simpan record',exact:true}).click();await root().getByText('v=DMARC1; p=reject',{exact:true}).waitFor();check(records.length===1,'editing updates the existing record without duplication');
    const beforeDelete=commands.filter(c=>c.action==='delete_record').length;await root().getByRole('button',{name:'Hapus _dmarc.example.com',exact:true}).click();
    check(commands.filter(c=>c.action==='delete_record').length===beforeDelete,'deleting a DNS record first requires confirmation');
    await root().getByRole('button',{name:'Ya, hapus record'}).click();await root().getByText('Record dihapus.',{exact:true}).waitFor();check(records.length===0,'confirmed deletion removes the record');
    await root().getByRole('button',{name:'Cloudflare Tunnel',exact:true}).click();await root().getByRole('button',{name:'Pasang cloudflared',exact:true}).click();await root().getByText(/Pemasangan cloudflared/).waitFor();
    check(await root().getByRole('button',{name:'Pasang cloudflared',exact:true}).isDisabled(),'accepted installation immediately locks its button');
    await open();await root().getByRole('button',{name:'Cloudflare Tunnel',exact:true}).click();await root().getByText(/Pemasangan cloudflared/).waitFor();check(true,'pending installation is restored after full navigation');
    await finish(()=>{status.connector_installed=true;});await root().getByText('Konektor server · Terpasang',{exact:true}).waitFor();check(true,'connector inventory refreshes after job completion');
    await root().getByText('Operasi Cloudflare selesai. Status server telah diperbarui.',{exact:true}).waitFor();check(true,'completed jobs replace the queued status message');
    await root().getByLabel('Nama tunnel baru').fill('project-server');await root().getByRole('button',{name:'Buat tunnel',exact:true}).click();await root().getByText(/Perubahan tunnel/).waitFor();
    check(await root().getByRole('button',{name:'Buat tunnel',exact:true}).isDisabled(),'creating a tunnel blocks duplicate submissions');
    await finish(()=>{tunnels=[{id:tunnel,name:'project-server',status:'inactive',managed:true,remote_config:true}];status.managed_tunnels=[{id:tunnel,local_status:'inactive',routes:[]}];});
    await root().getByRole('heading',{name:'project-server',exact:true}).waitFor();await root().getByRole('button',{name:'Mulai project-server',exact:true}).click();await root().getByText(/Perubahan tunnel/).waitFor();
    await finish(()=>{status.managed_tunnels[0].local_status='active';tunnels[0].status='healthy';});
    await root().getByText('Cloudflare: healthy · Lokal: active',{exact:true}).waitFor();check(true,'remote health and local service status are separate');
    tunnels[0].status='degraded';await root().getByText('Cloudflare: degraded · Lokal: active',{exact:true}).waitFor();check(true,'tunnel health refreshes automatically without clicking reload');
    await root().getByLabel('Pilih domain',{exact:true}).selectOption(zone);await root().getByLabel('Tunnel tujuan').selectOption(tunnel);await root().getByLabel('Hostname project').selectOption('app.example.com');
    await root().getByRole('button',{name:'Terbitkan project',exact:true}).click();await root().getByText(/Perubahan tunnel/).waitFor();
    check(commands.some(c=>c.action==='publish' && c.hostname==='app.example.com' && !('service' in c)),'publication selects an existing project and accepts no arbitrary origin URL');
    await finish(()=>{status.managed_tunnels[0].routes=[{hostname:'app.example.com',zone_id:zone}];});await root().getByRole('link',{name:'app.example.com',exact:true}).waitFor();
    check(await root().getByRole('button',{name:'Hapus tunnel project-server',exact:true}).isDisabled(),'tunnel deletion is blocked while project routes exist');
    await page.locator('main').evaluate(el=>{el.scrollTop=0;});
    await page.screenshot({path:path.join(artifacts,'desktop-light.png'),fullPage:true});
    await page.setViewportSize({width:390,height:844});await page.evaluate(()=>document.documentElement.classList.add('dark'));
    await root().getByRole('button',{name:'Cloudflare Tunnel',exact:true}).evaluate(async element=>{
      void getComputedStyle(element).color;
      await Promise.all(element.getAnimations().map(animation=>animation.finished));
    });
    const contrast = await root().getByRole('button',{name:'Cloudflare Tunnel',exact:true}).evaluate(element=>{
      const css=getComputedStyle(element);
      const luminance=value=>value.match(/[\d.]+/g).slice(0,3).map(Number).map(channel=>{const c=channel/255;return c<=0.04045?c/12.92:((c+0.055)/1.055)**2.4;}).reduce((sum,c,index)=>sum+c*[0.2126,0.7152,0.0722][index],0);
      const a=luminance(css.color),b=luminance(css.backgroundColor);return {ratio:(Math.max(a,b)+0.05)/(Math.min(a,b)+0.05),color:css.color,background:css.backgroundColor};
    });
    check(contrast.ratio>=4.5,`dark primary button text meets WCAG AA contrast ${JSON.stringify(contrast)}`);
    await page.locator('main').evaluate(el=>{el.scrollTop=0;});
    await page.screenshot({path:path.join(artifacts,'mobile-dark.png'),fullPage:true});
    await root().getByRole('heading',{name:'project-server',exact:true}).scrollIntoViewIfNeeded();
    check((await root().getByLabel('Nama tunnel baru').boundingBox()).width>250,'mobile tunnel name retains a usable full-width field');
    await page.screenshot({path:path.join(artifacts,'mobile-tunnel-dark.png')});
    check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'mobile tunnel settings do not overflow horizontally');
    await root().getByRole('button',{name:'Lepaskan hostname app.example.com',exact:true}).click();await root().getByText(/Perubahan tunnel/).waitFor();await finish(()=>{status.managed_tunnels[0].routes=[];});
    await root().getByRole('button',{name:'Hapus tunnel project-server',exact:true}).click();check(await root().getByRole('button',{name:'Hapus permanen',exact:true}).isDisabled(),'tunnel deletion requires typing the tunnel name');
    await root().getByLabel('Konfirmasi nama tunnel').fill('project-server');await root().getByRole('button',{name:'Hapus permanen',exact:true}).click();await root().getByText(/Perubahan tunnel/).waitFor();await finish(()=>{tunnels=[];status.managed_tunnels=[];});check(true,'confirmed tunnel removal completes through a persistent job');
    await root().getByRole('button',{name:'Cloudflare DNS',exact:true}).click();await root().getByLabel('Pilih domain',{exact:true}).selectOption(zone);await root().getByRole('heading',{name:'Tambah record',exact:true}).waitFor();
    denyRecords=true;await root().getByRole('button',{name:'Muat ulang record'}).click();await root().getByRole('alert').getByText(/DNS Read/).waitFor();check(true,'provider permission errors remain visible');
    denyRecords=false;await root().getByRole('button',{name:'Muat ulang record'}).click();await root().getByRole('heading',{name:'Tambah record',exact:true}).waitFor();
    const domainSection=root().getByRole('region',{name:'Domain Cloudflare'});await domainSection.getByRole('button',{name:'Berikutnya',exact:true}).click();await domainSection.getByText('Halaman 2 dari 2',{exact:true}).waitFor();check(zonePage===2,'Cloudflare zones support pagination');
    check(errors.length===0,`no runtime errors: ${errors.join(', ')}`);
    console.log(`${checks} network UI checks passed; fixture APIs only.`);
  } finally {await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
