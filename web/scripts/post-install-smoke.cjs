/* Browser regression with intercepted API responses. No panel services are started. */
const {chromium}=require('playwright');
const assert=require('node:assert/strict');
const base=process.env.NUSA_UI_URL || 'http://127.0.0.1:5178';
let checks=0;
function check(value,message){assert.ok(value,message);checks++;}
const owner={id:42,username:'customer',linux_username:'p_actual',linux_uid:10042,status:'active',role:'user'};
const site={id:7,user_id:42,primary_domain:'example.test',kind:'php',runtime:'php',version:'8.3',docroot:'/home/p_actual/sites/example.test/public',status:'active',domains:[],force_https:false,spa_fallback:false};
async function fixture(browser,{authenticated=false,role='admin',installed=true}={}){
  const page=await browser.newPage();const calls=[];const errors=[];
  const mode={authenticated,role,installed,setupFail:true,metrics:'ready',expired:false,sshFail:false};
  page.on('pageerror',error=>errors.push(error.message));
  await page.route('**/api/v1/**',async route=>{
    const req=route.request(),url=new URL(req.url()),path=url.pathname;calls.push({path,url,method:req.method(),body:req.postDataJSON()});
    let body={},status=200;
    if(path==='/api/v1/auth/me'){status=mode.authenticated?200:401;body=mode.authenticated?{user:{id:role==='admin'?1:42,username:role==='admin'?'admin':'customer',role},csrf_token:'csrf'}:{error:'Unauthenticated'};}
    else if(path==='/api/v1/auth/csrf')body={csrf_token:'csrf'};
    else if(path==='/api/v1/auth/login'){mode.authenticated=true;body={user:{id:role==='admin'?1:42,username:'admin',role},csrf_token:'csrf'};}
    else if(path==='/api/v1/auth/setup/validate')body={valid:true};
    else if(path==='/api/v1/auth/setup'){if(mode.setupFail){status=500;body={title:'Internal Server Error',error:'Penyimpanan administrator gagal; token belum digunakan.'};mode.setupFail=false;}else {status=201;body={created:true,user_id:1,username:'admin'};}}
    else if(path==='/api/v1/tenants')body={tenants:[owner]};
    else if(path==='/api/v1/tenants/42')body={tenant:owner,plan:null};
    else if(path==='/api/v1/tenants/42/shell-policy')body={user_id:42,username:'customer',shell_policy:null,effective_policy:'full',inherited_from_plan:true,plan_policy:'full'};
    else if(path==='/api/v1/php/versions')body={versions:installed?[{version:'8.3',installed:true,fpm_binary_exists:true}]:[]};
    else if(path==='/api/v1/sites'&&req.method()==='POST')body={...site,...req.postDataJSON()};
    else if(path==='/api/v1/sites')body=[site];
    else if(path==='/api/v1/sites/7')body=site;
    else if(path.endsWith('/ssl'))body={certificate:null};
    else if(path==='/api/v1/ssh/connection-info'){if(mode.sshFail){status=400;body={error:'Identitas tenant tidak tersedia.'};}else body={host:'server.example.test',port:22,username:'p_actual',user_id:42,home_dir:'/home/p_actual',active_path:'/home/p_actual/sites/example.test/public',relative_path:'sites/example.test/public',shell_policy:'full',can_shell:true,can_sftp:true,has_ssh_keys:true,ssh_keys_count:1};}
    else if(path==='/api/v1/system/info')body={hostname:'fixture-server',os_name:'Debian',os_version:'12',uptime_secs:300,agent_version:'0.1.1'};
    else if(path==='/api/v1/system/metrics')body={state:mode.metrics,sampling_interval_secs:10,error:mode.metrics==='failed'?'Collector gagal membaca metrik.':null,sample:mode.metrics==='waiting'?null:{timestamp:Math.floor(Date.now()/1000),cpu_percent:37.5,cpu_cores:4,memory_used_bytes:1024**3,memory_total_bytes:4*1024**3,disk_used_bytes:10*1024**3,disk_total_bytes:100*1024**3,net_rx_bytes_sec:1024,net_tx_bytes_sec:2048,load1:0.5}};
    else if(path.includes('timeseries'))body={points:[]};
    else if(path==='/api/v1/files/list')body={entries:[],total_count:0,has_more:false};
    else if(path==='/api/v1/terminal/sessions'&&req.method()==='POST'){status=403;body={error:'Shell tenant belum diaktifkan.'};}
    else if(path.includes('notifications'))body={notifications:[],count:0,unread_count:0};
    else if(path.includes('jobs'))body={items:[],total:0};
    else if(path.includes('plans'))body={plans:[]};
    else {status=404;body={error:'Fixture endpoint unavailable'};}
    if(mode.expired && !path.startsWith('/api/v1/auth/')){status=401;body={error:'Session expired'};}
    await route.fulfill({status,contentType:'application/json',body:JSON.stringify(body)});
  });
  return {page,calls,mode,errors};
}
(async()=>{
  const browser=await chromium.launch({headless:true,executablePath:process.env.NUSA_BROWSER_PATH || 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe'});
  try {
    const first=await fixture(browser);const p=first.page;
    await p.goto(base+'/sites/7/settings');await p.locator('#login-username').waitFor();
    check(new URL(p.url()).searchParams.get('returnTo')==='/sites/7/settings','anonymous deep link preserves route');
    check(!first.calls.some(c=>c.path.endsWith('/csrf')),'anonymous bootstrap does not fetch CSRF');
    await p.locator('#login-username').fill('admin');await p.locator('#login-password').fill('secret');await p.locator('button[type=submit]').click();
    await p.getByText('Preferensi Konfigurasi Vhost',{exact:true}).waitFor({timeout:15000}).catch(async err=>{console.error({url:p.url(),errors:first.errors,calls:first.calls.map(c=>c.path),text:(await p.locator('body').innerText()).slice(0,1700)});throw err;});check(new URL(p.url()).pathname==='/sites/7/settings','login restores intended tab');
    await p.reload();await p.getByText('Preferensi Konfigurasi Vhost',{exact:true}).waitFor();checks++;
    await p.locator('a[href="/files"]').first().click();await p.locator('#execution-tenant').waitFor();
    check(!first.calls.some(c=>c.path==='/api/v1/files/list'),'admin must select tenant before reading files');
    await p.locator('#execution-tenant').selectOption('42');await p.waitForResponse(r=>r.url().includes('/files/list'));
    check(first.calls.filter(c=>c.path==='/api/v1/files/list').every(c=>c.url.searchParams.get('tenant')==='42'),'file requests carry selected tenant');
    await p.goBack();await p.getByText('Pilih tenant untuk membuka berkas atau terminalnya.',{exact:true}).waitFor();checks++;
    await p.goForward();await p.locator('#execution-tenant').waitFor();check(new URL(p.url()).searchParams.get('tenant')==='42','forward restores tenant');
    check(await p.locator('a[href="/terminal"]').count()>0,'navigation uses real links');
    await p.goto(base+'/terminal?tenant=42');await p.getByText('Tidak ada sesi terminal aktif saat ini.',{exact:true}).waitFor();
    await p.waitForTimeout(600);
    check(first.calls.filter(c=>c.path==='/api/v1/terminal/sessions'&&c.method==='POST').length===1,'failed terminal open does not retry infinitely');
    check(first.calls.find(c=>c.path==='/api/v1/terminal/sessions'&&c.method==='POST').body.tenant==='42','terminal carries selected tenant');
    await Promise.all([p.waitForResponse(r=>r.url().includes('/files/list')&&r.url().includes('sites')),p.goto(base+'/sites/7/files')]);
    const siteFiles=first.calls.filter(c=>c.path==='/api/v1/files/list').at(-1);
    check(siteFiles.url.searchParams.get('path')==='sites/example.test/public'&&siteFiles.url.searchParams.get('tenant')==='42','site files use owner and a path relative to the tenant jail');
    await p.getByRole('button',{name:'Salin SSH',exact:true}).click();await p.getByText('Salin Perintah SSH (Buka di Folder Aktif)',{exact:true}).waitFor();
    check((await p.getByRole('dialog').locator('code').first().innerText()).includes('p_actual@server.example.test'),'SSH commands use the returned tenant identity');
    check(first.calls.filter(c=>c.path==='/api/v1/ssh/connection-info').at(-1).url.searchParams.get('tenant')==='42','SSH info carries the selected tenant');
    await p.getByRole('button',{name:'Tutup dialog',exact:true}).click();first.mode.sshFail=true;
    await p.getByRole('button',{name:'Salin SSH',exact:true}).click();await p.getByRole('alert').filter({hasText:'Identitas tenant tidak tersedia.'}).waitFor();
    check(await p.getByRole('dialog').locator('code').count()===0,'SSH errors never fabricate connection commands');
    await p.getByRole('button',{name:'Tutup dialog',exact:true}).click();
    await p.goto(base+'/dashboard');await p.getByText('37,5%',{exact:true}).first().waitFor();checks++;
    first.mode.metrics='waiting';await p.getByRole('button',{name:/Segarkan|Refresh/i}).first().click();await p.waitForResponse(r=>r.url().endsWith('/system/metrics'));
    check(await p.getByText('37,5%',{exact:true}).count()===0,'waiting state clears previous values');
    first.mode.metrics='failed';await p.getByRole('button',{name:/Segarkan|Refresh/i}).first().click();await p.getByRole('alert').filter({hasText:'Collector gagal'}).waitFor();checks++;
    await p.getByRole('button',{name:'Situs baru',exact:true}).click();await p.getByRole('button',{name:/PHP/}).first().click();
    await p.waitForFunction(() => [...document.querySelectorAll('[role="dialog"] button')].some(button => button.textContent.trim() === 'Lanjutkan' && !button.disabled));
    check(await p.getByRole('dialog').getByText(/\d+ versi PHP terpasang\./).count()===0,'ready PHP inventory omits redundant version count');
    await p.getByRole('button',{name:/Lanjut/}).last().click();await p.getByLabel('Versi PHP').waitFor();
    check(await p.getByLabel('Versi PHP').inputValue()==='8.3','PHP uses installed versions');
    await p.getByLabel('Preset PHP').selectOption('laravel');await p.locator('#step2-form input[type=text]').first().fill('new.example.test');
    await p.locator('#step2-form').evaluate(f=>f.requestSubmit());await p.getByRole('button',{name:/customer/}).click();
    await p.getByRole('button',{name:/Lanjut/}).last().click();
    await Promise.all([p.waitForResponse(r=>r.url().endsWith('/sites')&&r.request().method()==='POST'), p.getByRole('button',{name:/Terapkan.*Buat/}).click()]);
    const created=first.calls.find(c=>c.path==='/api/v1/sites'&&c.method==='POST').body;
    check(created.kind==='php'&&created.php_version==='8.3'&&created.preset==='laravel'&&created.user_id===42,'PHP provisioning request includes runtime preset and tenant');
    await p.setViewportSize({width:390,height:844});await p.goto(base+'/dashboard');await p.locator('.mobile-navigation').waitFor();
    check(await p.locator('.mobile-navigation a').first().evaluate(a=>getComputedStyle(a).display)==='flex','mobile navigation links keep their layout');
    check(await p.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'mobile dashboard has no horizontal overflow');
    first.mode.expired=true;await p.getByRole('button',{name:/Segarkan|Refresh/i}).first().click();await p.locator('#login-username').waitFor();
    check(new URL(p.url()).pathname==='/login','expired protected requests clear session and return to login');
    check(first.errors.length===0,'authenticated pages have no browser exceptions');await p.close();

    const setup=await fixture(browser);await setup.page.goto(base+'/setup');await setup.page.locator('#setup-token').fill('setup-secret');await setup.page.locator('button[type=submit]').click();
    await setup.page.locator('#setup-password').fill('StrongSecret123!');await setup.page.locator('#setup-confirm-password').fill('StrongSecret123!');await setup.page.locator('button[type=submit]').click();
    await setup.page.getByRole('alert').filter({hasText:'Penyimpanan administrator gagal'}).waitFor();
    check(await setup.page.locator('#setup-password').count()===1,'failed setup remains on administrator form');
    await setup.page.locator('button[type=submit]').click();await setup.page.getByRole('button',{name:'Masuk ke Dashboard'}).waitFor();
    check(setup.calls.filter(c=>c.path==='/api/v1/auth/setup').every(c=>c.body.token==='setup-secret'),'setup retry preserves token');
    check(!setup.calls.some(c=>c.path.endsWith('/csrf')),'setup does not request anonymous session CSRF');await setup.page.close();

    const noPhp=await fixture(browser,{authenticated:true,installed:false});await noPhp.page.goto(base+'/dashboard');await noPhp.page.getByRole('button',{name:'Situs baru',exact:true}).click();await noPhp.page.getByRole('button',{name:/PHP/}).first().click();
    await noPhp.page.getByRole('link',{name:'Buka Runtime Manager'}).waitFor();await noPhp.page.getByRole('button',{name:/Lanjut/}).last().click();
    check(await noPhp.page.locator('#step2-form').count()===0,'missing PHP cannot proceed');await noPhp.page.close();
    const tenant=await fixture(browser,{authenticated:true,role:'user'});await tenant.page.goto(base+'/runtime');await tenant.page.getByRole('alert').filter({hasText:'Anda tidak memiliki akses'}).waitFor();checks++;
    check(!tenant.calls.some(c=>c.path.includes('/php/versions')),'forbidden page does not fetch runtime data');await tenant.page.close();
    console.log(`${checks} post-install browser checks passed`);
  } finally {await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
