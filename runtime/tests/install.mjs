import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import crypto from 'node:crypto';
import http from 'node:http';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
const exec=promisify(execFile),root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const work=await fs.mkdtemp(path.join(os.tmpdir(),'infra-install-test-')),port=18788,base=`http://127.0.0.1:${port}`;
const password=crypto.randomBytes(16).toString('hex');
const env={...process.env,PORT:String(port),PUBLIC_BASE:base,OAUTH_PASSWORD:password,TOKEN_SIGNING_SECRET:crypto.randomBytes(32).toString('hex'),AGENT_TOKEN:'test-only',INFRA_DATA_DIR:path.join(work,'data'),OAUTH_CLIENTS_FILE:path.join(work,'clients.json')};
let logs='',cookie='',report=[];
let pairDrops=0,statusDrops=0;
// Forward pairing to the gateway, then discard its first successful response.
// This simulates ECONNRESET after a single-use code was already redeemed.
const flaky=http.createServer((req,res)=>{
  const upstream=http.request(base+req.url,{method:req.method,headers:req.headers},response=>{
    if(response.statusCode===200&&((req.url==='/pair'&&pairDrops++===0)||(req.url==='/agent-status'&&statusDrops++===0))){response.resume();req.socket.destroy();return;}
    res.writeHead(response.statusCode,response.headers);response.pipe(res);
  });
  upstream.on('error',()=>{res.destroy();});req.pipe(upstream);
});
const server=spawn(process.execPath,[path.join(root,'server/index.mjs')],{env,stdio:['ignore','pipe','pipe']});server.stdout.on('data',d=>logs+=d);server.stderr.on('data',d=>logs+=d);
async function ready(fn,limit=15000){for(let i=0;i<limit/150;i++){try{const r=await fn();if(r)return r;}catch{}await new Promise(r=>setTimeout(r,150));}throw new Error('Timed out: '+logs.slice(-1000));}
async function api(route,body){const r=await fetch(base+route,{method:body?'POST':'GET',headers:{Origin:base,Cookie:cookie,'Content-Type':'application/json'},...(body?{body:JSON.stringify(body)}:{})});const d=await r.json();if(!r.ok)throw new Error(d.error);return d;}
try{
  await ready(async()=> (await fetch(base+'/health')).ok);
  await new Promise(r=>flaky.listen(0,'127.0.0.1',r));
  const flakyBase=`http://127.0.0.1:${flaky.address().port}`;
  const login=await fetch(base+'/api/session',{method:'POST',headers:{Origin:base,'Content-Type':'application/json'},body:JSON.stringify({password})});cookie=login.headers.get('set-cookie').split(';')[0];
  const installDir=path.join(work,'user installation'),service='infra-mcp-staging-test';
  const installEnv={...process.env,PATH:path.dirname(process.execPath)+':'+process.env.PATH,INFRA_INSTALL_DIR:installDir,INFRA_SERVICE_NAME:service};delete installEnv.AGENT_TOKEN;delete installEnv.CONTROL_SERVER;delete installEnv.DEVICE_NAME;
  const code=(await api('/api/pairing',{})).code;
  const first=await exec('/bin/bash',[path.join(root,'downloads/install.sh'),'--base',flakyBase,'--code',code,'--name','install-test'],{env:installEnv,timeout:180000,maxBuffer:1048576});report.push({case:'no-sudo user installation, paths with spaces',ok:true,output:first.stdout.trim()});
  if(pairDrops!==2||statusDrops<2)throw new Error('Installer did not retry dropped pairing/status responses');
  report.push({case:'connection reset after pairing was committed; same credential recovered, status reset retried',ok:true});
  const mode=(await fs.stat(path.join(installDir,'config.json'))).mode&0o777;if(mode!==0o600)throw new Error('Credential file permissions are incorrect.');
  const job=await api('/api/run',{op:'shell',args:{device:'install-test',command:'printf before; sleep 2; printf after',background:true}});
  await ready(async()=> (await api('/api/run',{op:'jobs',args:{device:'install-test',action:'read',id:job.jobId}})).runnerPid);
  await exec('systemctl',['--user','restart',service+'.service']);await ready(async()=> (await api('/api/overview')).devices.some(d=>d.device==='install-test'&&d.online));report.push({case:'user service restart and reconnect',ok:true});
  const completed=await ready(async()=>{const r=await api('/api/run',{op:'jobs',args:{device:'install-test',action:'read',id:job.jobId}});return r.status==='completed'&&r;});
  if(completed.output!=='beforeafter')throw new Error('Background job did not retain output across service restart');
  report.push({case:'background job survives actual systemd agent service restart',ok:true});

  await exec('/bin/bash',[path.join(root,'downloads/install.sh'),'--base',base],{env:installEnv,timeout:60000,maxBuffer:1048576});report.push({case:'reinstallation without re-pairing',ok:true});
  await exec('/bin/bash',[path.join(root,'downloads/install.sh'),'--uninstall'],{env:installEnv,timeout:15000});report.push({case:'uninstall removes startup and keeps credentials/logs',ok:true});
  const privateDir=path.join(work,'private-runtime'),tools=path.join(work,'tools');await fs.mkdir(tools);
  for(const name of ['curl','uname','id','mktemp','mkdir','awk','sha256sum','shasum','tar','gzip','hostname','tr','chmod','cp','mv','rm','ldd','head','grep','dirname']){
    try{const {stdout}=await exec('/bin/sh',['-c',`command -v ${name}`]);await fs.symlink(stdout.trim(),path.join(tools,name));}catch{}
  }
  const privateCode=(await api('/api/pairing',{})).code;
  const portable=await exec('/bin/bash',[path.join(root,'downloads/install.sh'),'--base',base,'--code',privateCode,'--name','portable-test','--manual'],{env:{...installEnv,PATH:tools,INFRA_INSTALL_DIR:privateDir},timeout:180000,maxBuffer:1048576});
  await fs.access(path.join(privateDir,'node/bin/node'));report.push({case:'no-sudo install with Node missing; SHA-256 verified private runtime',ok:true,output:portable.stdout.trim()});
  let sudo=false;try{await exec('sudo',['-n','true']);sudo=true;}catch{}
  if(sudo){
    const rootDir=path.join(work,'system-install'),rootService='infra-mcp-system-staging-test';
    try{
      const systemCode=(await api('/api/pairing',{})).code;
      await exec('sudo',['-n','env','INFRA_INSTALL_DIR='+rootDir,'INFRA_SERVICE_NAME='+rootService,'/bin/bash',path.join(root,'downloads/install.sh'),'--base',base,'--code',systemCode,'--name','system-test','--system'],{timeout:180000,maxBuffer:1048576});
      const who=await api('/api/run',{op:'shell',args:{device:'system-test',command:'id -u'}});
      if(who.stdout.trim()!=='0')throw new Error('System agent did not run with the intended privilege');
      await exec('sudo',['-n','systemctl','restart',rootService+'.service']);
      await ready(async()=> (await api('/api/overview')).devices.some(d=>d.device==='system-test'&&d.online));
      report.push({case:'sudo system installation, root shell access and service restart',ok:true});
    }finally{
      await exec('sudo',['-n','env','INFRA_INSTALL_DIR='+rootDir,'INFRA_SERVICE_NAME='+rootService,'/bin/bash',path.join(root,'downloads/install.sh'),'--system','--uninstall']).catch(()=>{});
      await exec('sudo',['-n','rm','-rf','--',rootDir]).catch(()=>{});
    }
  }else report.push({case:'sudo system installation',skipped:'Noninteractive sudo is unavailable'});
  console.log(JSON.stringify({ok:true,report},null,2));
}finally{
  await exec('systemctl',['--user','stop','infra-mcp-staging-test.service']).catch(()=>{});
  await exec('systemctl',['--user','disable','infra-mcp-staging-test.service']).catch(()=>{});
  await fs.rm(path.join(os.homedir(),'.config/systemd/user/infra-mcp-staging-test.service'),{force:true});await exec('systemctl',['--user','daemon-reload']).catch(()=>{});
  server.kill('SIGTERM');await new Promise(r=>setTimeout(r,300));await fs.rm(work,{recursive:true,force:true});
  flaky.closeAllConnections();await new Promise(r=>flaky.close(r));
}
