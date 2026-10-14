import fs from 'node:fs/promises';
import path from 'node:path';
import crypto from 'node:crypto';
import os from 'node:os';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { createRequire } from 'node:module';
import assert from 'node:assert/strict';
const require=createRequire(import.meta.url),{chromium}=require('playwright'),exec=promisify(execFile);
const config={};
for(const line of (await fs.readFile(process.env.INFRA_LIVE_ENV,'utf8')).split(/\r?\n/)){
  const match=line.match(/^(?:export\s+)?([A-Z_]+)=(.*)$/);if(!match)continue;
  let value=match[2];if((value.startsWith('"')&&value.endsWith('"'))||(value.startsWith("'")&&value.endsWith("'")))value=value.slice(1,-1);
  config[match[1]]=value;
}
const base=config.PUBLIC_BASE||'https://mcp.phloraxx.us.to',captures=process.env.INFRA_LIVE_CAPTURES||'/tmp/infra-live-captures';
await fs.mkdir(captures,{recursive:true,mode:0o700});
let cookie='',browser;
async function api(route,body){const r=await fetch(base+route,{method:body?'POST':'GET',headers:{Origin:base,Cookie:cookie,'Content-Type':'application/json'},...(body?{body:JSON.stringify(body)}:{})});const data=await r.json();if(!r.ok)throw new Error(`HTTP ${r.status}: ${data.error}`);return data;}
const run=(op,args)=>api('/api/run',{op,args:{device:'oracle',...args}});
async function until(fn){for(let i=0;i<100;i++){try{const v=await fn();if(v)return v;}catch{}await new Promise(r=>setTimeout(r,150));}throw new Error('Live check timed out');}
const temp=`/tmp/infra-live-${crypto.randomUUID()}`;
try{
  const login=await fetch(base+'/api/session',{method:'POST',headers:{Origin:base,'Content-Type':'application/json'},body:JSON.stringify({password:config.OAUTH_PASSWORD})});assert.equal(login.status,200);cookie=login.headers.get('set-cookie').split(';')[0];assert.match(login.headers.get('set-cookie'),/Secure/);
  const overview=await api('/api/overview');assert.ok(overview.devices.find(d=>d.device==='oracle'&&d.online));
  browser=await chromium.launch({headless:true,executablePath:process.env.INFRA_CHROMIUM,args:['--no-sandbox']});
  const page=await browser.newPage({viewport:{width:1440,height:1000}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto(base);await page.locator('#login').waitFor({state:'visible'});await page.locator('#password').fill(config.OAUTH_PASSWORD);await page.getByRole('button',{name:'Sign in',exact:true}).click();await page.locator('#dashboard').waitFor({state:'visible'});await page.waitForFunction(()=>document.getElementById('device-title').textContent==='Oracle');
  await page.locator('#command').fill('echo UI_LIVE_OK');await page.getByRole('button',{name:'Run',exact:true}).click();await page.waitForFunction(()=>document.getElementById('console-output').textContent.includes('[exit 0]'));assert.match(await page.locator('#console-output').innerText(),/UI_LIVE_OK/);
  await page.screenshot({path:path.join(captures,'live-desktop.png')});
  await run('files',{action:'write',path:temp+'/test.txt',content:'before'});
  await page.getByRole('tab',{name:'Files',exact:true}).click();await page.locator('#file-path').fill(temp);await page.getByRole('button',{name:'Open directory'}).click();await page.getByRole('button',{name:/test.txt/}).click();await page.locator('#file-content').fill('after');await page.getByRole('button',{name:'Save file'}).click();await page.getByText('File saved.',{exact:true}).waitFor();assert.equal((await run('files',{action:'read',path:temp+'/test.txt'})).content,'after');
  const job=await run('shell',{command:'echo JOB_BEFORE; sleep 2; echo JOB_AFTER',background:true});await until(async()=> (await run('jobs',{action:'read',id:job.jobId})).runnerPid);
  const serviceEnv={...process.env,XDG_RUNTIME_DIR:`/run/user/${os.userInfo().uid}`,DBUS_SESSION_BUS_ADDRESS:`unix:path=/run/user/${os.userInfo().uid}/bus`};
  await exec('systemctl',['--user','restart','chatgpt-control-agent.service'],{env:serviceEnv});
  const done=await until(async()=>{const r=await run('jobs',{action:'read',id:job.jobId});return r.status==='completed'&&r;});assert.match(done.output,/JOB_BEFORE/);assert.match(done.output,/JOB_AFTER/);assert.equal(done.exitCode,0);
  await page.getByRole('tab',{name:'Console',exact:true}).click();await page.setViewportSize({width:390,height:844});await page.screenshot({path:path.join(captures,'live-mobile.png')});assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);assert.deepEqual(errors,[]);
  const status=await run('status',{});
  const samples=[];for(let i=0;i<30;i++){const start=performance.now();const r=await run('shell',{command:'echo BENCH_OK'});assert.equal(r.exitCode,0);samples.push(performance.now()-start);}samples.sort((a,b)=>a-b);
  console.log(JSON.stringify({ok:true,domain:base,devices:overview.devices.map(d=>({device:d.device,online:d.online,version:d.version})),checks:['live HTTPS sign-in','Secure session cookie','real browser Oracle command','real browser file edit','job retained across live agent service restart','mobile layout','no JavaScript errors'],metrics:{agentRssMB:Math.round(status.memory/1048576),gatewayRssMB:Math.round(overview.server.memory/1048576),domainHttpShellMedianMs:Math.round(samples[15]),domainHttpShellP95Ms:Math.round(samples[28]),samples:30},captures},null,2));
}finally{await run('files',{action:'delete',path:temp,recursive:true}).catch(()=>{});await browser?.close();}
