#!/usr/bin/env node
// Optional browser regression gate: install Playwright separately; no agent runtime dependency.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const { chromium } = require('playwright');
const root = path.resolve(__dirname, '../apps/dashboard');
const terminalId = '00000000-0000-4000-8000-000000000001';
const jobId = '00000000-0000-4000-8000-000000000002';
const transferId = '00000000-0000-4000-8000-000000000003';
const badName = '<img src=x onerror="window.xss=1">';
const devices = [
  {device_id:'online',device_name:badName,platform:'linux',arch:'aarch64',agent_version:'test',online:true,capabilities:['exec','files','terminal','jobs','transfer','desktop'],active_terminals:1,last_seen_unix_ms:Date.now()},
  {device_id:'offline',device_name:'Rose',platform:'windows',arch:'x86_64',agent_version:'test',online:false,connection_state:'reconnecting',last_disconnect_reason:'heartbeat_timeout',capabilities:['terminal'],last_seen_unix_ms:Date.now()-90000}
];
let sessions = [
  {id:terminalId,kind:'terminal',status:'running',device_id:'online',device_name:badName,device_online:true,started_at_unix_ms:Date.now()},
  {id:jobId,kind:'job',status:'running',device_id:'online',device_name:badName,device_online:true,started_at_unix_ms:Date.now()},
  {id:transferId,kind:'transfer',status:'ready',device_id:'online',device_name:badName,device_online:true,started_at_unix_ms:Date.now()}
];
let overviewMode=null, overviewCalls=0, hangEnrollment=false, enrollmentCalls=0;
let fileListMode='legacy',failFilePage=false,filePageDelay=0;
const pairId='00000000-0000-4000-8000-000000000004',pairDeviceId='00000000-0000-4000-8000-000000000005';
let pairState='pending',pairApproveState='approved',pairOnline=false,pairReply='ok',pairDelay=0,hangApproval=false,approvalCalls=0,pairChecks=0;
const pairRecord=()=>({request_id:pairId,device_name:'Windows Test PC',platform:'windows',arch:'x86_64',verification_code:'482731',state:pairState,device_id:pairState==='approved'?pairDeviceId:null});
const fixtureDevices=()=>pairState==='approved'?[...devices,{device_id:pairDeviceId,device_name:'Windows Test PC',platform:'windows',arch:'x86_64',agent_version:'test',online:pairOnline,capabilities:['terminal','files']}]:devices;
const hungResponses=new Set();
const hang=(res,body=false)=>{hungResponses.add(res);res.on('close',()=>hungResponses.delete(res));if(body){res.writeHead(200,{'Content-Type':'application/json'});res.write('{"devices":');}};
let authenticated=false, output='fixture PTY ready\r\n', failWrite=false, failRead=false, delayedRead=false, readDelay=0;
const operations=[], errors=[], streams=new Set();
const server=http.createServer(async(req,res)=>{
  const send=(value,status=200)=>{res.writeHead(status,{'Content-Type':'application/json','Cache-Control':'no-store'});res.end(JSON.stringify(value));};
  if(req.url==='/api/auth/session'){
    if(req.method==='DELETE')authenticated=false;
    if(req.method==='POST'){let body='';for await(const chunk of req)body+=chunk;const input=JSON.parse(body);if(input.username!=='owner'||input.password!=='fixture-password-123')return send({error:'Incorrect username or password.'},401);authenticated=true;}
    return send({authenticated,mode:authenticated?'browser_session':'none',login_method:'password'});
  }
  if(req.url.startsWith('/api/')){
    if(!authenticated)return send({error:'Authentication required'},401);
    if(req.url==='/api/devices/online/rename'&&req.method==='POST'){let body='';for await(const chunk of req)body+=chunk;devices[0].device_name=JSON.parse(body).name||badName;return send({ok:true});}
    if(req.url==='/api/pair/requests'&&req.method==='GET')return send(pairState==='pending'?[pairRecord()]:[]);
    if(req.url==='/api/pair/'+pairId&&req.method==='GET')return send(pairRecord());
    if(req.url==='/api/pair/'+pairId+'/approve'&&req.method==='POST'){approvalCalls++;pairState=pairApproveState;if(hangApproval)return hang(res);return send(pairRecord());}
    if(req.url==='/api/pair/00000000-0000-4000-8000-000000000004/reject'&&req.method==='POST')return send({ok:true});
    if(req.url==='/api/enrollments'&&req.method==='POST'){enrollmentCalls++;if(hangEnrollment)return hang(res);return send({code:'fixture-5-minute-code',expires_at_unix_ms:Date.now()+300000});}
    if(req.url==='/api/events'){res.writeHead(200,{'Content-Type':'text/event-stream'});res.write(': connected\n\n');streams.add(res);res.on('close',()=>streams.delete(res));return;}
    if(req.url==='/api/overview'){overviewCalls++;if(overviewMode)return hang(res,overviewMode==='body');return send({devices:fixtureDevices(),sessions,activity:[]});}
    if(req.url==='/api/devices')return send(fixtureDevices());
    if(req.url==='/api/devices/'+pairDeviceId+'/operations'){
      pairChecks++;const reply=pairReply,delay=pairDelay;
      let body='';for await(const chunk of req)body+=chunk;
      assert.deepEqual(JSON.parse(body),{op:'status'});
      if(delay)await new Promise(resolve=>setTimeout(resolve,delay));
      return send(reply==='ok'?{status:'ok',body:{pid:321,protocol:1}}:reply==='malformed'?{status:'ok',body:{}}:{status:'deadline_exceeded'});
    }
    if(req.url==='/api/sessions')return send(sessions);
    if(req.url==='/api/activity')return send([]);
    if(req.url==='/api/devices/online/operations'){
      let body='';for await(const chunk of req)body+=chunk;
      const op=JSON.parse(body);operations.push(op);const a=op.action;
      const ok=value=>send({status:'ok',body:value});
      if(op.op==='status')return ok({pid:100,protocol:1});
      if(op.op==='desktop'&&a.action==='permissions')return ok({accessibility:{granted:false},screen_recording:{granted:true},ready:false});
      if(op.op==='terminal'){
        if(a.action==='read'){
          if(failRead)return send({error:'network unavailable'},503);
          if(delayedRead){delayedRead=false;await new Promise(resolve=>setTimeout(resolve,readDelay));}
          return ok({session:{id:a.session_id,status:'running'},offset:a.offset,next_offset:output.length,output:output.slice(a.offset||0),has_more:false});
        }
        if(a.action==='write'){
          if(failWrite)return send({status:'outcome_unknown',error:'connection lost after dispatch'});
          output+=a.data;return ok({ok:true});
        }
        if(a.action==='open')return ok({session:{id:terminalId,status:'running'}});
        if(a.action==='close'){sessions=sessions.filter(s=>s.id!==a.session_id);return ok({ok:true});}
        return ok({ok:true});
      }
      if(op.op==='jobs'){
        if(a.action==='cancel'){sessions=sessions.filter(s=>s.id!==a.job_id);return ok({job:{id:a.job_id,status:'canceling'}});}
        return ok({job:{id:a.job_id,status:'running'},offset:0,next_offset:17,output:'fixture job output',has_more:false});
      }
      if(op.op==='transfer'){
        if(a.action==='cancel')sessions=sessions.filter(s=>s.id!==a.transfer_id);
        return ok({transfer:{id:a.transfer_id,status:a.action==='cancel'?'canceled':'ready',path:'build.zip',offset:20,size:100}});
      }
      if(op.op==='files'){
        if(a.action==='list'){
          if(fileListMode==='paged'){
            if(failFilePage)return send({error:'directory temporarily unavailable'},503);
            // A byte cap may return fewer entries than the requested length.
            const offset=a.offset||0,names=[badName,'entry-01','entry-02','entry-03','entry-04'];
            const entries=names.slice(offset,offset+2).map(name=>({name,path:a.path+'/'+name,kind:'file',size:9}));
            const result={path:a.path,entries,offset,next_offset:offset+entries.length,total_entries:5,has_more:offset+entries.length<5,truncated:offset+entries.length<5,pagination_supported:true};
            if(filePageDelay)await new Promise(resolve=>setTimeout(resolve,filePageDelay));
            return ok(result);
          }
          return ok({path:a.path,entries:[{name:badName,path:'./'+badName,kind:'file',size:9}],truncated:fileListMode==='legacy-truncated'});
        }
        return ok({path:a.path,content:'<script>window.xss=2</script>',offset:0,next_offset:29,size:29,has_more:false});
      }
      return send({error:'unexpected operation'},400);
    }
    return send({error:'not found'},404);
  }
  const routes={'/logo.png':'assets/logo.png','/assets/logo.png':'assets/logo.png','/favicon.ico':'assets/favicon.ico','/favicon.png':'assets/favicon.png','/apple-touch-icon.png':'assets/apple-touch-icon.png','/':'index.html','/app.js':'app.js','/styles.css':'styles.css','/vendor/xterm.js':'vendor/xterm.js','/vendor/xterm.css':'vendor/xterm.css'};
  const name=routes[req.url];if(!name){res.writeHead(404);res.end();return;}
  const type=name.endsWith('.js')?'text/javascript':name.endsWith('.css')?'text/css':name.endsWith('.png')?'image/png':name.endsWith('.ico')?'image/x-icon':'text/html';
  res.writeHead(200,{'Content-Type':type,'Content-Security-Policy':"default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'"});res.end(fs.readFileSync(path.join(root,name)));
});
(async()=>{
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const base=`http://127.0.0.1:${server.address().port}`;
  const browser=await chromium.launch({headless:true,...(process.env.INFRA_CHROMIUM_EXECUTABLE?{executablePath:process.env.INFRA_CHROMIUM_EXECUTABLE,args:['--no-sandbox','--disable-dev-shm-usage']}: {})});
  try{
    const page=await browser.newPage({viewport:{width:1280,height:900}});
    page.on('pageerror',e=>errors.push(e.message));
    page.on('console',msg=>{if(msg.type()==='error'&&msg.text().includes('Content Security Policy'))errors.push(msg.text());});
    await page.goto(base);await page.locator('#login-submit:not([disabled])').waitFor();
    assert(await page.locator('.signin-showcase-brand img.brand-mark').evaluate(img=>img.complete&&img.naturalWidth>0),'Infra logo did not load');
    assert.equal(await page.locator('.signin-showcase-brand img.brand-mark').getAttribute('src'),'/assets/logo.png');
    assert.equal(await page.locator('.signin-main .brand-mark').count(),1);


    const darkMetrics=await page.evaluate(()=>{
      const rgb=selector=>{
        const value=getComputedStyle(document.querySelector(selector)).backgroundColor;
        return value.match(/[0-9.]+/g).slice(0,3).map(Number);
      };
      return {mode:getComputedStyle(document.documentElement).colorScheme,
        page:rgb('body'),login:rgb('#signin'),input:rgb('#login-password')};
    });
    assert.equal(darkMetrics.mode,'dark');
    assert(darkMetrics.page.every(channel=>channel<40),'Page is not dark');
    assert(darkMetrics.login.every(channel=>channel<40),'Login is not dark');
    assert(darkMetrics.input.every(channel=>channel<55),'Login input is not dark');

    if(process.env.INFRA_QA_LOGIN_SCREENSHOT){await page.waitForTimeout(650);await page.screenshot({path:process.env.INFRA_QA_LOGIN_SCREENSHOT,fullPage:true});}
    assert.equal(await page.locator('#app-shell').isVisible(),false);
    await page.locator('#login-username').fill('owner');await page.locator('#login-password').fill('wrong');await page.locator('#login-submit').click();
    await page.waitForFunction(()=>document.querySelector('#login-error').textContent.includes('Incorrect'));assert.equal(await page.locator('#app-shell').isVisible(),false);
    await page.locator('#login-password').fill('fixture-password-123');await page.locator('#login-submit').click();
    await page.locator('#overview-devices [data-device="online"]').waitFor();
    assert.equal(await page.locator('#overview-devices .device-avatar svg.ui-icon').count(),2);
    assert.equal(await page.locator('#summary .metric-icon svg.ui-icon').count(),4);
    assert.equal(await page.locator('#overview-sessions .session-kind-icon svg.ui-icon').count(),3);
    assert.equal(await page.locator('#overview-devices .device-row').count(),2);
    assert.match(await page.locator('#summary').textContent(),/Connected/);
    if(process.env.INFRA_QA_OVERVIEW_SCREENSHOT||process.env.INFRA_QA_MOBILE_OVERVIEW_SCREENSHOT){
      // Public design previews use readable sample labels; the functional
      // tests below still exercise the original hostile-name/XSS fixture.
      const saved=[devices.map(d=>d.device_name),sessions.map(item=>item.device_name)];
      devices[0].device_name='Studio MacBook';
      devices[1].device_name='Windows Workstation';
      sessions.forEach(item=>item.device_name='Studio MacBook');
      await page.reload();
      await page.locator('#overview-devices .device-row').first().waitFor();
      await page.waitForTimeout(500);
      if(process.env.INFRA_QA_OVERVIEW_SCREENSHOT)
        await page.screenshot({path:process.env.INFRA_QA_OVERVIEW_SCREENSHOT,fullPage:true});
      if(process.env.INFRA_QA_MOBILE_OVERVIEW_SCREENSHOT){
        await page.setViewportSize({width:390,height:844});
        await page.evaluate(()=>scrollTo(0,0));
        await page.waitForTimeout(200);
        await page.screenshot({path:process.env.INFRA_QA_MOBILE_OVERVIEW_SCREENSHOT,fullPage:true});
        await page.setViewportSize({width:1280,height:900});
      }
      devices.forEach((d,i)=>d.device_name=saved[0][i]);
      sessions.forEach((item,i)=>item.device_name=saved[1][i]);
      await page.reload();
      await page.locator('#overview-devices [data-device="online"]').waitFor();
    }
    await page.goto(base+'/#add-device');
    await page.locator('#enroll-dialog[open]').waitFor();
    assert(await page.locator('#enroll-dialog').evaluate(node=>{
      const rgb=getComputedStyle(node).backgroundColor.match(/[0-9.]+/g).slice(0,3).map(Number);
      return rgb.every(value=>value<50);
    }),'Pairing dialog contains a light surface');

    await page.locator('#enroll-dialog').evaluate(element=>element.close());
    await page.goto(base);
    await page.locator('#overview-devices [data-device="online"]').waitFor();
    await page.locator('#add-device').click();
    await page.locator('[data-quick-platform="windows"]').click();
    const quickCommand=await page.locator('#quick-install-command').textContent();
    assert.match(quickCommand,/Invoke-WebRequest[\s\S]*-UseBasicParsing[\s\S]*-OutFile/);
    assert.match(quickCommand,/guid/);
    assert.match(quickCommand,/LASTEXITCODE/);
    assert.match(quickCommand,/finally[\s\S]*Remove-Item/);
    await page.waitForFunction(()=>document.querySelector('#pending-pairs').textContent.includes('Windows Test PC'));
    assert.match(await page.locator('#pending-pairs').textContent(),/482731/);
    const originalPairDecoyName=devices[0].device_name;devices[0].device_name='Windows Test PC';
    await page.locator('#pending-pairs [data-pair-action="approve"]').click();
    await page.waitForFunction(()=>document.querySelector('#pair-result').textContent.includes('Approved'));
    assert.equal(await page.locator('#pair-open').isVisible(),false,'Approval alone must not mean connected');
    pairOnline=true;pairReply='failed';
    await page.waitForFunction(()=>document.querySelector('#pair-error').textContent.includes('Connection check unavailable'));
    assert.equal(await page.locator('#pair-open').isVisible(),false,'Online metadata without a live reply must not pass');
    pairReply='malformed';const malformedBefore=pairChecks;
    for(let n=0;pairChecks===malformedBefore&&n<120;n++)await page.waitForTimeout(50);
    assert(pairChecks>malformedBefore,'Malformed reply fixture was not reached');
    await page.waitForTimeout(100);
    assert.equal(await page.locator('#pair-open').isVisible(),false,'Malformed status must not pass');
    pairReply='ok';
    await page.locator('#pair-open:not(.hidden)').waitFor();
    assert.equal(approvalCalls,1,'Connection checks must never replay approval');
    if(process.env.INFRA_QA_PAIRING_SCREENSHOT)await page.locator('#enroll-dialog').screenshot({path:process.env.INFRA_QA_PAIRING_SCREENSHOT});
    await page.locator('#pair-open').click();
    await page.waitForURL('**/#device/'+pairDeviceId+'/overview');
    assert.equal(await page.locator('#enroll-dialog').isVisible(),false);
    pairState='pending';pairOnline=false;devices[0].device_name=originalPairDecoyName;
    await page.goto(base);await page.locator('#add-device').click();
    await page.locator('.legacy-enrollment summary').click();
    await page.locator('#create-enrollment').click();
    await page.waitForFunction(()=>document.querySelector('#enrollment-code').textContent.includes('fixture-5-minute-code'));
    await page.locator('[data-platform="windows"]').click();
    assert.match(await page.locator('#install-command').textContent(),/Invoke-WebRequest -UseBasicParsing -Uri/);
    assert.match(await page.locator('#install-command').textContent(),/fixture-5-minute-code/);
    await page.locator('#enroll-dialog').evaluate(element=>element.close());

    assert.equal(await page.locator('#summary .summary-card').nth(2).locator('strong').textContent(),'3');
    await page.locator('#overview-devices [data-device="online"]').click();
    await page.waitForFunction(()=>document.querySelector('#device-detail').classList.contains('active'));
    assert.equal(await page.locator('#view-title').textContent(),badName);
    assert.equal(await page.locator('#device-summary img').count(),0);
    assert.equal(await page.locator('#device-summary .device-hero-avatar svg.ui-icon').count(),1);
    assert.equal(await page.locator('#device-capabilities .cap-icon svg.ui-icon').count(),11);
    await page.locator('#device-rename').click();
    await page.locator('#device-rename-name').fill('Developer Mac');
    await page.locator('#device-rename-form button[type="submit"]').click();
    await page.waitForFunction(()=>document.querySelector('#device-rename-form').classList.contains('hidden'));
    assert.equal(await page.locator('#view-title').textContent(),'Developer Mac');
    await page.locator('#device-status').click();await page.locator('#device-status-output').waitFor();
    assert.match(await page.locator('#device-status-output').textContent(),/PID: 100/);
    devices[0].device_name=badName;
    await page.locator('#device-permissions').click();
    await page.waitForFunction(()=>document.querySelector('#device-status-output').textContent.includes('Needs permission'));
    await page.locator('[data-device-tab="files"]').click();await page.locator('#files-browse button').first().click();
    await page.locator('[data-file-path]').waitFor();
    assert.equal(await page.locator('#files-list [data-file-path] svg.ui-icon').count(),1);await page.locator('[data-file-path]').click();await page.locator('#file-preview:not(.hidden)').waitFor();
    assert.equal(await page.locator('#file-content').textContent(),'<script>window.xss=2</script>');assert.equal(await page.evaluate(()=>window.xss),undefined);
    assert.equal(operations.filter(op=>op.op==='files'&&op.action.action==='list').at(-1).action.offset,undefined);
    assert.equal(await page.locator('#files-pagination').isVisible(),false);

    fileListMode='paged';devices[0].capabilities.push('files.list_page');await page.evaluate(()=>refresh());
    await page.locator('#files-browse button').first().click();
    await page.waitForFunction(()=>document.querySelector('#files-note').textContent.includes('1–2 of 5'));
    assert.equal(await page.locator('#files-list [data-file-path]').count(),2);
    assert.equal(await page.locator('#files-previous').isDisabled(),true);
    if(process.env.INFRA_QA_FILES_SCREENSHOT)await page.locator('#device-files').screenshot({path:process.env.INFRA_QA_FILES_SCREENSHOT});
    await page.locator('#files-next').focus();await page.keyboard.press('Enter');
    await page.waitForFunction(()=>document.querySelector('#files-note').textContent.includes('3–4 of 5'));
    assert.equal(operations.filter(op=>op.op==='files'&&op.action.action==='list').at(-1).action.offset,2);
    failFilePage=true;await page.locator('#files-next').click();
    await page.waitForFunction(()=>document.querySelector('#device-error').textContent.includes('directory temporarily unavailable'));
    assert.equal(await page.locator('#files-list [data-file-path]').count(),2);
    assert.match(await page.locator('#files-note').textContent(),/last confirmed page/);
    failFilePage=false;await page.locator('#files-path').fill('unsubmitted-path');await page.locator('#files-next').click();
    await page.waitForFunction(()=>document.querySelector('#files-note').textContent.includes('5–5 of 5'));
    assert.equal(await page.locator('#files-list [data-file-path]').count(),1);
    assert.equal(await page.locator('#files-next').isDisabled(),true);
    assert.equal(operations.filter(op=>op.op==='files'&&op.action.action==='list').at(-1).action.path,'.');
    await page.locator('#files-previous').click();
    await page.waitForFunction(()=>document.querySelector('#files-note').textContent.includes('3–4 of 5'));
    await page.locator('#files-previous').click();
    await page.waitForFunction(()=>document.querySelector('#files-note').textContent.includes('1–2 of 5'));
    await page.setViewportSize({width:390,height:844});
    assert(await page.locator('#files-next').isVisible());
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
    if(process.env.INFRA_QA_MOBILE_FILES_SCREENSHOT)await page.locator('#device-files').screenshot({path:process.env.INFRA_QA_MOBILE_FILES_SCREENSHOT});
    await page.setViewportSize({width:1280,height:900});
    filePageDelay=250;await page.locator('#files-next').click();
    await page.evaluate(()=>navigateDevice('offline','files'));
    await page.waitForFunction(()=>document.querySelector('#view-title').textContent==='Rose');
    await page.waitForTimeout(350);filePageDelay=0;
    assert.equal(await page.locator('#files-list [data-file-path]').count(),0);
    assert.equal(await page.locator('#files-pagination').isVisible(),false);
    await page.evaluate(()=>navigateDevice('online','files'));
    await page.locator('#files-browse button:not([disabled])').first().waitFor();
    await page.locator('#files-browse button').first().click();
    await page.waitForFunction(()=>document.querySelector('#files-note').textContent.includes('1–2 of 5'));
    filePageDelay=250;await page.locator('#files-next').click();
    await page.evaluate(()=>{location.hash='overview';});await page.waitForTimeout(350);filePageDelay=0;
    await page.evaluate(()=>navigateDevice('online','files'));
    await page.waitForFunction(()=>document.querySelector('#device-files').classList.contains('hidden')===false);
    assert.match(await page.locator('#files-note').textContent(),/last confirmed page/);
    devices[0].capabilities=devices[0].capabilities.filter(cap=>cap!=='files.list_page');await page.evaluate(()=>refresh());
    assert.equal(await page.locator('#files-next').isDisabled(),true);
    fileListMode='legacy-truncated';await page.locator('#files-browse button').first().click();
    await page.waitForFunction(()=>document.querySelector('#files-note').textContent.includes('Listing incomplete'));
    assert.equal(await page.locator('#files-pagination').isVisible(),false);
    const legacy=operations.filter(op=>op.op==='files'&&op.action.action==='list').at(-1).action;
    assert.equal(legacy.offset,undefined);assert.equal(legacy.length,undefined);fileListMode='legacy';
    await page.locator('[data-view="devices"]').click();
    await page.locator('#device-search').fill('no-such-device');
    assert.equal(await page.locator('#all-devices .device-row:not(.hidden)').count(),0);
    assert.match(await page.locator('#device-search-empty').textContent(),/No matching devices/);
    await page.locator('#device-search').fill('windows');
    assert.equal(await page.locator('#all-devices .device-row:not(.hidden)').count(),1);
    await page.locator('#device-search').fill('');
    assert.equal(await page.locator('#all-devices .device-row:not(.hidden)').count(),2);
    await page.locator('[data-view="sessions"]').click();await page.locator('#session-kind').selectOption('job');
    assert.equal(await page.locator('#all-sessions .session-row').count(),1);
    await page.locator('#session-kind').selectOption('transfer');await page.locator('#all-sessions [data-session]').click();
    await page.waitForFunction(()=>document.querySelector('#session-output').textContent.includes('build.zip'));
    assert.equal(await page.locator('#session-stop').isDisabled(),false);
    page.once('dialog',dialog=>dialog.accept());await page.locator('#session-stop').click();await page.waitForTimeout(100);
    assert(operations.some(o=>o.op==='transfer'&&o.action.action==='cancel'&&o.action.transfer_id===transferId));
    await page.locator('[data-view="sessions"]').click();
    await page.locator('#session-kind').selectOption('terminal');await page.locator('#all-sessions [data-session]').click();
    await page.locator('.xterm-screen').waitFor();await page.waitForFunction(()=>document.querySelector('#session-read-state').textContent.includes('bytes read'));
    assert(await page.locator('#terminal-screen').evaluate(node=>
      getComputedStyle(node).backgroundColor.match(/[0-9.]+/g).slice(0,3).map(Number).every(value=>value<30)
    ),'Terminal is not near-black');

    await page.locator('.xterm-helper-textarea').focus();await page.keyboard.type('abc');
    await page.waitForFunction(()=>document.querySelector('.xterm-screen').textContent.includes('abc'));
    assert.equal(operations.filter(o=>o.op==='terminal'&&o.action.action==='open').length,0);
    assert.equal(operations.filter(o=>o.op==='terminal'&&o.action.action==='write').map(o=>o.action.data).join(''),'abc');
    await page.locator('#terminal-cols').fill('90');await page.locator('#terminal-resize').click();
    await page.waitForTimeout(100);assert(operations.some(o=>o.action?.action==='resize'&&o.action.session_id===terminalId&&o.action.cols===90));
    // Refresh attaches to the same ID, then navigation detaches without closing it.
    await page.reload();await page.locator('.xterm-screen').waitFor();
    await page.waitForFunction(()=>document.querySelector('#session-read-state').textContent.includes('bytes read'));
    assert(operations.filter(o=>o.op==='terminal'&&o.action.action==='read').every(o=>o.action.session_id===terminalId));
    failWrite=true;await page.locator('.xterm-helper-textarea').focus();await page.keyboard.type('!');
    await page.waitForFunction(()=>document.querySelector('#session-error').textContent.includes('outcome_unknown'));
    const writes=operations.filter(o=>o.action?.action==='write').length;await page.keyboard.type('discarded');await page.waitForTimeout(850);
    assert.equal(operations.filter(o=>o.action?.action==='write').length,writes);
    failWrite=false;await page.locator('#session-resume').click();await page.locator('.xterm-helper-textarea').focus();await page.keyboard.type('Z');await page.waitForTimeout(150);
    assert.equal(operations.filter(o=>o.action?.action==='write').at(-1).action.data,'Z');
    await page.locator('[data-view="devices"]').click();assert.equal(operations.filter(o=>o.action?.action==='close').length,0);
    await page.locator('#all-devices [data-device="offline"]').click();
    await page.waitForFunction(()=>document.querySelector('#device-summary').textContent.includes('No heartbeat was received'));
    assert.match(await page.locator('#device-summary').textContent(),/No heartbeat was received/);
    assert.match(await page.locator('#device-summary .status').textContent(),/reconnecting/);await page.locator('[data-device-tab="terminal"]').click();assert(await page.locator('#terminal-open button').isDisabled());
    await page.goto(`${base}/#session/online/job/${jobId}`);await page.locator('#session-output').waitFor();
    await page.waitForFunction(()=>document.querySelector('#session-output').textContent.includes('fixture job output'));
    page.once('dialog',dialog=>dialog.accept());await page.locator('#session-stop').click();await page.waitForTimeout(100);
    assert(operations.some(o=>o.op==='jobs'&&o.action.action==='cancel'&&o.action.job_id===jobId));
    await page.setViewportSize({width:390,height:844});await page.goto(base);await page.locator('#overview-devices [data-device="online"]').waitFor();
    await page.locator('#overview-devices [data-device="online"]').click();
    if(process.env.INFRA_QA_SCREENSHOT){await page.waitForTimeout(300);await page.screenshot({path:process.env.INFRA_QA_SCREENSHOT,fullPage:true});}
    await page.waitForFunction(()=>document.documentElement.scrollWidth<=innerWidth+1,{timeout:4000});
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=window.innerWidth),JSON.stringify(await page.evaluate(()=>[...document.querySelectorAll('body *')].filter(e=>e.getBoundingClientRect().right>innerWidth+1).map(e=>[e.tagName,e.id,e.className,e.getBoundingClientRect().right]).slice(0,15))));
    if(process.env.INFRA_QA_SCREENSHOT)await page.screenshot({path:process.env.INFRA_QA_SCREENSHOT,fullPage:true});
    // Global responsive gate, including tiny phones and dark OS preference.
    for(const width of [320,390,768,1280]){
      const navNames=await page.locator('.sidebar button.nav').evaluateAll(nodes=>nodes.map(node=>node.getAttribute('aria-label')));
      assert.deepEqual(navNames,['Overview','Devices','Sessions','Activity','Settings']);

      await page.setViewportSize({width,height:810});
      await page.goto(base+'/#overview');
      await page.locator('#overview-devices .device-row').first().waitFor();
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),'Overview overflowed at '+width+'px: '+JSON.stringify(await page.evaluate(()=>[...document.querySelectorAll('body *')].filter(e=>e.getBoundingClientRect().right>innerWidth+1).slice(0,16).map(e=>[e.tagName,e.id,e.className,Math.round(e.getBoundingClientRect().right),e.parentElement?.outerHTML.slice(0,260)]))));
      await page.goto(base+'/#devices');
      await page.locator('#all-devices .device-row').first().waitFor();
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),'Devices inventory overflowed at '+width+'px');
    }
    await page.emulateMedia({colorScheme:'dark'});
    await page.goto(base+'/#overview');
    await page.locator('#overview-devices .device-row').first().waitFor();
    assert.equal(await page.evaluate(()=>getComputedStyle(document.documentElement).colorScheme),'dark');
    await page.emulateMedia({colorScheme:'light'});
    await page.goto(base+'/#overview');
    await page.locator('#overview-devices .device-row').first().waitFor();
    assert.equal(await page.evaluate(()=>getComputedStyle(document.documentElement).colorScheme),'dark');

    // Both absent headers and a stalled JSON body must release the coalesced
    // refresh promise. Last confirmed device data remains visibly stale.
    for(const mode of ['headers','body']){
      overviewMode=mode;
      const before=overviewCalls;
      await page.evaluate(()=>{void refresh();});
      await page.waitForFunction(()=>document.querySelector('#gateway-label').textContent.includes('last confirmed data'),{},{timeout:15000});
      assert(overviewCalls>before);
      assert.equal(await page.locator('#overview-devices .device-row').count(),2);
      assert.match(await page.locator('#gateway-label').getAttribute('title'),/timed out/);
      overviewMode=null;
      await page.evaluate(()=>refresh());
      await page.waitForFunction(()=>document.querySelector('#gateway-label').textContent==='Gateway online');
    }
    // Separate context: virtual time checks the real wait/expiry UI without
    // waiting 90 seconds or changing production budgets.
    const pairPage=await browser.newPage({viewport:{width:390,height:844}});
    await pairPage.clock.install();
    const waitFixture=async(predicate,message)=>{
      for(let n=0;!predicate()&&n<240;n++)await new Promise(resolve=>setTimeout(resolve,25));
      assert(predicate(),message);
    };
    try{
      pairState='pending';pairOnline=false;pairReply='ok';
      await pairPage.goto(base);await pairPage.locator('#add-device').click();
      const before=approvalCalls;
      await pairPage.locator('[data-pair-action="approve"]').click();
      await pairPage.waitForFunction(()=>document.querySelector('#pair-result').textContent.includes('Approved'));
      await pairPage.clock.fastForward(95_000);
      await pairPage.locator('#pair-check:not([disabled])').waitFor();
      assert.match(await pairPage.locator('#pair-result').textContent(),/connection could not be verified/);
      assert.equal(await pairPage.locator('#pair-open').isVisible(),false);
      assert.equal(approvalCalls,before+1);
      pairOnline=true;
      await pairPage.locator('#pair-check').click();
      await pairPage.locator('#pair-open:not(.hidden)').waitFor();
      assert.equal(approvalCalls,before+1,'Explicit connection retry must be read-only apart from status');
      assert(await pairPage.locator('#enroll-dialog').evaluate(node=>node.scrollWidth<=node.clientWidth+1),'Pairing dialog overflows on mobile');
      if(process.env.INFRA_QA_PAIRING_MOBILE_SCREENSHOT)await pairPage.locator('#enroll-dialog').screenshot({path:process.env.INFRA_QA_PAIRING_MOBILE_SCREENSHOT});
      await pairPage.clock.fastForward(31_000);
      await pairPage.locator('#pair-check:not([disabled])').waitFor();
      assert.match(await pairPage.locator('#pair-result').textContent(),/last connection check expired/);
      assert.equal(await pairPage.locator('#pair-open').isVisible(),false);

      // Server records approval but its response never reaches the browser.
      pairState='pending';hangApproval=true;
      await pairPage.locator('#pair-another').click();
      const lostBefore=approvalCalls;
      await pairPage.locator('[data-pair-action="approve"]').click();
      await waitFixture(()=>approvalCalls===lostBefore+1,'Approval fixture was not reached');
      await pairPage.clock.fastForward(11_000);
      await pairPage.locator('#pair-open:not(.hidden)').waitFor();
      hangApproval=false;
      assert.equal(approvalCalls,lostBefore+1,'Lost response must be observed without approval replay');

      // If approval did not commit, explicitly review the request; no replay.
      pairState='pending';pairApproveState='pending';
      await pairPage.locator('#pair-another').click();
      await pairPage.locator('[data-pair-action="approve"]').click();
      await pairPage.waitForFunction(()=>document.querySelector('#pair-result').textContent.includes('Approval has not been recorded'));
      const unrecorded=approvalCalls;
      await pairPage.locator('#pair-another').click();
      await pairPage.locator('[data-pair-action="approve"]').waitFor();
      assert.equal(approvalCalls,unrecorded);

      // Expired requests explain recovery rather than reporting Connected.
      pairApproveState='expired';
      await pairPage.locator('[data-pair-action="approve"]').click();
      await pairPage.waitForFunction(()=>document.querySelector('#pair-result').textContent.includes('pairing request expired'));
      assert.equal(await pairPage.locator('#pair-open').isVisible(),false);

      // A late successful status from a closed dialog cannot verify a reopened
      // flow whose new check fails. The observer cancels the old request.
      pairState='pending';pairApproveState='approved';pairReply='ok';pairDelay=800;
      await pairPage.locator('#pair-another').click();
      const delayedBefore=pairChecks;
      await pairPage.locator('[data-pair-action="approve"]').click();
      await waitFixture(()=>pairChecks>delayedBefore,'Delayed check was not reached');
      await pairPage.locator('#enroll-dialog').evaluate(node=>node.close());
      pairReply='failed';pairDelay=0;
      await pairPage.locator('#add-device').click();
      await pairPage.waitForFunction(()=>document.querySelector('#pair-error').textContent.includes('Connection check unavailable'));
      await new Promise(resolve=>setTimeout(resolve,1000));
      assert.equal(await pairPage.locator('#pair-open').isVisible(),false,'Old dialog response restored success');

      // Session expiry closes and clears private pairing state, including an
      // outstanding response. No prior label or comparison code can return.
      await pairPage.locator('#enroll-dialog').evaluate(node=>node.close());
      pairReply='ok';pairDelay=800;
      const logoutBefore=pairChecks;
      await pairPage.locator('#add-device').click();
      await waitFixture(()=>pairChecks>logoutBefore,'Logout check was not reached');
      authenticated=false;
      await pairPage.evaluate(()=>jsonFetch('/api/pair/requests').catch(()=>{}));
      assert.equal(await pairPage.locator('#enroll-dialog').isVisible(),false);
      assert.equal(await pairPage.locator('#pair-result').textContent(),'');
      assert.equal(await pairPage.locator('#pending-pairs').textContent(),'');
      await new Promise(resolve=>setTimeout(resolve,1000));
      assert.equal(await pairPage.locator('#pair-result').textContent(),'');
    }finally{
      await pairPage.close();authenticated=true;pairState='pending';pairApproveState='approved';pairOnline=false;pairReply='ok';pairDelay=0;hangApproval=false;
    }
    await page.reload();await page.locator('#overview-devices [data-device="online"]').waitFor();

    // A mutation may have reached the server even if its response never arrives.
    // The browser reports uncertainty and must never replay it automatically.
    hangEnrollment=true;
    const mutationsBefore=enrollmentCalls;
    const timeout=await page.evaluate(async()=>{
      try{await jsonFetch('/api/enrollments',{method:'POST',body:'{}'});return null;}
      catch(error){return {code:error.code,message:error.message};}
    });
    hangEnrollment=false;
    assert.equal(timeout.code,'request_timeout');
    assert.match(timeout.message,/outcome may be unknown/);
    assert.equal(enrollmentCalls,mutationsBefore+1);

    await page.setViewportSize({width:1280,height:900});
    await page.locator('[data-view="settings"]').click();await page.locator('#logout-session').click();
    await page.waitForFunction(()=>document.querySelector('#all-devices').textContent.includes('No devices'));
    assert.equal(await page.locator('#app-shell').isVisible(),false);assert.equal(await page.locator('#signin').isVisible(),true);
    assert.equal(await page.locator('#file-content').textContent(),'');assert.equal(await page.locator('#session-output').textContent(),'');
    assert.equal(await page.locator('#files-list').textContent(),'');assert.equal(await page.locator('#files-note').textContent(),'');
    assert.equal(await page.evaluate(()=>state.listing),null);
    assert.equal(await page.locator('#files-path').inputValue(),'.');
    assert.deepEqual(errors,[]);
    console.log('Dashboard browser gate PASS: pairing completion with live proof, uncertainty/no replay, bounded recovery, expiry and dialog/auth isolation; auth/device/XSS/terminal/session/CSP coverage; file pages with keyboard/mobile navigation, byte-shortened offsets, retained error recovery, stale-response isolation, legacy fallback and private-state clearing; bounded header/body timeout recovery and mutation no-replay.');
  }finally{for(const response of hungResponses)response.destroy();await browser.close();for(const stream of streams)stream.end();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);server.close();process.exitCode=1;});
