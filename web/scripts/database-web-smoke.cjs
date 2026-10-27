/* Browser/API fixtures only. Does not start PHP, panel daemons or databases. */
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const base = process.env.NUSA_UI_URL || 'http://127.0.0.1:5180';
let checks = 0;
const check = (value, message) => { assert.ok(value, message); console.log(`ok ${++checks}: ${message}`); };
(async () => {
  const browser = await chromium.launch({ headless: true, channel: process.env.NUSA_BROWSER_CHANNEL || undefined });
  try {
    const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
    const page = await context.newPage(); page.setDefaultTimeout(20000);
    const errors = [], calls = [];
    const mode = { inventoryFail: false, ready: false, accepted: false, phase: 'queued', launchFail: false, maliciousUrl: false };
    context.on('page', p => p.on('pageerror', error => errors.push(error.message)));
    page.on('pageerror', error => errors.push(error.message));
    const job = () => ({ id:'db-web-job', user_id:1, name:'database.web.install', status:mode.phase, progress:mode.phase==='running'?55:mode.phase==='succeeded'?100:0, payload_json:'{}', result_json:null, error_message:null, retry_count:0,max_retries:1,created_at:1,updated_at:1 });
    const site = { id:1,user_id:2,kind:'static',primary_domain:'shop.test',docroot:'/home/p_user1/web/shop/public',status:'active',spa_fallback:false,force_https:false,domains:[],created_at:1,updated_at:1 };
    await context.route('**/api/v1/**', async route => {
      const request=route.request(), url=new URL(request.url()), name=url.pathname, method=request.method(); calls.push({name,method});
      if (/\/databases\/[12]\/web\/$/.test(name)) {
        await route.fulfill({contentType:'text/html',body:`<!doctype html><h1>${name.includes('/1/')?'phpMyAdmin':'Adminer'} fixture</h1>`}); return;
      }
      let body={}, status=200;
      if (name==='/api/v1/auth/me') body={user:{id:1,username:'admin',role:'admin'},csrf_token:'fixture'};
      else if (name==='/api/v1/auth/csrf') body={csrf_token:'fixture'};
      else if (name==='/api/v1/runtimes/catalog') body={enabled:true,interval_hours:24,sources:[]};
      else if (name==='/api/v1/php/versions') body={ppa_configured:true,versions:[]};
      else if (name==='/api/v1/databases/servers') body=[];
      else if (name==='/api/v1/databases/web-tools') {
        if (mode.inventoryFail) {status=503;body={error:'Status pengelola sementara tidak tersedia'};}
        else body={installed:mode.ready,ready:mode.ready};
      } else if (name==='/api/v1/databases/web-tools/install') {mode.accepted=true; status=202;body=job();}
      else if (name==='/api/v1/jobs') {const items=mode.accepted&&url.searchParams.get('status')===mode.phase?[job()]:[];body={items,total:items.length,total_pages:1,page:1,per_page:100};}
      else if (name==='/api/v1/jobs/db-web-job') body=job();
      else if (name==='/api/v1/sites') body=[site];
      else if (name==='/api/v1/sites/1') body=site;
      else if (name==='/api/v1/sites/1/ssl') body={has_ssl:false,certificate:null};
      else if (name==='/api/v1/databases') body={total:2,plan_max_databases:5,used_databases:2,databases:[1,2].map(id=>({id,user_id:2,site_id:1,engine:id===1?'mariadb':'postgresql',name:`p_user1_${id===1?'shop':'app'}`,db_user:`p_user1_db${id}`,remote_access:false,allowed_hosts:'localhost',collation:'utf8mb4_unicode_ci',character_set:'utf8mb4',size_bytes:1024,created_at:1,updated_at:1}))};
      else if (/\/databases\/[12]\/web-access$/.test(name)) {
        if(mode.launchFail){status=503;body={error:'Pengelola database belum siap. Minta administrator memasangnya.'};}
        else {const id=name.split('/')[4];body={url:mode.maliciousUrl?'https://evil.test/':`/api/v1/databases/${id}/web/`,tool:id==='1'?'phpMyAdmin':'Adminer'};}
      } else if (name.includes('unread-count')) body={count:0,unread_count:0};
      else if (name.includes('notifications')) body={notifications:[],total:0};
      await route.fulfill({status,contentType:'application/json',body:JSON.stringify(body)});
    });
    await page.goto(base+'/runtime',{waitUntil:'domcontentloaded'});
    await page.getByRole('button',{name:'Server Basis Data',exact:true}).click();
    check(await page.getByRole('heading',{name:'Server basis data',exact:true}).isVisible(),'database tab has its own server and web access heading');
    const install=page.getByRole('button',{name:'Pasang pengelola database',exact:true});
    await install.waitFor(); await install.click();
    await page.getByRole('progressbar',{name:'Progres tugas Pengelola database'}).waitFor();
    check(mode.accepted,'administrator installation is queued');
    check(await page.getByRole('button',{name:'Sedang dipasang',exact:true}).isDisabled(),'duplicate installation is blocked');
    await page.getByRole('button',{name:'Segarkan',exact:true}).click();
    await page.getByRole('progressbar',{name:'Progres tugas Pengelola database'}).waitFor();
    check(calls.filter(c=>c.name==='/api/v1/databases/web-tools/install').length===1,'page refresh recovers the active job without reinstallation');
    mode.phase='running';
    await page.goto(base+'/sites/shop.test/database',{waitUntil:'domcontentloaded'});
    const mysql=page.getByRole('button',{name:'Buka phpMyAdmin untuk p_user1_shop di tab baru'});
    const postgres=page.getByRole('button',{name:'Buka Adminer untuk p_user1_app di tab baru'});
    await mysql.waitFor(); check(await postgres.isVisible(),'database rows offer the appropriate tool for both engines');
    let popupPromise=page.waitForEvent('popup'); await mysql.click(); let popup=await popupPromise; await popup.waitForURL('**/api/v1/databases/1/web/');
    check(await popup.getByRole('heading',{name:'phpMyAdmin fixture'}).isVisible(),'MySQL opens its internal manager in a new tab');
    check(await popup.evaluate(()=>window.opener===null),'new tab cannot control the panel opener'); await popup.close();
    popupPromise=page.waitForEvent('popup'); await postgres.click(); popup=await popupPromise; await popup.waitForURL('**/api/v1/databases/2/web/');
    check(await popup.getByRole('heading',{name:'Adminer fixture'}).isVisible(),'PostgreSQL opens Adminer in a new tab');await popup.close();
    await page.evaluate(()=>{window.__databaseOpen=window.open;window.open=()=>null;});
    const previous=calls.filter(c=>c.name.endsWith('/web-access')).length;await mysql.click();
    await page.getByRole('alert').filter({hasText:'Izinkan tab baru'}).waitFor();
    check(calls.filter(c=>c.name.endsWith('/web-access')).length===previous,'blocked popups do not create an access request');
    await page.evaluate(()=>{window.open=window.__databaseOpen;});
    mode.launchFail=true; await mysql.click(); await page.getByRole('alert').filter({hasText:'Pengelola database belum siap'}).waitFor();
    check(context.pages().length===1,'failed launch closes its temporary tab and shows an actionable error');
    mode.launchFail=false;mode.maliciousUrl=true;await mysql.click();await page.getByRole('alert').filter({hasText:'Alamat pengelola database tidak valid'}).waitFor();
    check(context.pages().length===1,'external or unexpected launch URLs are rejected'); mode.maliciousUrl=false;
    await page.goto(base+'/runtime',{waitUntil:'domcontentloaded'});await page.getByRole('button',{name:'Server Basis Data',exact:true}).click();
    const progress=page.getByRole('progressbar',{name:'Progres tugas Pengelola database'});await progress.waitFor();
    check(await progress.getAttribute('value')==='55','installation progress is recovered after navigation');
    mode.phase='succeeded';mode.inventoryFail=true;await page.getByRole('alert').filter({hasText:'Status pengelola sementara tidak tersedia'}).waitFor();
    check(await progress.count()===1,'successful install remains tracked until inventory can confirm readiness');
    mode.inventoryFail=false;mode.ready=true;await page.getByRole('button',{name:'Perbarui pengelola',exact:true}).waitFor();
    await progress.waitFor({state:'detached'});check(calls.filter(c=>c.name==='/api/v1/databases/web-tools/install').length===1,'navigation and refresh do not enqueue a second install');
    check(!await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),'database tools card fits a 390px viewport');
    check(errors.length===0,`no browser exceptions (${errors.join(', ')})`);
    await fs.mkdir(path.join(__dirname,'../artifacts/database-web'),{recursive:true});
    await page.screenshot({path:path.join(__dirname,'../artifacts/database-web/runtime-390.png'),fullPage:true});
    console.log(`${checks} database web browser checks passed (API fixtures only).`);
  } finally {await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
