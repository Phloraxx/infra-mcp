import http from 'node:http';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { installProduct } from './product.mjs';
import express from 'express';
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/streamableHttp.js';
import { createMcpExpressApp } from '@modelcontextprotocol/sdk/server/express.js';
import { WebSocketServer } from 'ws';
import * as z from 'zod/v4';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const DATA_DIR = process.env.INFRA_DATA_DIR || path.join(ROOT, 'data');
fs.mkdirSync(DATA_DIR, {recursive:true, mode:0o700});
const PORT = Number(process.env.PORT || 8787);
const PUBLIC_BASE = process.env.PUBLIC_BASE || 'https://mcp.phloraxx.us.to';
const OAUTH_USER = process.env.OAUTH_USER || 'sourav';
const OAUTH_PASSWORD = process.env.OAUTH_PASSWORD || '';
const TOKEN_SECRET = process.env.TOKEN_SIGNING_SECRET || '';
const AGENT_TOKEN = process.env.AGENT_TOKEN || '';
const CLIENTS_FILE = process.env.OAUTH_CLIENTS_FILE || path.join(ROOT, 'oauth-clients.json');
const agents = new Map();
const pending = new Map();
const codes = new Map();
let clients = {};
try { clients = JSON.parse(fs.readFileSync(CLIENTS_FILE, 'utf8')); } catch {}

const json = v => ({ content: [{ type: 'text', text: JSON.stringify(v) }] });
const b64 = x => Buffer.from(x).toString('base64url');
function signToken(payload) {
  const body = b64(JSON.stringify(payload));
  const sig = crypto.createHmac('sha256', TOKEN_SECRET).update(body).digest('base64url');
  return `${body}.${sig}`;
}
function verifyToken(token) {
  if (!TOKEN_SECRET || !token?.includes('.')) return null;
  const parts = token.split('.');
  if(parts.length !== 2) return null;
  const [body, sig] = parts;
  const want = crypto.createHmac('sha256', TOKEN_SECRET).update(body).digest('base64url');
  if (!/^[A-Za-z0-9_-]{43}$/.test(sig) || sig.length !== want.length || !crypto.timingSafeEqual(Buffer.from(sig), Buffer.from(want))) return null;
  try {
    const p = JSON.parse(Buffer.from(body, 'base64url').toString());
    if (!p.exp || p.exp < Math.floor(Date.now()/1000)) return null;
    if (p.aud !== `${PUBLIC_BASE}/mcp` || p.exp <= Math.floor(Date.now()/1000)) return null;
    return p;
  } catch { return null; }
}
function saveClients() { fs.writeFileSync(CLIENTS_FILE, JSON.stringify(clients, null, 2)); }
function requireAgent(device) {
  const a = agents.get(device);
  if (!a || a.ws.readyState !== 1) throw new Error(`Device '${device}' is not online`);
  return a;
}
function rpc(device, op, args={}, timeoutMs=30000) {
  const a = requireAgent(device), id = crypto.randomUUID();
  return new Promise((resolve,reject) => {
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Timed out waiting for ${device} (${op})`)); }, timeoutMs);
    pending.set(id,{resolve,reject,timer,device,ws:a.ws});
    a.ws.send(JSON.stringify({type:'request',id,op,args}));
  });
}
function makeServer() {
  const s = new McpServer({name:'infra-mcp',version:'0.3.0'});
  s.registerTool('devices',{description:'List connected personal devices or inspect one device.',inputSchema:{action:z.enum(['list','info']).default('list'),device:z.string().optional()},annotations:{readOnlyHint:true}},async({action,device})=>{
    if(action==='info'){if(!device) throw new Error('device is required'); const a=product.list().find(x=>x.device===device&&!x.revoked);return json(a||{online:false,device});}
    return json(product.list().filter(x=>!x.revoked));
  });
  s.registerTool('shell',{description:'Run CMD, PowerShell, bash, zsh, git, Docker, ADB or other shell commands on a connected device.',inputSchema:schemas.shell.shape,annotations:{readOnlyHint:false,destructiveHint:true,openWorldHint:true}},async a=>json(await rpc(a.device,'shell',a,(a.timeoutMs||30000)+5000)));
  s.registerTool('files',{description:'Read, write, append, list, search, stat, copy, move, mkdir or delete files/directories on a connected device.',inputSchema:schemas.files.shape,annotations:{readOnlyHint:false,destructiveHint:true}},async a=>json(await rpc(a.device,'files',a,60000)));
  s.registerTool('process',{description:'List, find or kill processes on a connected device.',inputSchema:schemas.process.shape,annotations:{readOnlyHint:false,destructiveHint:true}},async a=>json(await rpc(a.device,'process',a,30000)));
  s.registerTool('jobs',{description:'List saved background jobs, read output with a cursor, or cancel a job. Background shell returns a job ID. Never resubmit a command just because a connection timed out.',inputSchema:schemas.jobs.shape,annotations:{readOnlyHint:false,destructiveHint:true}},async a=>json(await rpc(a.device,'jobs',a,30000)));
  s.registerTool('status',{description:'Inspect device runtime, memory, startup mode and available capabilities.',inputSchema:schemas.status.shape,annotations:{readOnlyHint:true}},async a=>json(await rpc(a.device,'status',a,10000)));
  return s;
}

const MCP_RESOURCE = `${PUBLIC_BASE}/mcp`;
const isChatGptRedirect = (u) => {
  try {
    const x = new URL(String(u));
    return x.protocol === 'https:' && x.hostname === 'chatgpt.com' && (x.pathname === '/connector_platform_oauth_redirect' || x.pathname.startsWith('/connector/oauth/'));
  } catch { return false; }
};
const validResource = (r) => !r || r === PUBLIC_BASE || r === MCP_RESOURCE;
const app = createMcpExpressApp({ host: '127.0.0.1', allowedHosts: ['127.0.0.1', 'localhost', new URL(PUBLIC_BASE).hostname] });
app.use(express.json({ limit:'1mb' }));
app.use(express.urlencoded({ extended: false, limit:'32kb' }));
app.use((req,res,next)=>{ if(req.method==='POST' && (req.path==='/token'||req.path==='/register'||req.path==='/authorize')) return next(); next(); });
app.get('/health',(_q,r)=>r.json({ok:true,version:'0.3.0',oauth:true}));
app.get('/downloads/manifest.json',(_q,r)=>r.json({version:'0.3.0',nodeVersion:'v22.23.3',files:Object.fromEntries(['agent.mjs','jobs.mjs'].map(name=>[name,crypto.createHash('sha256').update(fs.readFileSync(path.join(ROOT,'agent',name==='agent.mjs'?'index.mjs':name))).digest('hex')]))}));
app.get('/downloads/:file', (q,r)=>{
  const allowed = {'agent.mjs':path.join(ROOT,'agent/index.mjs'),'jobs.mjs':path.join(ROOT,'agent/jobs.mjs'),'install.sh':path.join(ROOT,'downloads/install.sh'),'install-macos.sh':path.join(ROOT,'downloads/install.sh'),'install-linux.sh':path.join(ROOT,'downloads/install.sh'),'install-windows.ps1':path.join(ROOT,'downloads/install-windows.ps1')};
  if(!allowed[q.params.file])return r.sendStatus(404);
  r.set('Cache-Control','no-store').sendFile(allowed[q.params.file]);
});
const schemas = {
  shell:z.object({device:z.string(),command:z.string().min(1).max(100000),cwd:z.string().optional(),shell:z.string().optional(),timeoutMs:z.number().int().min(1000).max(120000).optional(),background:z.boolean().optional(),outputLimit:z.number().int().min(100).max(100000).optional()}),
  files:z.object({device:z.string(),action:z.enum(['read','write','append','list','search','stat','copy','move','mkdir','delete']),path:z.string().min(1),destination:z.string().optional(),content:z.string().optional(),query:z.string().optional(),recursive:z.boolean().optional(),maxResults:z.number().int().min(1).max(500).optional()}),
  process:z.object({device:z.string(),action:z.enum(['list','find','kill']),pid:z.number().int().positive().optional(),name:z.string().optional()}),
  jobs:z.object({device:z.string(),action:z.enum(['list','read','cancel']),id:z.string().optional(),offset:z.number().int().min(0).optional(),limit:z.number().int().min(100).max(100000).optional()}),
  status:z.object({device:z.string()})
};
const product = installProduct({app,agents,rpc,signToken,verifyToken,base:PUBLIC_BASE,password:OAUTH_PASSWORD,user:OAUTH_USER,dataDir:DATA_DIR,publicDir:path.join(ROOT,'public'),validate:(op,args)=>{if(!schemas[op])throw new Error('Unsupported operation');return schemas[op].parse(args);}});

const protectedResourceMetadata = (_q,r)=>r.json({resource:MCP_RESOURCE,authorization_servers:[PUBLIC_BASE],scopes_supported:['device:control']});
app.get('/.well-known/oauth-protected-resource', protectedResourceMetadata);
app.get('/.well-known/oauth-protected-resource/mcp', protectedResourceMetadata);
app.get('/.well-known/oauth-authorization-server',(_q,r)=>r.json({issuer:PUBLIC_BASE,authorization_endpoint:`${PUBLIC_BASE}/authorize`,token_endpoint:`${PUBLIC_BASE}/token`,registration_endpoint:`${PUBLIC_BASE}/register`,authorization_response_iss_parameter_supported:true,code_challenge_methods_supported:['S256'],token_endpoint_auth_methods_supported:['none'],scopes_supported:['device:control'],response_types_supported:['code'],grant_types_supported:['authorization_code','refresh_token']}));

app.post('/register',(req,res)=>{
  const redirect_uris = Array.isArray(req.body?.redirect_uris)?req.body.redirect_uris:[];
  if(!redirect_uris.length || !redirect_uris.every(isChatGptRedirect)) return res.status(400).json({error:'invalid_redirect_uris'});
  const client_id='c_'+crypto.randomBytes(16).toString('hex');
  clients[client_id]={redirect_uris,client_name:req.body?.client_name||'ChatGPT',created:Date.now()}; saveClients();
  res.status(201).json({client_id,redirect_uris,token_endpoint_auth_method:'none',grant_types:['authorization_code','refresh_token'],response_types:['code']});
});

app.get('/authorize',(req,res)=>{
  const {client_id,redirect_uri,response_type,state,code_challenge,code_challenge_method,resource,scope='device:control'}=req.query;
  const c=clients[client_id];
  const redirectOk = c ? c.redirect_uris.includes(String(redirect_uri)) : isChatGptRedirect(redirect_uri);
  if(!client_id || !redirectOk || response_type!=='code' || code_challenge_method!=='S256' || !code_challenge || !validResource(resource)) {
    return res.status(400).send('Invalid OAuth request');
  }
  const esc=v=>String(v||'').replaceAll('&','&amp;').replaceAll('"','&quot;').replaceAll('<','&lt;');
  res.type('html').send(`<!doctype html><meta name="viewport" content="width=device-width"><title>Phloraxx MCP</title><style>body{font-family:system-ui;max-width:420px;margin:12vh auto;padding:24px}input,button{width:100%;box-sizing:border-box;padding:12px;margin:8px 0}button{cursor:pointer}</style><h2>Infra MCP</h2><p>Sign in to allow ChatGPT to control your connected devices.</p><form method="post" action="/authorize"><p><strong>User:</strong> sourav</p><input name="password" type="password" placeholder="Password" required>${Object.entries({client_id,redirect_uri,state,code_challenge,resource,scope}).map(([k,v])=>`<input type="hidden" name="${k}" value="${esc(v)}">`).join('')}<button>Authorize</button></form>`);
});
app.post('/authorize',(req,res)=>{
  const {password,client_id,redirect_uri,state,code_challenge,resource,scope='device:control'}=req.body||{};
  const c=clients[client_id];
  if(!OAUTH_PASSWORD || password!==OAUTH_PASSWORD) return res.status(401).type('html').send('<h2>Incorrect password</h2><p>Go back and try again.</p>');
  const redirectOk = c ? c.redirect_uris.includes(String(redirect_uri)) : isChatGptRedirect(redirect_uri);
  if(!client_id || !redirectOk || !code_challenge || !validResource(resource)) return res.status(400).send('Invalid OAuth request');
  const code=crypto.randomBytes(24).toString('base64url');
  codes.set(code,{client_id,redirect_uri,code_challenge,resource:resource||MCP_RESOURCE,scope,exp:Date.now()+5*60_000});
  const u=new URL(String(redirect_uri)); u.searchParams.set('code',code); if(state)u.searchParams.set('state',state); u.searchParams.set('iss',PUBLIC_BASE); res.redirect(u.toString());
});
function issueTokens(client_id, scope='device:control') {
  const now=Math.floor(Date.now()/1000);
  return {access_token:signToken({sub:OAUTH_USER,aud:MCP_RESOURCE,kind:'access',scope:'device:control',iat:now,exp:now+3600}),refresh_token:signToken({sub:OAUTH_USER,aud:MCP_RESOURCE,kind:'refresh',client_id,scope:'device:control',iat:now,exp:now+90*24*3600}),token_type:'Bearer',expires_in:3600,scope:'device:control'};
}
app.post('/token',(req,res)=>{
  const {grant_type,code,client_id,redirect_uri,code_verifier,resource,refresh_token}=req.body||{};
  if(grant_type==='refresh_token'){
    const p=verifyToken(refresh_token);
    if(!p||p.kind!=='refresh'||p.client_id!==client_id||p.sub!==OAUTH_USER||!validResource(resource))return res.status(400).json({error:'invalid_grant'});
    return res.json(issueTokens(client_id));
  }
  if(grant_type!=='authorization_code') return res.status(400).json({error:'unsupported_grant_type'});
  const c=codes.get(code); codes.delete(code);
  if(!c||c.exp<Date.now()||c.client_id!==client_id||c.redirect_uri!==redirect_uri||!validResource(resource)||!validResource(c.resource)) return res.status(400).json({error:'invalid_grant'});
  const challenge=crypto.createHash('sha256').update(String(code_verifier||'')).digest('base64url');
  if(challenge!==c.code_challenge) return res.status(400).json({error:'invalid_grant'});
  res.json(issueTokens(client_id));
});

function mcpAuth(req,res,next){
  const auth=req.headers.authorization||''; const token=auth.startsWith('Bearer ')?auth.slice(7):'';
  const payload=verifyToken(token);
  // Legacy tokens were minted with the client's requested scope. This single-owner gateway has one control grant.
  if(payload && (payload.kind===undefined || payload.kind==='access') && payload.sub===OAUTH_USER) return next();
  res.set('WWW-Authenticate',`Bearer resource_metadata="${PUBLIC_BASE}/.well-known/oauth-protected-resource/mcp", scope="device:control"`);
  return res.status(401).json({error:'unauthorized'});
}
app.post('/mcp',mcpAuth,async(req,res)=>{
  const server=makeServer();
  try{const t=new StreamableHTTPServerTransport({sessionIdGenerator:undefined}); res.on('close',()=>{t.close();server.close();}); await server.connect(t); await t.handleRequest(req,res,req.body);}
  catch(e){console.error(e); if(!res.headersSent)res.status(500).json({jsonrpc:'2.0',error:{code:-32603,message:String(e.message||e)},id:null});}
});
app.get('/mcp',mcpAuth,(_q,r)=>r.status(405).json({error:'POST required'}));
app.delete('/mcp',mcpAuth,(_q,r)=>r.status(405).json({error:'stateless server'}));

const httpServer=http.createServer(app),wss=new WebSocketServer({noServer:true,maxPayload:2*1024*1024});
httpServer.on('upgrade',(req,socket,head)=>{if(req.url!=='/agent')return socket.destroy();wss.handleUpgrade(req,socket,head,ws=>wss.emit('connection',ws,req));});
wss.on('connection',ws=>{
  ws.alive=true;ws.on('pong',()=>{ws.alive=true});
  let device=null; const timer=setTimeout(()=>ws.close(4001,'hello timeout'),5000);
  ws.on('message',raw=>{let m;try{m=JSON.parse(raw.toString())}catch{return}
    if(!device){if(m.type!=='hello'||!m.device||!product.authenticate(String(m.device),m.token,AGENT_TOKEN))return ws.close(4003,'unauthorized');clearTimeout(timer);device=String(m.device);const old=agents.get(device);if(old&&old.ws!==ws){ws.close(4000,'duplicate agent');return;}const meta={device,version:m.version||'0.2.0',home:m.home||null,startup:m.startup||'unknown',os:m.os||'unknown',hostname:m.hostname||device,arch:m.arch||'unknown',capabilities:m.capabilities||[],connectedAt:new Date().toISOString()};agents.set(device,{ws,meta});product.connected(meta);ws.send(JSON.stringify({type:'hello_ack'}));console.log('agent online:',device);return}
    if(m.type==='response'&&m.id){const p=pending.get(m.id);if(!p||p.ws!==ws)return;clearTimeout(p.timer);pending.delete(m.id);m.ok?p.resolve(m.result):p.reject(new Error(m.error||'agent operation failed'));}
  });
  ws.on('close',()=>{clearTimeout(timer);if(device&&agents.get(device)?.ws===ws){agents.delete(device);product.disconnected(device);for(const [id,p] of pending){if(p.ws===ws){clearTimeout(p.timer);pending.delete(id);p.reject(new Error('Device disconnected. Command outcome may be unknown; inspect jobs before retrying.'));}}}});
});
const heartbeat=setInterval(()=>{for(const ws of wss.clients){if(!ws.alive){ws.terminate();continue}ws.alive=false;ws.ping();}},30000);heartbeat.unref();
httpServer.listen(PORT,'127.0.0.1',()=>console.log(`Infra MCP v0.3.0 on 127.0.0.1:${PORT}`));

