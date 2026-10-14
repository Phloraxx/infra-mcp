const $ = id => document.getElementById(id);
let state = { devices: [], selected: 'oracle', endpoint: '', tab: 'console', paired: null, editing: '', job: null, jobOffset: 0, generation: 0, running: new Set() };
const text = (tag, value, className) => { const e = document.createElement(tag); e.textContent = value; if (className) e.className = className; return e; };
const bytes = value => value < 1024 ? `${value} B` : value < 1048576 ? `${(value / 1024).toFixed(1)} KB` : `${(value / 1048576).toFixed(1)} MB`;
async function api(url, options = {}) {
  const res = await fetch(url, { ...options, headers: { 'Content-Type': 'application/json', ...options.headers } });
  const data = await res.json();
  if (!res.ok) { if (res.status === 401 && url !== '/api/session') showLogin(); throw new Error(data.error || 'Request failed.'); }
  return data;
}
const post = (url, body) => api(url, { method: 'POST', body: JSON.stringify(body) });
const run = (op, args) => post('/api/run', { op, args: { device: state.selected, ...args } });
const message = (value, error = false) => { $('status-message').textContent = value; $('status-message').classList.toggle('error', error); };
function showLogin() { $('login').hidden = false; $('dashboard').hidden = true; $('pair-dialog').close(); }
function showDashboard() { $('login').hidden = true; $('dashboard').hidden = false; }
function switchTab(tab) {
  state.tab = tab;
  for (const button of document.querySelectorAll('[data-tab]')) { const selected = button.dataset.tab === tab; button.setAttribute('aria-selected', String(selected)); $(`tab-${button.dataset.tab}`).hidden = !selected; }
  if (tab === 'jobs') refreshJobs().catch(e => message(e.message, true));
}
function selectDevice(device) {
  if (state.selected !== device) { state.selected = device; state.generation++; state.job = null; state.editing = ''; $('file-editor').hidden = true; $('file-path').value = ''; $('file-list').replaceChildren(); $('job-output').hidden = true; $('console-output').textContent = 'Ready. Run a command on the selected device.'; }
  const d = state.devices.find(x => x.device === device);
  $('device-title').textContent = device === 'oracle' ? 'Oracle' : device;
  $('device-detail').textContent = d ? `${d.hostname || d.device} · ${d.os || d.platform || 'Unknown system'} · ${d.arch || ''}` : 'No devices connected yet. Add your first device.';
  $('offline-banner').hidden = !!d?.online;
  $('run-command').disabled = !d?.online || state.running.has(device);
  $('remove-device').disabled = !d;
  $('cwd').placeholder = d?.home || 'Agent default directory';
  if (!$('file-path').value) $('file-path').value = d?.home || '';
  $('capabilities').replaceChildren(...(d?.capabilities || []).map(x => text('span', x, 'chip')));
  $('startup-detail').textContent = `Startup: ${d?.startup || 'Not reported by this agent'}. Agent ${d?.version || 'version unknown'}.`;
  for (const b of $('device-list').children) b.classList.toggle('selected', b.dataset.device === device);
}
async function refresh() {
  if ($('dashboard').hidden) return;
  try {
    const data = await api('/api/overview');
    state.devices = data.devices.filter(d => !d.revoked);
    state.endpoint = data.server.endpoint;
    $('gateway-state').textContent = 'Gateway online'; $('gateway-state').classList.remove('offline');
    $('version').textContent = `Infra MCP ${data.server.version}`;
    $('gateway-memory').textContent = `Gateway memory · ${bytes(data.server.memory)}`;
    $('device-count').textContent = state.devices.length;
    $('mcp-endpoint').textContent = state.endpoint;
    $('device-list').replaceChildren(...state.devices.map(d => {
      const b = text('button', ''); b.type = 'button'; b.dataset.device = d.device;
      b.append(text('span', '', `dot ${d.online ? 'online' : ''}`), text('span', d.device === 'oracle' ? 'Oracle' : d.device), text('small', d.online ? 'Connected' : 'Offline'));
      b.addEventListener('click', () => selectDevice(d.device)); return b;
    }));
    if (!state.devices.some(d => d.device === state.selected) && state.devices.length) state.selected = state.devices.find(d => d.device === 'oracle')?.device || state.devices[0].device;
    selectDevice(state.selected);
    $('activity-list').replaceChildren(...data.events.slice(0, 10).map(e => { const li = document.createElement('li'); li.append(text('span', `${e.action} · ${e.device}`), text('time', new Date(e.at).toLocaleTimeString())); return li; }));
    if (state.paired && !$('pair-dialog').open) state.paired = null;
    if (state.paired && state.devices.some(d => !state.paired.before.includes(d.device) && d.online)) $('pair-progress').textContent = 'Device connected. You can close this window and select it in the sidebar.';
  } catch (e) { $('gateway-state').textContent = 'Connection interrupted'; $('gateway-state').classList.add('offline'); message(e.message, true); }
}
$('login-form').addEventListener('submit', async e => {
  e.preventDefault(); const b = e.target.querySelector('button'); b.disabled = true; $('login-error').textContent = '';
  try { await post('/api/session', { password: $('password').value }); $('password').value = ''; showDashboard(); await refresh(); }
  catch (error) { $('login-error').textContent = error.message; } finally { b.disabled = false; }
});
$('logout').addEventListener('click', async () => { try { await api('/api/session', { method: 'DELETE' }); showLogin(); } catch (e) { message(e.message, true); } });
document.querySelectorAll('[data-tab]').forEach(b => b.addEventListener('click', () => switchTab(b.dataset.tab)));
$('clear-console').addEventListener('click', () => { $('console-output').textContent = ''; });
$('command').addEventListener('keydown', e => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); $('command-form').requestSubmit(); } });
$('command-form').addEventListener('submit', async e => {
  e.preventDefault(); if ($('run-command').disabled) return;
  const generation = state.generation, device = state.selected, command = $('command').value;
  state.running.add(device); $('run-command').disabled = true; $('console-output').textContent += `\n\n${device} › ${command}\n`;
  try {
    const result = await run('shell', { command, cwd: $('cwd').value || undefined, shell: $('shell').value, background: $('background').checked, timeoutMs: $('background').checked ? undefined : 30000 });
    if (generation !== state.generation) return;
    if (result.background) { $('console-output').textContent += `Background job ${result.jobId || result.pid} started. Open Jobs for output.`; message('Background job started.'); }
    else { $('console-output').textContent += `${result.stdout || ''}${result.stderr || ''}\n[exit ${result.exitCode}${result.truncated ? '; output truncated' : ''}]`; message(`Command finished with exit code ${result.exitCode}.`, result.exitCode !== 0); }
  } catch (error) { if (generation === state.generation) { $('console-output').textContent += error.message; message(error.message, true); } }
  finally { state.running.delete(device); selectDevice(state.selected); $('console-output').scrollTop = $('console-output').scrollHeight; }
});
async function openDirectory(path) {
  const generation = state.generation, result = await run('files', { action: 'list', path });
  if (generation !== state.generation) return;
  $('file-path').value = path; $('file-editor').hidden = true;
  const rows = result.entries.map(f => {
    const b = text('button', '', 'file-row'); b.type = 'button';
    b.append(text('span', `${f.type === 'directory' ? '▸ ' : ''}${f.name}`), text('small', f.type === 'directory' ? 'Directory' : bytes(f.size || 0)));
    b.addEventListener('click', async () => { try { if (f.type === 'directory') await openDirectory(f.path); else { const g = state.generation, data = await run('files', { action: 'read', path: f.path }); if (g !== state.generation) return; if (data.binary || data.truncated) throw new Error(data.binary ? 'Binary file. Use the console for this file.' : 'This file is too large to edit safely in the dashboard.'); state.editing = f.path; $('editing-path').textContent = f.path; $('file-content').value = data.content; $('file-editor').hidden = false; } } catch (e) { message(e.message, true); } });
    return b;
  });
  $('file-list').replaceChildren(...(rows.length ? rows : [text('div', 'This directory is empty.', 'empty')]));
}
$('files-form').addEventListener('submit', e => { e.preventDefault(); openDirectory($('file-path').value).catch(e => message(e.message, true)); });
$('save-file').addEventListener('click', async () => { if (!state.editing) return; try { await run('files', { action: 'write', path: state.editing, content: $('file-content').value }); message('File saved.'); } catch (e) { message(e.message, true); } });
async function refreshJobs() {
  const generation = state.generation, result = await run('jobs', { action: 'list' });
  if (generation !== state.generation) return;
  $('job-list').replaceChildren(...(result.jobs.length ? result.jobs.map(j => {
    const div = text('div', '', 'job'), body = document.createElement('div'), actions = text('div', '', 'job-actions');
    body.append(text('div', j.command, 'job-title'), text('small', `${j.status} · ${new Date(j.startedAt).toLocaleString()}${j.exitCode != null ? ` · exit ${j.exitCode}` : ''}`));
    const read = text('button', 'View output'); read.addEventListener('click', () => { state.job = j.id; state.jobOffset = 0; $('job-log').textContent = ''; $('job-title').textContent = j.command; readJob().catch(e => message(e.message, true)); }); actions.append(read);
    if (['running', 'starting'].includes(j.status)) { const cancel = text('button', 'Cancel', 'quiet danger'); cancel.addEventListener('click', async () => { try { await run('jobs', { action: 'cancel', id: j.id }); await refreshJobs(); } catch (e) { message(e.message, true); } }); actions.append(cancel); }
    div.append(body, actions); return div;
  }) : [text('div', 'No background jobs yet. Select “Run as a background job” in the console to start one.', 'empty')]));
}
async function readJob() { const id = state.job, g = state.generation; if (!id) return; const r = await run('jobs', { action: 'read', id, offset: state.jobOffset }); if (g !== state.generation || id !== state.job) return; $('job-output').hidden = false; $('job-log').textContent += r.output; state.jobOffset = r.nextOffset; message(`Job ${r.status}${r.exitCode != null ? ` · exit ${r.exitCode}` : ''}${r.outputTruncated ? ' · log limit reached' : ''}`); }
$('refresh-jobs').addEventListener('click', () => refreshJobs().catch(e => message(e.message, true)));
$('refresh-job').addEventListener('click', () => readJob().catch(e => message(e.message, true)));
async function copy(value, button) { try { await navigator.clipboard.writeText(value); const old = button.textContent; button.textContent = 'Copied'; setTimeout(() => button.textContent = old, 1500); } catch { message('Select and copy the text manually.', true); } }
$('copy-endpoint').addEventListener('click', e => copy(state.endpoint, e.target));
function installCommand() {
  if (!state.paired) return;
  const { base, code } = state.paired, platform = $('pair-platform').value;
  const curlFlags = platform === 'macos' ? '-4 --http1.1 ' : '';
  const installerEnv = platform === 'macos' ? 'INFRA_CURL_COMPAT=1 ' : '';
  const command = platform === 'windows' ? `Invoke-WebRequest '${base}/downloads/install-windows.ps1' -OutFile "$env:TEMP\\infra-install.ps1"\npowershell -NoProfile -ExecutionPolicy Bypass -File "$env:TEMP\\infra-install.ps1" -Code ${code}` : `(\n  infra_installer="$(mktemp)" && trap 'rm -f "$infra_installer"' EXIT &&\n  curl ${curlFlags}-fsSL '${base}/downloads/install.sh' -o "$infra_installer" &&\n  ${installerEnv}bash "$infra_installer" --code ${code}\n)`;
  $('install-command').value = command;
  $('install-command').rows = platform === 'windows' ? 4 : 6;
  $('system-install-help').textContent = platform === 'windows' ? 'In an elevated PowerShell window, add -System to the installer command. This installs a startup task for system shell access; it does not provide desktop access.' : 'Download the installer first, then run it with sudo and --system. This starts a system agent at boot; it does not provide desktop access.';
}
async function newPair() { const result = await post('/api/pairing', {}); state.paired = { ...result, before: state.devices.map(d => d.device) }; $('pair-code').textContent = result.code.slice(0, 5) + '-' + result.code.slice(5); $('pair-expiry').textContent = `Expires at ${new Date(result.expiresAt).toLocaleTimeString()}`; $('pair-progress').textContent = 'The installer asks for a device name. Your device will appear here after connection.'; installCommand(); }
$('add-device').addEventListener('click', async () => { $('pair-platform').value = /Win/.test(navigator.platform) ? 'windows' : /Mac/.test(navigator.platform) ? 'macos' : 'linux'; try { await newPair(); $('pair-dialog').showModal(); } catch (e) { message(e.message, true); } });
$('new-code').addEventListener('click', () => newPair().catch(e => message(e.message, true)));
$('pair-platform').addEventListener('change', installCommand);
$('close-pair').addEventListener('click', () => $('pair-dialog').close());
$('copy-install').addEventListener('click', e => copy($('install-command').value, e.target));
$('remove-device').addEventListener('click', async () => { const device = state.selected; if (!confirm(`Remove ${device}? It will lose access until you pair it again.`)) return; try { await api(`/api/devices/${encodeURIComponent(device)}`, { method: 'DELETE' }); await refresh(); } catch (e) { message(e.message, true); } });
api('/api/session').then(async s => { if (s.authenticated) { showDashboard(); await refresh(); } else showLogin(); }).catch(e => { showLogin(); $('login-error').textContent = e.message; });
setInterval(() => { if (!document.hidden) refresh(); }, 15000);
