import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';
import WebSocket from 'ws';

const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const work=await fs.mkdtemp(path.join(os.tmpdir(),'infra-test-'));
const port=Number(process.env.INFRA_TEST_PORT||18787),base=`http://127.0.0.1:${port}`;
const password=crypto.randomBytes(16).toString('hex'),secret=crypto.randomBytes(32).toString('hex');
let server,agent,cookie='',client;let output='';
const env={...process.env,PORT:String(port),PUBLIC_BASE:base,OAUTH_PASSWORD:password,TOKEN_SIGNING_SECRET:secret,AGENT_TOKEN:crypto.randomBytes(16).toString('hex'),INFRA_DATA_DIR:path.join(work,'data'),OAUTH_CLIENTS_FILE:path.join(work,'clients.json')};
function startServer(){server=spawn(process.execPath,[path.join(root,'server/index.mjs')],{env,stdio:['ignore','pipe','pipe']});server.stdout.on('data',x=>output+=x);server.stderr.on('data',x=>output+=x);}
async function until(fn,timeout=10000){const start=Date.now();while(Date.now()-start<timeout){try{const value=await fn();if(value)return value;}catch{}await new Promise(r=>setTimeout(r,100));}throw new Error('Timed out. '+output.slice(-1500));}
async function req(url,body,method=body?'POST':'GET',session=true,origin=base){return fetch(base+url,{method,headers:{'Content-Type':'application/json',...(session?{Cookie:cookie}:{}),...(origin?{Origin:origin}:{})},...(body?{body:JSON.stringify(body)}:{})});}
async function api(url,body,method){const r=await req(url,body,method);const d=await r.json();assert.equal(r.status,200,JSON.stringify(d));return d;}
async function stop(child){if(!child||child.exitCode!==null)return;child.kill('SIGTERM');await until(()=>child.exitCode!==null||child.signalCode!==null);}
const call=async(name,args)=>{const r=await client.callTool({name,arguments:args});assert.ok(!r.isError,JSON.stringify(r));return JSON.parse(r.content[0].text);};

await test('Infra MCP full local journey',async t=>{
  try{
    startServer();await until(async()=>{const r=await fetch(base+'/health');return r.ok;});
    await t.test('domain landing and assets work; unknown downloads do not expose files',async()=>{
      assert.match(await (await fetch(base+'/')).text(),/Your devices/);
      assert.equal((await fetch(base+'/ui/app.js')).status,200);
      assert.equal((await fetch(base+'/downloads/nope')).status,404);
      assert.equal((await req('/api/overview',undefined,'GET',false)).status,401);
    });
    await t.test('login, secure cookie properties and cross-origin rejection',async()=>{
      assert.equal((await req('/api/session',{password:'wrong'})).status,401);
      assert.equal((await req('/api/session',{password},'POST',false,'https://evil.example')).status,403);
      const r=await req('/api/session',{password});assert.equal(r.status,200);
      const header=r.headers.get('set-cookie');assert.match(header,/HttpOnly/i);assert.match(header,/SameSite=Strict/i);cookie=header.split(';')[0];
      assert.equal((await api('/api/session')).authenticated,true);
    });
    let paired;
    await t.test('pairing is single-use, validates names, and keeps credentials private',async()=>{
      const p=await api('/api/pairing',{});
      assert.equal((await req('/pair',{code:p.code,device:'bad name'},'POST',false)).status,400);
      paired=await api('/pair',{code:p.code,device:'testdevice',platform:'linux'});
      assert.ok(paired.token);
      assert.equal((await req('/pair',{code:p.code,device:'another'},'POST',false)).status,400);
      const overview=await api('/api/overview');assert.ok(!JSON.stringify(overview).includes(paired.token));assert.ok(!('tokenHash' in overview.devices[0]));
    });
    const agentDir=path.join(work,'agent');await fs.mkdir(agentDir);
    for(const [src,dest] of [['index.mjs','agent.mjs'],['jobs.mjs','jobs.mjs']])await fs.copyFile(path.join(root,'agent',src),path.join(agentDir,dest));
    await fs.writeFile(path.join(agentDir,'config.json'),JSON.stringify({...paired,startup:'test'}));
    function startAgent(){agent=spawn(process.execPath,[path.join(agentDir,'agent.mjs')],{cwd:work,env:{...process.env,CONTROL_SERVER:paired.server,AGENT_TOKEN:paired.token,DEVICE_NAME:'testdevice'},stdio:['ignore','pipe','pipe']});agent.stderr.on('data',x=>output+=x);agent.stdout.on('data',x=>output+=x);}
    startAgent();await until(async()=> (await api('/api/overview')).devices.some(d=>d.device==='testdevice'&&d.online));
    await t.test('OAuth DCR, PKCE and MCP discovery; dashboard cookies cannot call MCP',async()=>{
      const reg=await req('/register',{redirect_uris:['https://chatgpt.com/connector/oauth/test'],client_name:'test'},'POST',false);assert.equal(reg.status,201);const c=await reg.json();
      const verifier=crypto.randomBytes(32).toString('base64url');const challenge=crypto.createHash('sha256').update(verifier).digest('base64url');
      const form=new URLSearchParams({password,client_id:c.client_id,redirect_uri:c.redirect_uris[0],code_challenge:challenge,resource:base+'/mcp',scope:'device:control',state:'test-state'});
      const ar=await fetch(base+'/authorize',{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},body:form,redirect:'manual'});assert.equal(ar.status,302);
      const location=new URL(ar.headers.get('location'));assert.equal(location.searchParams.get('state'),'test-state');
      const tr=await req('/token',{grant_type:'authorization_code',code:location.searchParams.get('code'),client_id:c.client_id,redirect_uri:c.redirect_uris[0],code_verifier:verifier,resource:base+'/mcp'},'POST',false);assert.equal(tr.status,200);const token=await tr.json();
      assert.ok(token.refresh_token);
      const refresh=await req('/token',{grant_type:'refresh_token',client_id:c.client_id,refresh_token:token.refresh_token},'POST',false);assert.equal(refresh.status,200);
      const wrongClient=await req('/token',{grant_type:'refresh_token',client_id:'wrong',refresh_token:token.refresh_token},'POST',false);assert.equal(wrongClient.status,400);
      const refreshAsAccess=await fetch(base+'/mcp',{method:'POST',headers:{Authorization:'Bearer '+token.refresh_token,'Content-Type':'application/json'},body:'{}'});assert.equal(refreshAsAccess.status,401);
      client=new Client({name:'integration',version:'1'});await client.connect(new StreamableHTTPClientTransport(new URL(base+'/mcp'),{requestInit:{headers:{Authorization:'Bearer '+token.access_token}}}));
      assert.deepEqual((await client.listTools()).tools.map(x=>x.name),['devices','shell','files','process','jobs','status']);
      const dashboardToken=decodeURIComponent(cookie.split('=')[1]);const forbidden=await fetch(base+'/mcp',{method:'POST',headers:{Authorization:'Bearer '+dashboardToken,'Content-Type':'application/json'},body:'{}'});assert.equal(forbidden.status,401);
    });
    await t.test('commands, nonzero exit and filesystem CRUD through MCP',async()=>{
      const r=await call('shell',{device:'testdevice',command:'echo infra-ok'});assert.equal(r.stdout.trim(),'infra-ok');
      const bad=await call('shell',{device:'testdevice',command:'exit 7'});assert.equal(bad.exitCode,7);
      const f=path.join(work,'file.txt'),dest=path.join(work,'moved.txt');await call('files',{device:'testdevice',action:'write',path:f,content:'first'});await call('files',{device:'testdevice',action:'append',path:f,content:' second'});assert.equal((await call('files',{device:'testdevice',action:'read',path:f})).content,'first second');
      await call('files',{device:'testdevice',action:'move',path:f,destination:dest});await call('files',{device:'testdevice',action:'delete',path:dest});assert.equal((await call('status',{device:'testdevice'})).version,'0.3.0');
    });
    await t.test('a background job retains logs and completes across agent restart',async()=>{
      const j=await call('shell',{device:'testdevice',command:`node -e "process.stdout.write('first');setTimeout(()=>{process.stdout.write('second');process.exit(3)},2000)"`,background:true});assert.ok(j.jobId);
      await until(async()=> (await call('jobs',{device:'testdevice',action:'read',id:j.jobId})).runnerPid);
      await stop(agent);startAgent();await until(async()=> (await api('/api/overview')).devices.some(d=>d.online));
      const done=await until(async()=>{const r=await call('jobs',{device:'testdevice',action:'read',id:j.jobId});return r.status==='completed'&&r;});assert.equal(done.output,'firstsecond');assert.equal(done.exitCode,3);
      const page=await call('jobs',{device:'testdevice',action:'read',id:j.jobId,offset:5,limit:100});assert.equal(page.output,'second');
    });
    await t.test('cancellation and duplicate local agent prevention',async()=>{
      const duplicate=spawn(process.execPath,[path.join(agentDir,'agent.mjs')],{stdio:'ignore'});await until(()=>duplicate.exitCode!==null);assert.equal(duplicate.exitCode,0);
      const j=await call('shell',{device:'testdevice',command:'node -e "setInterval(()=>{},1000)"',background:true});await until(async()=> (await call('jobs',{device:'testdevice',action:'read',id:j.jobId})).runnerPid);
      assert.equal((await call('jobs',{device:'testdevice',action:'cancel',id:j.jobId})).status,'canceled');
    });
    await t.test('gateway restart restores paired identity and dashboard session',async()=>{
      await client.close();await stop(server);startServer();await until(async()=> (await fetch(base+'/health')).ok);
      assert.equal((await api('/api/session')).authenticated,true);
      await until(async()=> (await api('/api/overview')).devices.some(d=>d.device==='testdevice'&&d.online));
    });
    await t.test('credentials are scoped to a device and revocation persists',async()=>{
      const ws=new WebSocket(paired.server);await new Promise(r=>ws.once('open',r));ws.send(JSON.stringify({type:'hello',device:'imposter',token:paired.token}));const code=await new Promise(r=>ws.once('close',r));assert.equal(code,4003);
      await api('/api/devices/testdevice',undefined,'DELETE');await until(async()=> !(await api('/api/overview')).devices.find(d=>d.device==='testdevice').online);
      assert.equal((await req('/agent-status',{device:'testdevice',token:paired.token},'POST',false)).status,401);
    });
  }finally{await client?.close().catch(()=>{});await stop(agent);await stop(server);await fs.rm(work,{recursive:true,force:true});}
});
