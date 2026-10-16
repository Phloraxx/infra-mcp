#!/usr/bin/env node
// Explicit live acceptance: credentials are read from a private file, never argv.
const fs=require('node:fs');
const assert=require('node:assert/strict');
const {chromium}=require('playwright');
const credentials=JSON.parse(fs.readFileSync(process.env.INFRA_QA_CREDENTIALS_FILE,'utf8'));
const base=process.env.INFRA_QA_URL;
if(!base?.startsWith('https://'))throw new Error('INFRA_QA_URL must use HTTPS');
const minimum=Number(process.env.INFRA_QA_MIN_DEVICES||1);
const redact=e=>String(e.stack||e).split(credentials.password).join('[redacted]');
(async()=>{
 const browser=await chromium.launch({headless:true});let session=null,device=null,page;
 try{
  const context=await browser.newContext({viewport:{width:1280,height:900}});page=await context.newPage();
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto(base,{waitUntil:'domcontentloaded'});
  await page.locator('#login-submit:not([disabled])').waitFor();
  assert.equal(await page.locator('#app-shell').isVisible(),false);
  await page.locator('#login-username').fill(credentials.username);
  await page.locator('#login-password').fill(credentials.password);
  await page.locator('#login-submit').click();
  await page.locator('#app-shell:not(.hidden)').waitFor();
  await page.locator('#overview-devices .device-row').first().waitFor();
  const response=await context.request.get(base+'/api/devices');assert.equal(response.status(),200);
  const devices=await response.json();assert(devices.length>=minimum);
  assert(devices.filter(d=>d.online).length>=minimum);
  assert.equal(await page.locator('#overview-devices .device-row').count(),devices.slice(0,5).length);
  const cookie=(await context.cookies()).find(c=>c.name==='infra_session');
  assert(cookie?.secure&&cookie.httpOnly&&cookie.sameSite==='Strict');
  assert(await page.locator('.brand-logo').evaluate(img=>img.complete&&img.naturalWidth>0));
  assert.equal((await context.request.get(base+'/favicon.ico')).status(),200);
  assert.equal((await context.request.get(base+'/mcp')).status(),404);
  device=devices.find(d=>d.online&&d.platform==='linux')||devices.find(d=>d.online);
  await page.locator(`#overview-devices [data-device="${device.device_id}"]`).click();
  await page.locator('[data-device-tab="terminal"]').click();
  await page.locator('#terminal-command').fill('cat');await page.locator('#terminal-open button').click();
  await page.waitForURL(/#session\//);session=new URL(page.url()).hash.split('/').at(-1);
  await page.locator('.xterm-screen').waitFor();
  const sessionUrl=page.url();const marker='infra-public-browser-acceptance-'+Date.now();
  await page.locator('.xterm-helper-textarea').focus();await page.keyboard.type(marker);await page.keyboard.press('Enter');
  await page.waitForFunction(marker=>document.querySelector('.xterm-screen')?.textContent.includes(marker),marker);
  await page.reload();await page.locator('.xterm-screen').waitFor();assert.equal(page.url(),sessionUrl);
  await page.waitForFunction(marker=>document.querySelector('.xterm-screen')?.textContent.includes(marker),marker);
  page.once('dialog',d=>d.accept());await page.locator('#session-stop').click();
  for(let i=0;i<30;i++){const active=await (await context.request.get(base+'/api/sessions')).json();if(!active.some(s=>s.id===session)){session=null;break;}await page.waitForTimeout(200);}
  assert.equal(session,null,'closed terminal did not reconcile');
  await page.locator('[data-view="overview"]').click();
  if(process.env.INFRA_QA_SCREENSHOT)await page.screenshot({path:process.env.INFRA_QA_SCREENSHOT,fullPage:true});
  await page.locator('[data-view="settings"]').click();assert(await page.locator('#password-change').isVisible());
  await page.locator('#logout-session').click();await page.locator('#signin:not(.hidden)').waitFor();
  assert.equal(await page.locator('#app-shell').isVisible(),false);
  assert.equal((await context.request.get(base+'/api/devices')).status(),401);
  assert.deepEqual(errors,[]);
  console.log('LIVE_PUBLIC_BROWSER_PASS '+JSON.stringify({devices:devices.map(d=>({name:d.device_name,online:d.online})),https:true,secureHttpOnlyCookie:true,logoAndFavicon:true,samePtyAfterRefresh:true,signout:true}));
 }catch(e){if(page&&process.env.INFRA_QA_SCREENSHOT)await page.screenshot({path:process.env.INFRA_QA_SCREENSHOT+'.failed.png',fullPage:true}).catch(()=>{});throw new Error(redact(e));}
 finally{
  if(session&&device&&page){try{await page.context().request.post(base+'/api/devices/'+device.device_id+'/operations',{headers:{'X-Infra-CSRF':'1'},data:{op:'terminal',action:{action:'close',session_id:session}}});}catch{}}
  await browser.close();
 }
})().catch(e=>{console.error(redact(e));process.exitCode=1;});
