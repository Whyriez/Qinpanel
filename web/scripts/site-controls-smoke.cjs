/* Built UI + intercepted HTTP fixtures only; never runs agent or host services. */
const {chromium}=require('playwright');
const assert=require('node:assert/strict');const fs=require('node:fs/promises');const path=require('node:path');
const base='http://127.0.0.1:5195',output=path.join(__dirname,'../artifacts/site-controls');
const site={id:42,user_id:2,primary_domain:'example.test',kind:'nodejs',version:'22',docroot:'/home/p_alice/sites/example.test/public',status:'active',spa_fallback:false,force_https:false,created_at:1770000000,updated_at:1770000000,domains:[]};
const initial={site_id:42,domain:'example.test',status:{webserver_status:'active',runtime_status:'running',healthy:true,uptime_secs:60,pid:123},resources:{cpu_usage_usec:0,cpu_percent:0,memory_bytes:0,memory_limit_bytes:0,pids_current:1,pids_limit:0},ssl:{has_certificate:false,issuer:null,valid_until:null,days_remaining:null,auto_renew:false,status:'none',force_https:false},runtime:{kind:'nodejs',version:'22',entrypoint_or_target:site.docroot},storage:{docroot_bytes:0,databases_bytes:0,backups_bytes:0,total_site_bytes:0,plan_disk_limit_bytes:0,usage_percentage:0},quick_actions_available:['start','stop','restart','reload','clear_cache'],warnings:[],latest_deployment:null,latest_backup:null};
let checks=0;function check(condition,message){assert.ok(condition,message);console.log(`ok ${++checks}: ${message}`);}
(async()=>{await fs.mkdir(output,{recursive:true});const browser=await chromium.launch({headless:true,channel:'chrome'});try{
 const page=await browser.newPage({viewport:{width:1440,height:1100},reducedMotion:'reduce'});page.setDefaultTimeout(15000);
 let data=structuredClone(initial),failGet=false,failAction=false,negativeAction=false,delayAction=null,overviewReads=0;const actions=[],errors=[],calls=[];
 page.on('pageerror',e=>errors.push(e.message));await page.addInitScript(()=>{if(!localStorage.getItem('nusa_theme'))localStorage.setItem('nusa_theme','light');});
 await page.route('**/api/v1/**',async route=>{const req=route.request(),url=new URL(req.url());let body={},status=200;calls.push(url.pathname);
  if(url.pathname==='/api/v1/auth/me')body={user:{id:2,username:'alice',role:'user'},csrf_token:'fixture'};
  else if(url.pathname==='/api/v1/auth/csrf')body={csrf_token:'fixture'};
  else if(url.pathname==='/api/v1/sites')body=[{...site,kind:data.runtime.kind}];
  else if(url.pathname==='/api/v1/sites/42')body={...site,kind:data.runtime.kind};
  else if(url.pathname==='/api/v1/sites/42/ssl')body={site_id:42,has_ssl:false,certificate:null};
  else if(url.pathname==='/api/v1/sites/42/overview'){overviewReads++;status=failGet?503:200;body=failGet?{error:'Fixture status unavailable'}:data;}
  else if(url.pathname==='/api/v1/sites/42/action'){
   const action=req.postDataJSON().action;actions.push(action);if(delayAction)await delayAction;
   if(failAction){status=502;body={error:'Fixture action failed'};}else if(negativeAction)body={success:false,action,message:'Fixture action not confirmed'};
   else {if(action==='stop')data={...data,status:{...data.status,webserver_status:'suspended',runtime_status:'inactive',healthy:false}};if(action==='start')data={...data,status:{...data.status,webserver_status:'active',runtime_status:'running',healthy:true}};body={success:true,action,message:'Fixture action acknowledged'};}
  }else if(url.pathname==='/api/v1/jobs')body={items:[],total:0};
  else if(url.pathname.includes('notifications'))body={notifications:[],count:0,unread_count:0,total:0};
  else if(url.pathname==='/api/v1/settings/feature-flags')body={};
  await route.fulfill({status,contentType:'application/json',body:JSON.stringify(body)});
 });
 await page.goto(`${base}/sites/42/overview`);const controls=page.locator('.project-service-actions');await controls.getByRole('button',{name:'Hentikan',exact:true}).waitFor().catch(async error=>{console.log(JSON.stringify({url:page.url(),text:await page.locator('body').innerText(),calls,errors}));await page.screenshot({path:path.join(output,'failure.png')});throw error;});
 check(await controls.getByRole('button',{name:'Mulai',exact:true}).isDisabled(),'running app disables redundant start');
 check((await controls.innerText()).includes('Proses aplikasi: Berjalan'),'runtime state is visible beside controls');
 check(await controls.getByRole('button',{name:'Bersihkan cache',exact:true}).count()===0,'unsupported cache action is hidden for old API payloads');
 let release;delayAction=new Promise(resolve=>{release=resolve;});await controls.getByRole('button',{name:'Hentikan',exact:true}).click();await controls.getByText('Menghentikan aplikasi... Menunggu hasil dan pemeriksaan status.').waitFor();
 check(await controls.getByRole('button',{name:'Mulai',exact:true}).isDisabled(),'all controls are locked while stop is pending');
 check((await controls.innerText()).includes('Proses aplikasi: Berjalan'),'pending request does not optimistically report stopped');
 release();delayAction=null;await controls.getByText('Berhenti',{exact:true}).waitFor();
 check(await controls.getByRole('button',{name:'Hentikan',exact:true}).isDisabled(),'successful stop is reread and stop is disabled');
 check(await controls.getByRole('button',{name:'Mulai ulang aplikasi',exact:true}).isDisabled(),'stopped app cannot be restarted accidentally');
 await page.reload();await controls.getByText('Berhenti',{exact:true}).waitFor();check(await controls.getByRole('button',{name:'Mulai',exact:true}).isEnabled(),'stopped state survives reload and can be started');
 await controls.getByRole('button',{name:'Mulai',exact:true}).click();await controls.getByText('Berjalan',{exact:true}).waitFor();
 failAction=true;await controls.getByRole('button',{name:'Mulai ulang aplikasi',exact:true}).click();await page.getByRole('alert').filter({hasText:'Fixture action failed'}).waitFor();check((await controls.innerText()).includes('Berjalan'),'failed restart retains observed state and reports failure');failAction=false;
 negativeAction=true;await controls.getByRole('button',{name:'Mulai ulang aplikasi',exact:true}).click();await page.getByRole('alert').filter({hasText:'Fixture action not confirmed'}).waitFor();check(true,'HTTP success with negative agent acknowledgement is treated as failure');negativeAction=false;
 failGet=true;await page.getByRole('button',{name:'Segarkan ringkasan',exact:true}).click();await controls.getByText(/Status terakhir belum dapat diperbarui/).waitFor();check(await controls.getByRole('button',{name:'Hentikan',exact:true}).isDisabled(),'stale status disables mutations instead of claiming current truth');failGet=false;
 data.status.runtime_status='unknown';data.status.healthy=false;await page.getByRole('button',{name:'Segarkan ringkasan',exact:true}).click();await controls.getByText('Belum diketahui',{exact:true}).waitFor();check(!((await controls.innerText()).includes('Berhenti')),'unavailable process is unknown rather than stopped');
 data.status.runtime_status='running';data.status.healthy=true;const before=overviewReads;await controls.getByText('Berjalan',{exact:true}).waitFor({timeout:10000});check(overviewReads>before,'automatic polling reflects external process changes');
 for(const kind of ['php','static','proxy']){data={...structuredClone(initial),runtime:{...initial.runtime,kind},status:{...initial.status,runtime_status:kind==='php'?'running':'not_applicable'}};await page.reload();await controls.getByRole('button',{name:'Nonaktifkan situs',exact:true}).waitFor();check(await controls.getByRole('button',{name:'Mulai ulang aplikasi',exact:true}).count()===0,`${kind} does not offer duplicate application restart`);}
 data=structuredClone(initial);await page.reload();await controls.getByRole('button',{name:'Hentikan',exact:true}).waitFor();await page.screenshot({path:path.join(output,'desktop.png')});
 await page.setViewportSize({width:320,height:740});check(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'controls fit a 320px viewport');await page.screenshot({path:path.join(output,'mobile.png'),fullPage:true});
 await page.evaluate(()=>{localStorage.setItem('nusa_theme','dark');location.reload();});await controls.getByRole('button',{name:'Hentikan',exact:true}).waitFor();check(await page.locator('html').evaluate(el=>el.classList.contains('dark')),'controls follow existing dark theme');await page.screenshot({path:path.join(output,'dark-mobile.png'),fullPage:true});
 check(errors.length===0,'no browser runtime errors');await fs.writeFile(path.join(output,'results.json'),JSON.stringify({checks,actions,errors},null,2));
 }finally{await browser.close();}})().catch(error=>{console.error(error);process.exitCode=1;});
