import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';

const bootTime = () => Date.now() - os.uptime() * 1000;
const alive = pid => { try { process.kill(pid, 0); return true; } catch (e) { return e.code === 'EPERM'; } };
function write(file, data) {
  const temp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(temp, JSON.stringify(data), { mode: 0o600 });
  fs.renameSync(temp, file);
}
export function createJobs(root, entry) {
  fs.mkdirSync(root, { recursive: true, mode: 0o700 });
  function read(id) {
    if (!/^[a-f0-9-]{36}$/.test(String(id))) throw new Error('A valid job ID is required.');
    const dir = path.join(root, id), file = path.join(dir, 'job.json');
    let job;
    try { job = JSON.parse(fs.readFileSync(file, 'utf8')); } catch { throw new Error('Job not found.'); }
    if (['starting', 'running'].includes(job.status) && (Math.abs(job.bootTime - bootTime()) > 60000 || (job.runnerPid && !alive(job.runnerPid)))) {
      job = { ...job, status: 'interrupted', finishedAt: new Date().toISOString() }; write(file, job);
    }
    return { dir, file, job };
  }
  return {
    async start(args, shell) {
      const existing=fs.readdirSync(root).filter(x=>/^[a-f0-9-]{36}$/.test(x)).map(x=>read(x));
      const completed=existing.filter(x=>!['running','starting'].includes(x.job.status)).sort((a,b)=>b.job.startedAt.localeCompare(a.job.startedAt));
      for(const old of completed.slice(50))fs.rmSync(old.dir,{recursive:true,force:true});
      const id = crypto.randomUUID(), dir = path.join(root, id), file = path.join(dir, 'job.json');
      const running = fs.readdirSync(root).filter(x => /^[a-f0-9-]{36}$/.test(x)).map(x => read(x).job).filter(j => ['running', 'starting'].includes(j.status));
      if (running.length >= 20) throw new Error('20 jobs are already running. Finish or cancel a job first.');
      fs.mkdirSync(dir, { mode: 0o700 });
      write(file, { id, command: args.command, cwd: args.cwd || process.cwd(), shell, timeoutMs: args.timeoutMs || null, status: 'starting', bootTime: bootTime(), startedAt: new Date().toISOString() });
      const child = spawn(process.execPath, [entry, '--job-runner', dir], { detached: true, stdio: 'ignore', windowsHide: true });
      await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject); });
      child.unref();
      return { background: true, jobId: id, pid: child.pid, status: 'starting', message: 'Use jobs.read with this job ID to retrieve output and exit status.' };
    },
    async operate(args) {
      if (args.action === 'list') {
        const jobs = fs.readdirSync(root).filter(x => /^[a-f0-9-]{36}$/.test(x)).map(id => read(id).job)
          .sort((a, b) => b.startedAt.localeCompare(a.startedAt)).slice(0, 50)
          .map(({ shell, bootTime, ...j }) => j);
        return { jobs };
      }
      const { dir, file, job } = read(args.id);
      if (args.action === 'cancel') {
        if (!['running', 'starting'].includes(job.status)) return { ok: true, ...job };
        if (!job.runnerPid) throw new Error('Job is starting; inspect it again before cancellation.');
        // read() checks both the boot and runner liveness before signalling.
        if (process.platform === 'win32') {
          await new Promise((resolve, reject) => {
            const child = spawn('taskkill', ['/PID', String(job.runnerPid), '/T', '/F'], { windowsHide: true });
            child.once('error', reject); child.once('exit', code => code === 0 ? resolve() : reject(new Error('Could not cancel job.')));
          });
        } else process.kill(-job.runnerPid, 'SIGTERM');
        write(file, { ...job, status: 'canceled', finishedAt: new Date().toISOString() });
        return { ok: true, id: job.id, status: 'canceled' };
      }
      if (args.action !== 'read') throw new Error('Unsupported job action.');
      const log = path.join(dir, 'output.log');
      const size = fs.existsSync(log) ? fs.statSync(log).size : 0;
      const offset = Math.min(Math.max(0, args.offset || 0), size), limit = Math.min(args.limit || 16000, 100000);
      const buffer = Buffer.alloc(Math.min(limit, size - offset));
      if (buffer.length) { const fd = fs.openSync(log, 'r'); try { fs.readSync(fd, buffer, 0, buffer.length, offset); } finally { fs.closeSync(fd); } }
      return { ...job, output: buffer.toString('utf8'), offset, nextOffset: offset + buffer.length, totalBytes: size, hasMore: offset + buffer.length < size };
    }
  };
}

export function runJob(dir) {
  const file = path.join(dir, 'job.json'), log = path.join(dir, 'output.log');
  const job = JSON.parse(fs.readFileSync(file, 'utf8'));
  let bytes = 0, finished = false, canceled = false, timer;
  const maxBytes = 8 * 1024 * 1024;
  const append = data => {
    const remaining = maxBytes - bytes;
    if (remaining <= 0) { job.outputTruncated = true; return; }
    const part = Buffer.from(data).subarray(0, remaining);
    fs.appendFileSync(log, part, { mode: 0o600 }); bytes += part.length;
    if (part.length < data.length) job.outputTruncated = true;
  };
  job.status = 'running'; job.runnerPid = process.pid; write(file, job);
  let child;
  const finish = (code, signal, error) => {
    if (finished) return;
    finished = true; clearTimeout(timer);
    const latest = JSON.parse(fs.readFileSync(file, 'utf8'));
    write(file, { ...job, status: canceled || latest.status === 'canceled' ? 'canceled' : error ? 'failed' : 'completed', exitCode: code, signal, error: error?.message, finishedAt: new Date().toISOString(), outputBytes: bytes });
  };
  try {
    child = spawn(job.command, { cwd: job.cwd, shell: job.shell, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    job.commandPid = child.pid; write(file, job);
    child.stdout.on('data', append); child.stderr.on('data', append);
    child.once('error', error => finish(null, null, error));
    child.once('close', (code, signal) => finish(code, signal));
    process.on('SIGTERM', () => { canceled = true; child.kill('SIGTERM'); });
    if (job.timeoutMs) timer = setTimeout(() => { canceled = true; child.kill('SIGTERM'); }, Math.min(job.timeoutMs, 120000));
  } catch (error) { finish(null, null, error); }
}
