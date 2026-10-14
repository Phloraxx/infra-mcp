import os from 'node:os';
import fs from 'node:fs/promises';
import fsSync from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { exec, spawn } from 'node:child_process';
import { promisify } from 'node:util';
import { createJobs, runJob } from './jobs.mjs';
let WebSocketImpl = globalThis.WebSocket;
if (!WebSocketImpl) {
  try { WebSocketImpl = (await import('ws')).default; }
  catch { throw new Error('WebSocket is unavailable. Install Node.js 22+ or run npm install ws@8 in the agent folder.'); }
}

const execAsync = promisify(exec);
const HERE = path.dirname(fileURLToPath(import.meta.url));
let localConfig = {};
try { localConfig = JSON.parse(fsSync.readFileSync(path.join(HERE, 'config.json'), 'utf8')); } catch {}
const SERVER = process.env.CONTROL_SERVER || localConfig.server || 'ws://127.0.0.1:8787/agent';
const TOKEN = process.env.AGENT_TOKEN || localConfig.token || '';
const DEVICE = process.env.DEVICE_NAME || localConfig.device || os.hostname();
const MAX_TEXT = 16000;
const STATE_DIR = process.env.INFRA_AGENT_DIR || HERE;
const jobs = createJobs(path.join(STATE_DIR,'jobs'),fileURLToPath(import.meta.url));

function shellPath(requested) {
  if (!requested || requested === 'auto') return true;
  if (process.platform === 'win32') {
    if (requested === 'powershell') return 'powershell.exe';
    if (requested === 'pwsh') return 'pwsh.exe';
    if (requested === 'cmd') return 'cmd.exe';
  }
  return requested;
}

async function shellOp(args) {
  const cwd = args.cwd || undefined;
  const sh = shellPath(args.shell);
  if (args.background) return jobs.start(args, sh);
  const maxText = Math.min(args.outputLimit || MAX_TEXT, 100000);
  const timeout = Math.min(Number(args.timeoutMs || 30000), 120000);
  try {
    const { stdout, stderr } = await execAsync(args.command, { cwd, shell: sh, timeout, maxBuffer: 8 * 1024 * 1024, windowsHide: true });
    return {
      exitCode: 0,
      stdout: stdout.length > maxText ? stdout.slice(-maxText) : stdout,
      stderr: stderr.length > maxText ? stderr.slice(-maxText) : stderr,
      truncated: stdout.length > maxText || stderr.length > maxText,
      cwd: cwd || process.cwd()
    };
  } catch (e) {
    return {
      exitCode: typeof e.code === 'number' ? e.code : 1,
      stdout: String(e.stdout || '').slice(-maxText),
      stderr: String(e.stderr || e.message || e).slice(-maxText),
      cwd: cwd || process.cwd()
    };
  }
}

async function listDir(p) {
  const entries = await fs.readdir(p, { withFileTypes: true });
  return Promise.all(entries.slice(0, 1000).map(async e => {
    const full = path.join(p, e.name);
    let stat = null;
    try { stat = await fs.stat(full); } catch {}
    return { name: e.name, path: full, type: e.isDirectory() ? 'directory' : e.isFile() ? 'file' : 'other', size: stat?.size ?? null, modified: stat?.mtime?.toISOString?.() ?? null };
  }));
}

async function searchTree(root, query, maxResults = 100) {
  const q = String(query || '').toLowerCase();
  if (!q) throw new Error('query is required for search');
  const out = [];
  const queue = [root];
  let scanned = 0;
  while (queue.length && out.length < maxResults && scanned < 20000) {
    const dir = queue.shift();
    let entries;
    try { entries = await fs.readdir(dir, { withFileTypes: true }); } catch { continue; }
    for (const e of entries) {
      if (out.length >= maxResults || scanned++ >= 20000) break;
      const full = path.join(dir, e.name);
      if (e.name.toLowerCase().includes(q)) out.push({ path: full, match: 'name', type: e.isDirectory() ? 'directory' : 'file' });
      if (e.isDirectory()) {
        if (!['node_modules', '.git', '$Recycle.Bin', 'System Volume Information'].includes(e.name)) queue.push(full);
      } else if (e.isFile() && out.length < maxResults) {
        try {
          const st = await fs.stat(full);
          if (st.size <= 1024 * 1024) {
            const buf = await fs.readFile(full);
            if (!buf.includes(0)) {
              const text = buf.toString('utf8');
              const idx = text.toLowerCase().indexOf(q);
              if (idx >= 0) out.push({ path: full, match: 'content', preview: text.slice(Math.max(0, idx - 120), idx + q.length + 220) });
            }
          }
        } catch {}
      }
    }
  }
  return { results: out.slice(0, maxResults), scanned, truncated: queue.length > 0 || out.length >= maxResults };
}

async function filesOp(args) {
  switch (args.action) {
    case 'read': {
      const data = await fs.readFile(args.path);
      if (data.includes(0)) return { path: args.path, binary: true, size: data.length, message: 'Binary file; use shell or transfer tooling for raw bytes.' };
      const text = data.toString('utf8');
      return { path: args.path, content: text.length > MAX_TEXT ? text.slice(0, MAX_TEXT) : text, size: data.length, truncated: text.length > MAX_TEXT };
    }
    case 'write':
      await fs.mkdir(path.dirname(args.path), { recursive: true });
      await fs.writeFile(args.path, args.content ?? '', 'utf8');
      return { ok: true, path: args.path };
    case 'append':
      await fs.mkdir(path.dirname(args.path), { recursive: true });
      await fs.appendFile(args.path, args.content ?? '', 'utf8');
      return { ok: true, path: args.path };
    case 'list': return { path: args.path, entries: await listDir(args.path) };
    case 'search': return searchTree(args.path, args.query, args.maxResults || 100);
    case 'stat': {
      const s = await fs.stat(args.path);
      return { path: args.path, size: s.size, directory: s.isDirectory(), file: s.isFile(), modified: s.mtime.toISOString(), created: s.birthtime.toISOString(), mode: s.mode };
    }
    case 'copy':
      if (!args.destination) throw new Error('destination is required');
      await fs.cp(args.path, args.destination, { recursive: args.recursive !== false, force: true });
      return { ok: true, source: args.path, destination: args.destination };
    case 'move':
      if (!args.destination) throw new Error('destination is required');
      await fs.rename(args.path, args.destination);
      return { ok: true, source: args.path, destination: args.destination };
    case 'mkdir':
      await fs.mkdir(args.path, { recursive: true });
      return { ok: true, path: args.path };
    case 'delete':
      await fs.rm(args.path, { recursive: args.recursive !== false, force: true });
      return { ok: true, path: args.path };
    default: throw new Error(`Unknown files action: ${args.action}`);
  }
}

async function processOp(args) {
  if (args.action === 'list' || args.action === 'find') {
    if (process.platform === 'win32') {
      const cmd = 'powershell -NoProfile -Command "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,CommandLine | ConvertTo-Json -Compress"';
      const { stdout } = await execAsync(cmd, { maxBuffer: 8 * 1024 * 1024 });
      let rows = [];
      try { rows = JSON.parse(stdout || '[]'); } catch {}
      if (!Array.isArray(rows)) rows = [rows];
      const q = String(args.name || '').toLowerCase();
      if (args.action === 'find' && q) rows = rows.filter(r => `${r.Name || ''} ${r.CommandLine || ''}`.toLowerCase().includes(q));
      const total=rows.length;
      rows = rows.slice(0, args.action === 'find' ? 100 : 150).map(r => ({ pid: r.ProcessId, ppid: r.ParentProcessId, name: r.Name, command: r.CommandLine || '' }));
      return { processes: rows, count: total, truncated: total > (args.action === 'find' ? 100 : 150) };
    }
    const psCmd = process.platform === 'darwin'
      ? "ps -axo pid=,ppid=,user=,stat=,comm=,args="
      : "ps -eo pid=,ppid=,user=,stat=,comm=,args= --sort=-pid";
    const { stdout } = await execAsync(psCmd, { maxBuffer: 8 * 1024 * 1024 });
    const rows = stdout.split(/\r?\n/).filter(Boolean).map(line => {
      const m = line.match(/^\s*(\d+)\s+(\d+)\s+(\S+)\s+(\S+)\s+(\S+)\s*(.*)$/);
      return m ? { pid: Number(m[1]), ppid: Number(m[2]), user: m[3], stat: m[4], name: m[5], command: m[6] || '' } : null;
    }).filter(Boolean);
    rows.sort((a,b) => b.pid - a.pid);
    const q = String(args.name || '').toLowerCase();
    const filtered = args.action === 'find' && q ? rows.filter(r => `${r.name} ${r.command}`.toLowerCase().includes(q)) : rows;
    const limit = args.action === 'find' ? 100 : 150;
    return { processes: filtered.slice(0, limit), count: filtered.length, truncated: filtered.length > limit };
  }
  if (args.action === 'kill') {
    if (!args.pid) throw new Error('pid is required for kill');
    if (process.platform === 'win32') await execAsync(`taskkill /PID ${Number(args.pid)} /F`);
    else process.kill(Number(args.pid), 'SIGKILL');
    return { ok: true, pid: Number(args.pid) };
  }
  throw new Error(`Unknown process action: ${args.action}`);
}

export async function dispatch(op, args) {
  if (op === 'jobs') return jobs.operate(args);
  if (op === 'status') return {device:DEVICE,version:'0.3.0',platform:process.platform,arch:os.arch(),home:os.homedir(),uptime:process.uptime(),memory:process.memoryUsage().rss,startup:localConfig.startup||process.env.INFRA_STARTUP||'unknown',capabilities:['shell','files','process','jobs'],desktop:false};
  if (op === 'shell') return shellOp(args);
  if (op === 'files') return filesOp(args);
  if (op === 'process') return processOp(args);
  throw new Error(`Unknown operation: ${op}`);
}

function onWs(ws, event, handler) {
  if (typeof ws.on === 'function') return ws.on(event, handler);
  if (typeof ws.addEventListener === 'function') {
    return ws.addEventListener(event, ev => handler(event === 'message' ? ev.data : ev));
  }
  throw new Error('Unsupported WebSocket implementation');
}

function rawToText(raw) {
  if (typeof raw === 'string') return raw;
  if (Buffer.isBuffer(raw)) return raw.toString('utf8');
  if (raw instanceof ArrayBuffer) return Buffer.from(raw).toString('utf8');
  if (ArrayBuffer.isView(raw)) return Buffer.from(raw.buffer, raw.byteOffset, raw.byteLength).toString('utf8');
  return String(raw ?? '');
}

let reconnectDelay = 1000, reconnectTimer, activeSocket, stopping=false;
function connect() {
  if(stopping)return;
  const ws = new WebSocketImpl(SERVER);
  activeSocket=ws;
  onWs(ws, 'open', () => {
    ws.send(JSON.stringify({
      type: 'hello', token: TOKEN, device: DEVICE,version:'0.3.0',home:os.homedir(),startup:localConfig.startup||process.env.INFRA_STARTUP||'unknown',
      os: `${process.platform} ${os.release()}`, hostname: os.hostname(), arch: os.arch(),
      capabilities: ['shell', 'files', 'process','jobs']
    }));
  });
  onWs(ws, 'message', async raw => {
    let msg;
    try { msg = JSON.parse(rawToText(raw)); } catch { return; }
    if (msg.type === 'hello_ack') {
      reconnectDelay=1000;
      console.log(`connected to Phloraxx MCP as '${DEVICE}'`);
      return;
    }
    if (msg.type !== 'request') return;
    try {
      const result = await dispatch(msg.op, msg.args || {});
      if(ws.readyState === 1)ws.send(JSON.stringify({ type: 'response', id: msg.id, ok: true, result }));
    } catch (e) {
      if(ws.readyState === 1)ws.send(JSON.stringify({ type: 'response', id: msg.id, ok: false, error: String(e.message || e) }));
    }
  });
  onWs(ws, 'close', () => {if(stopping)return;reconnectTimer=setTimeout(connect,reconnectDelay+Math.random()*500);reconnectDelay=Math.min(reconnectDelay*2,30000);});
  onWs(ws, 'error', err => console.error('agent socket error:', err?.message || err?.error?.message || 'websocket error'));
}

function startAgent() {
  fsSync.mkdirSync(STATE_DIR,{recursive:true,mode:0o700});
  const lock=path.join(STATE_DIR,'agent.lock');
  const ownLock=()=>fsSync.writeFileSync(lock,JSON.stringify({pid:process.pid}),{flag:'wx',mode:0o600});
  try {ownLock();}catch(e){
    if(e.code !== 'EEXIST')throw e;
    let pid;try{pid=JSON.parse(fsSync.readFileSync(lock,'utf8')).pid;}catch{}
    let alive=false;if(pid)try{process.kill(pid,0);alive=true;}catch{}
    if(alive){console.error('Infra MCP agent is already running for this installation.');return;}
    fsSync.rmSync(lock,{force:true});ownLock();
  }
  process.on('exit',()=>{try{if(JSON.parse(fsSync.readFileSync(lock,'utf8')).pid===process.pid)fsSync.rmSync(lock);}catch{}});
  const shutdown=()=>{stopping=true;clearTimeout(reconnectTimer);activeSocket?.close();setTimeout(()=>process.exit(0),200).unref();};
  process.on('SIGTERM',shutdown);process.on('SIGINT',shutdown);
  console.log(`Infra MCP agent 0.3.0 starting as '${DEVICE}'`);
  connect();
}
if(process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if(process.argv[2] === '--job-runner')runJob(process.argv[3]);
  else startAgent();
}


