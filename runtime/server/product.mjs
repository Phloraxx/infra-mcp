import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import os from 'node:os';

export function installProduct({ app, agents, rpc, signToken, verifyToken, base, password, user, dataDir, publicDir, validate }) {
  fs.mkdirSync(dataDir, { recursive: true, mode: 0o700 });
  const file = path.join(dataDir, 'devices.json');
  let records = {};
  try { records = JSON.parse(fs.readFileSync(file, 'utf8')); } catch (e) { if (e.code !== 'ENOENT') throw e; }
  const pairing = new Map(), attempts = new Map();
  const events = [];
  const hash = value => crypto.createHash('sha256').update(String(value)).digest('hex');
  const equal = (a, b) => crypto.timingSafeEqual(Buffer.from(hash(a)), Buffer.from(hash(b)));
  const persist = () => {
    const temp = `${file}.tmp`;
    fs.writeFileSync(temp, JSON.stringify(records), { mode: 0o600 });
    fs.renameSync(temp, file);
  };
  const event = (action, device) => { events.unshift({ action, device, at: new Date().toISOString() }); events.length = Math.min(events.length, 100); };
  const deviceView = () => Object.values(records).map(({ tokenHash, ...r }) => ({ ...r, online: agents.has(r.device), ...(agents.get(r.device)?.meta || {}) }));
  function limited(req, key, max = 10) {
    const now = Date.now(), id = `${key}:${req.headers['cf-connecting-ip'] || req.socket.remoteAddress}`;
    let a = attempts.get(id);
    if (!a || a.until < now) a = { n: 0, until: now + 60000 };
    attempts.set(id, a);
    if (attempts.size > 1000) for (const [k, v] of attempts) if (v.until < now) attempts.delete(k);
    return ++a.n > max;
  }
  const cookie = req => {
    const raw = (req.headers.cookie || '').split(';').find(x => x.trim().startsWith('infra_session='));
    if (!raw) return null;
    try { return verifyToken(decodeURIComponent(raw.trim().slice('infra_session='.length))); } catch { return null; }
  };
  const auth = (req, res, next) => {
    if (cookie(req)?.kind === 'dashboard') return next();
    res.status(401).json({ error: 'Sign in to continue.' });
  };
  const origin = (req, res, next) => {
    if (req.method === 'GET') return next();
    if (req.headers.origin === base) return next();
    res.status(403).json({ error: 'This request must come from your Infra MCP dashboard.' });
  };
  app.get('/', (_req, res) => res.set('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'").sendFile(path.join(publicDir, 'index.html')));
  app.get('/ui/:asset', (req, res) => {
    if (!['app.js', 'style.css'].includes(req.params.asset)) return res.sendStatus(404);
    res.set('Cache-Control', 'no-cache').sendFile(path.join(publicDir, req.params.asset));
  });
  app.use('/api', (_req, res, next) => {
    res.set('Cache-Control', 'no-store');
    res.set('X-Content-Type-Options', 'nosniff');
    next();
  });
  app.get('/api/session', (req, res) => res.json({ authenticated: cookie(req)?.kind === 'dashboard', name: 'Infra MCP', version: '0.3.0', defaultDevice: 'oracle' }));
  app.post('/api/session', origin, (req, res) => {
    if (limited(req, 'login')) return res.status(429).json({ error: 'Too many attempts. Try again in a minute.' });
    if (!password || !equal(String(req.body?.password || ''), password)) return res.status(401).json({ error: 'Incorrect password. Try again.' });
    const now = Math.floor(Date.now() / 1000);
    const token = signToken({ sub: user, aud: `${base}/mcp`, kind: 'dashboard', iat: now, exp: now + 12 * 3600 });
    res.cookie('infra_session', token, { httpOnly: true, secure: base.startsWith('https:'), sameSite: 'strict', maxAge: 12 * 3600000, path: '/' });
    event('Signed in', 'dashboard');
    res.json({ ok: true });
  });
  app.delete('/api/session', origin, auth, (_req, res) => { res.clearCookie('infra_session', { path: '/' }); res.json({ ok: true }); });
  app.use('/api', auth, origin);
  app.get('/api/overview', (_req, res) => res.json({ devices: deviceView(), events, server: { version: '0.3.0', uptime: process.uptime(), memory: process.memoryUsage().rss, host: os.hostname(), endpoint: `${base}/mcp` } }));
  app.post('/api/pairing', (req, res) => {
    for (const [code, p] of pairing) if (p.expiresAt < Date.now()) pairing.delete(code);
    if (pairing.size >= 100) return res.status(429).json({ error: 'Too many active pairing codes.' });
    const code = crypto.randomBytes(5).toString('hex').toUpperCase();
    const expiresAt = Date.now() + 10 * 60000;
    pairing.set(code, { expiresAt });
    res.json({ code, expiresAt, base });
  });
  app.post('/pair', (req, res) => {
    if (limited(req, 'pair', 15)) return res.status(429).json({ error: 'Too many attempts. Try again in a minute.' });
    const code = String(req.body?.code || '').replace(/[-\s]/g, '').toUpperCase();
    const p = pairing.get(code), device = String(req.body?.device || '').toLowerCase();
    if (!p || p.expiresAt < Date.now()) return res.status(400).json({ error: 'Pairing code expired or was already used. Get a new code from the dashboard.' });
    if (!/^[a-z0-9][a-z0-9._-]{0,62}$/.test(device)) return res.status(400).json({ error: 'Use a device name with letters, numbers, dots, dashes or underscores (up to 63 characters).' });
    if (records[device] && !records[device].revoked) return res.status(409).json({ error: 'That device name is already paired. Use a different name, or remove the old device first.' });
    const token = crypto.randomBytes(32).toString('base64url');
    records[device] = { device, tokenHash: hash(token), auth: 'paired', revoked: false, pairedAt: new Date().toISOString(), hostname: String(req.body?.hostname || device).slice(0, 120), platform: String(req.body?.platform || 'unknown').slice(0, 20) };
    persist(); pairing.delete(code); event('Device paired', device);
    res.json({ device, token, server: `${base.replace(/^http/, 'ws')}/agent` });
  });
  app.post('/agent-status', (req,res)=>{
    const device=String(req.body?.device||''),r=records[device];
    if(!r || r.revoked || !req.body?.token || !equal(hash(req.body.token),r.tokenHash||''))return res.status(401).json({error:'Invalid device credential.'});
    res.json({device,online:agents.has(device)});
  });
  app.delete('/api/devices/:device', (req, res) => {
    const d = records[req.params.device];
    if (!d) return res.status(404).json({ error: 'Device not found.' });
    d.revoked = true; persist(); agents.get(d.device)?.ws.close(4003, 'device removed'); event('Device removed', d.device);
    res.json({ ok: true });
  });
  app.post('/api/run', async (req, res) => {
    try {
      const { op, args } = req.body || {};
      const input = validate(op, args);
      const result = await rpc(input.device, op, input, op === 'shell' ? (input.timeoutMs || 30000) + 5000 : 60000);
      event(op === 'shell' ? 'Command submitted' : `${op} operation`, input.device);
      res.json(result);
    } catch (e) { res.status(400).json({ error: e.message }); }
  });
  return {
    authenticate(device, token, legacyToken) {
      const r = records[device];
      if (r?.revoked) return false;
      if (r?.auth === 'paired') return equal(hash(token), r.tokenHash);
      return !!legacyToken && equal(token, legacyToken);
    },
    connected(meta) {
      records[meta.device] = { ...records[meta.device], ...meta, auth: records[meta.device]?.auth || 'legacy', revoked: false, lastSeen: new Date().toISOString() };
      persist(); event('Device connected', meta.device);
    },
    disconnected(device) {
      if (records[device]) { records[device].lastSeen = new Date().toISOString(); persist(); }
      event('Device disconnected', device);
    },
    list: deviceView
  };
}
