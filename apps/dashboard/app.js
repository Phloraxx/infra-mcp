const state={health:null,devices:[],sessions:[],activity:[],platform:'unix',enrollment:null,device:null,tab:'overview',attachment:null,file:null,listing:null,routeVersion:0,authEpoch:0,authenticated:false,loginMethod:'password',events:null,streamHealthy:false,refreshPromise:null,refreshPending:false,refreshTimer:null,lastRefresh:0};
const $=s=>document.querySelector(s);const $$=s=>[...document.querySelectorAll(s)];
const esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
// Local, literal SVGs. No CDN/icon font and no interpolated user markup.
const iconShapes=Object.freeze({
monitor:'<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8m-4-4v4"/>',
laptop:'<rect x="4" y="4" width="16" height="13" rx="2"/><path d="M2 20h20"/>',
smartphone:'<rect x="7" y="2" width="10" height="20" rx="2"/><path d="M11 18h2"/>',
server:'<rect x="4" y="3" width="16" height="8" rx="2"/><rect x="4" y="13" width="16" height="8" rx="2"/><path d="M8 7h.01M8 17h.01"/>',
terminal:'<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3m6 3h4"/>',
folder:'<path d="M3 7a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v10H3z"/>',
file:'<path d="M7 2h7l5 5v15H5V4a2 2 0 0 1 2-2zM14 2v6h5M9 13h6M9 17h6"/>',
inbox:'<path d="M4 4h16l2 12v4H2v-4zm-2 12h6l2 3h4l2-3h6"/>',
jobs:'<rect x="4" y="3" width="16" height="18" rx="2"/><path d="M9 3.5h6M8 10h8m-8 4h8m-8 4h5"/>',
transfer:'<path d="M7 3v13m-4-4 4 4 4-4M17 21V8m-4 4 4-4 4 4"/>',
activity:'<path d="M3 12h4l3-7 4 14 3-7h4"/>',
devices:'<rect x="3" y="4" width="14" height="11" rx="2"/><path d="M7 20h6m-3-5v5"/><rect x="17" y="10" width="5" height="11" rx="1"/>',
check:'<circle cx="12" cy="12" r="9"/><path d="m8 12 3 3 5-6"/>',
alert:'<circle cx="12" cy="12" r="9"/><path d="M12 7v6m0 4h.01"/>',
offline:'<circle cx="12" cy="12" r="9"/><path d="M6 6l12 12"/>',
cpu:'<rect x="7" y="7" width="10" height="10" rx="2"/><path d="M9 1v6m6-6v6M9 17v6m6-6v6M1 9h6m-6 6h6m10-6h6m-6 6h6"/>',
network:'<circle cx="12" cy="4" r="2"/><circle cx="5" cy="19" r="2"/><circle cx="19" cy="19" r="2"/><path d="M11 6 6 17m7-11 5 11M7 19h10"/>',
git:'<circle cx="6" cy="4" r="2"/><circle cx="6" cy="20" r="2"/><circle cx="18" cy="6" r="2"/><path d="M6 6v12M18 8a7 7 0 0 1-7 7H6"/>',
package:'<path d="m12 2 9 5-9 5-9-5zm-9 5v10l9 5 9-5V7m-9 5v10"/>',
layers:'<path d="m12 2 9 5-9 5-9-5zm-9 10 9 5 9-5m-18 5 9 5 9-5"/>',
plus:'<path d="M12 5v14M5 12h14"/>',
left:'<path d="M19 12H5m7-7-7 7 7 7"/>',
right:'<path d="M5 12h14m-7-7 7 7-7 7"/>',
up:'<path d="M12 19V5m-7 7 7-7 7 7"/>',
search:'<circle cx="11" cy="11" r="7"/><path d="m16 16 5 5"/>',
pencil:'<path d="m4 20 4-.8L20 7a2 2 0 0 0-3-3L5 16z"/>',
refresh:'<path d="M20 7v5h-5M4 17v-5h5M5.5 9a7 7 0 0 1 12-2L20 12M4 12l2.5 5a7 7 0 0 0 12-2"/>',
lock:'<rect x="5" y="10" width="14" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/>',
shield:'<path d="m12 2 8 4v6c0 5-3 8-8 10-5-2-8-5-8-10V6z"/><path d="m9 12 2 2 4-4"/>',
logout:'<path d="M10 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h5M14 16l4-4-4-4m4 4H9"/>',
close:'<path d="M6 6l12 12M18 6 6 18"/>',
copy:'<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/>',
key:'<circle cx="8" cy="15" r="4"/><path d="M11 12 21 2l1 4-3 1-1 3-3 1-4 4"/>'
});
function icon(name){
  const safe=Object.hasOwn(iconShapes,name)?name:'devices';
  return '<svg class="ui-icon ui-icon-'+safe+'" viewBox="0 0 24 24" aria-hidden="true" focusable="false">'+iconShapes[safe]+'</svg>';
}
function platformIcon(platform){
  const value=String(platform||'').toLowerCase();
  return icon(value.includes('android')||value.includes('ios')?'smartphone':value.includes('mac')||value.includes('darwin')?'laptop':value.includes('linux')?'server':'monitor');
}
function kindIcon(kind){return icon(({terminal:'terminal',job:'jobs',transfer:'transfer'})[kind]||'activity');}
function emptyState(message,name='inbox'){return '<div class="empty">'+icon(name)+'<span>'+esc(message)+'</span></div>';}
function decorateStaticIcons(){
  const controls=[
    ['#add-device .button-plus','plus'],
    ['#signin-lock','lock'],
    ['#enroll-dialog .icon-button','close'],
    ['#copy-command','copy'],['#device-status','refresh'],
    ['#device-rename','pencil'],['#logout-session','logout'],
    ['#session-resume','refresh'],['#files-up','up'],
    ['#files-browse .secondary','folder'],['#file-close','close'],
    ['#file-next','right'],['#device-permissions','shield'],
    ['#create-enrollment','key'],
    ['.search-icon','search'],['#terminal-open .primary','terminal']
  ];
  for(const [selector,name] of controls){
    const element=$(selector);if(!element)continue;
    if(element.matches('.button-plus,#signin-lock,.icon-button,.search-icon'))element.innerHTML=icon(name);
    else element.insertAdjacentHTML('afterbegin',icon(name));
  }
  $$('.text-button.back span').forEach(element=>element.innerHTML=icon('left'));
  $$('.section-head .text-button span').forEach(element=>element.innerHTML=icon('right'));
}
decorateStaticIcons();
const REQUEST_TIMEOUT_MS=10000;
// The budget includes response-body reads. No request, especially a mutation, is
// automatically replayed after timeout: the server may already have acted.
async function requestWithDeadline(url,options,consume){
  const controller=new AbortController();
  const external=options.signal;
  const abort=()=>controller.abort();
  if(external?.aborted)abort();else external?.addEventListener('abort',abort,{once:true});
  let expired=false;
  const timer=setTimeout(()=>{expired=true;controller.abort();},REQUEST_TIMEOUT_MS);
  try{return await consume(await fetch(url,{...options,signal:controller.signal}));}
  catch(error){
    if(expired){
      const timeout=new Error('Request timed out. Its outcome may be unknown; check current state before repeating an action.');
      timeout.code='request_timeout';throw timeout;
    }
    throw error;
  }finally{clearTimeout(timer);external?.removeEventListener('abort',abort);}
}
async function jsonFetch(url,options={}){
  const epoch=state.authEpoch;
  return requestWithDeadline(url,{...options,credentials:'same-origin',headers:{'Content-Type':'application/json','X-Infra-CSRF':'1',...(options.headers||{})}},async response=>{
    if(response.status===401){
      if(epoch===state.authEpoch){clearPrivateViews();showSignedOut();}
      const error=new Error('Your session ended. Sign in again.');error.status=401;throw error;
    }
    if(!response.ok){const text=await response.text();let message=text;try{message=JSON.parse(text).error||text;}catch{}const error=new Error(message||`HTTP ${response.status}`);error.status=response.status;throw error;}
    return response.status===204?null:response.json();
  });
}
function showSignedOut(){state.authenticated=false;state.events?.close();state.events=null;state.streamHealthy=false;clearTimeout(state.refreshTimer);state.refreshTimer=null;$('#app-shell').classList.add('hidden');$('#signin').classList.remove('hidden');$('#login-submit').disabled=false;$('#login-submit').textContent='Sign in';}
function showSignedIn(){state.authenticated=true;$('#signin').classList.add('hidden');$('#app-shell').classList.remove('hidden');}
async function authStatus(){
  try{const v=await requestWithDeadline('/api/auth/session',{credentials:'same-origin'},async response=>{if(!response.ok)throw new Error('Authentication status unavailable');return response.json();});state.loginMethod=v.login_method||'password';$('#username-label').classList.toggle('hidden',state.loginMethod!=='password');$('#login-username').required=state.loginMethod==='password';$('#password-label').textContent=state.loginMethod==='password'?'Password':'Admin credential';$('#password-change').classList.toggle('hidden',state.loginMethod!=='password');$('#auth-state').textContent=v.authenticated?'Signed in':'Signed out';if(v.authenticated)showSignedIn();else showSignedOut();return !!v.authenticated;}
  catch(error){showSignedOut();$('#login-error').textContent='Cannot reach Infra. Try again shortly.';return false;}
}
$('#login-form').addEventListener('submit',async event=>{
  event.preventDefault();const button=$('#login-submit');button.disabled=true;button.textContent='Signing in…';$('#login-error').textContent='';
  const password=$('#login-password').value;const body=state.loginMethod==='password'?{username:$('#login-username').value,password}:{token:password};
  try{await requestWithDeadline('/api/auth/session',{method:'POST',credentials:'same-origin',headers:{'Content-Type':'application/json','X-Infra-CSRF':'1'},body:JSON.stringify(body)},async response=>{if(!response.ok)throw new Error((await response.text())||'Sign-in failed.');});$('#login-password').value='';if(await authStatus()){await refresh();route();connectEvents();}}
  catch(error){$('#login-error').textContent=error.message;}
  finally{button.disabled=false;button.textContent='Sign in';}
});
$('#password-change').addEventListener('submit',async event=>{
  event.preventDefault();const button=$('#password-change button'),current=$('#current-password').value,next=$('#new-password').value;$('#password-result').textContent='';
  if(next!==$('#confirm-password').value){$('#password-result').textContent='The new passwords do not match.';return;}button.disabled=true;
  try{await jsonFetch('/api/auth/password',{method:'POST',body:JSON.stringify({current_password:current,new_password:next})});$('#password-change').reset();$('#password-result').textContent='Password changed. Other browser sessions are now signed out.';}
  catch(error){$('#password-result').textContent=error.message;}finally{button.disabled=false;}
});
function showView(name,title,subtitle){
  $$('.view').forEach(v=>v.classList.toggle('active',v.id===name));
  const section=name==='device-detail'?'devices':name==='session-detail'?'sessions':name;
  $$('.nav').forEach(v=>{const active=v.dataset.view===section;v.classList.toggle('active',active);if(active)v.setAttribute('aria-current','page');else v.removeAttribute('aria-current');});
  $('#view-title').textContent=title;$('#view-subtitle').textContent=subtitle;$('#breadcrumb-view').textContent=name==='device-detail'?'Device':name==='session-detail'?'Session':'Workspace';
}
function setView(name){location.hash=name;route();}
$$('.nav').forEach(b=>b.addEventListener('click',()=>setView(b.dataset.view)));
$$('[data-view-target]').forEach(b=>b.addEventListener('click',()=>setView(b.dataset.viewTarget)));
function relative(ms){if(!ms)return 'never';const sec=Math.max(0,Math.floor((Date.now()-ms)/1000));if(sec<5)return 'just now';if(sec<60)return `${sec}s ago`;if(sec<3600)return `${Math.floor(sec/60)}m ago`;if(sec<86400)return `${Math.floor(sec/3600)}h ago`;return `${Math.floor(sec/86400)}d ago`;}
function capabilityLabel(cap){return ({exec:'shell',files:'files',terminal:'terminal',jobs:'jobs',transfer:'transfer',system:'system',network:'network',services:'services',containers:'docker',git:'git',desktop:'desktop'})[cap]||cap;}
function deviceState(d){return d.connection_state|| (d.revoked?'revoked':d.online?'online':'offline');}
function reconnectExplanation(d){
  const reason=({heartbeat_timeout:'No heartbeat was received; check sleep, network quality or agent load.',
    heartbeat_queue_full:'The control channel was saturated; inspect concurrent work and agent logs.',
    control_path_timeout:'A device operation stopped responding; its result may be uncertain.',
    send_timeout:'Sending to the agent timed out; check connectivity and system load.',
    send_error:'The gateway could not deliver a request to the agent.',
    socket_error:'The agent connection was interrupted; check Wi-Fi or tunnel stability.',
    socket_closed:'The WebSocket connection closed.'})[d.last_disconnect_reason]||'The agent is not connected.';
  const action=d.platform==='android'?'Open Infra Phone and check its background-service and battery settings.':'If this persists, run infra doctor --json on the device.';
  return reason+' '+action;
}
function deviceRow(d){
  const status=deviceState(d),work=(d.active_jobs||0)+(d.active_terminals||0)+(d.active_transfers||0),caps=d.capabilities||[];
  const shown=caps.slice(0,3).map(c=>`<span class="pill">${esc(capabilityLabel(c))}</span>`).join(''),more=caps.length>3?`<span class="pill more">+${caps.length-3}</span>`:'';
  return `<div class="device-row"><div class="device-primary"><div class="device-avatar ${d.online&&!d.revoked?'online':''}">${platformIcon(d.platform)}</div><div class="device-primary-text"><button class="device-link" data-device="${esc(d.device_id)}">${esc(d.device_name)}</button><div class="device-meta">${esc(d.platform)} · ${esc(d.arch)} · ${d.online?'connected now':`seen ${relative(d.last_seen_unix_ms)}`}</div></div></div><div class="caps">${shown}${more}</div><div class="work-count">${work?`${work} active`:'—'}</div><div class="status ${status}">${status[0].toUpperCase()+status.slice(1)}</div></div>`;
}
function sessionRow(s){
  const kind=String(s.kind||'session'),status=String(s.status||'unknown');
  return `<div class="session-row"><div class="session-kind-icon ${esc(kind)}">${kindIcon(kind)}</div><div><button class="device-link" data-session="${esc(s.id)}" data-device="${esc(s.device_id)}" data-kind="${esc(kind)}">${esc(s.device_name)} · ${esc(kind)}</button><div class="session-meta">${esc(status)} · started ${relative(s.started_at_unix_ms)}${s.device_online?'':' · device offline'} · ${esc(s.id)}</div></div><span class="pill">${esc(status)}</span></div>`;
}
function activityRow(a){
  const outcome=String(a.outcome||'');const state=outcome==='ok'||outcome==='success'?'ok':outcome.includes('fail')||outcome.includes('error')?'failed':'';
  return `<div class="activity-row"><div class="activity-dot ${state}"></div><div><div class="session-title">${esc(a.action)}${a.device_name?` · ${esc(a.device_name)}`:''}</div><div class="session-meta">${relative(a.at_unix_ms)} · ${esc(outcome)}${a.request_id?` · ${esc(a.request_id)}`:''}</div></div></div>`;
}
function activeSession(s){return ['running','starting','canceling','uploading','downloading','ready'].includes(s.status);}
function render(){
  const online=state.devices.filter(d=>d.online&&!d.revoked).length,offline=state.devices.filter(d=>!d.online&&!d.revoked).length,revoked=state.devices.filter(d=>d.revoked).length,active=state.sessions.filter(activeSession).length;
  const metrics=[['Connected',online,'online','check'],['Offline',offline,'offline','offline'],['Active sessions',active,'work','terminal'],['Total devices',state.devices.length-revoked,'enrolled','devices']];
  const indicator=$('#overview-availability');if(indicator){indicator.classList.toggle('has-offline',offline>0);indicator.innerHTML='<span class="availability-dot"></span>'+online+' of '+(state.devices.length-revoked)+' connected';}
  $('#summary').innerHTML=metrics.map(([k,v,c,i])=>`<div class="summary-card ${c}"><div class="metric-top"><span>${k}</span><span class="metric-icon">${icon(i)}</span></div><strong>${v}</strong></div>`).join('');
  const devices=state.devices.length?state.devices.map(deviceRow).join(''):emptyState('No devices enrolled yet.','devices');
  $('#all-devices').innerHTML=devices;applyDeviceSearch();
  $('#overview-devices').innerHTML=state.devices.length?state.devices.slice(0,5).map(deviceRow).join(''):'<div class="first-device">'+icon('devices')+'<h3>Add your first computer</h3><p>Install Infra, compare the pairing codes, and approve it here.</p><button class="primary" data-add-device>Add a device</button></div>';
  const sessions=state.sessions.filter(activeSession);renderSessionList();
  $('#overview-sessions').innerHTML=sessions.length?sessions.slice(0,6).map(sessionRow).join(''):emptyState('No sessions running right now.','terminal');
  $('#activity-list').innerHTML=state.activity.length?state.activity.map(activityRow).join(''):emptyState('No operational activity recorded yet.','activity');
  const attention=[];state.devices.filter(d=>!d.revoked&&!d.online).forEach(d=>attention.push(`<div class="attention-row"><div class="session-title">${esc(d.device_name)} is ${esc(deviceState(d))}</div><div class="session-meta">Last seen ${relative(d.last_seen_unix_ms)}. ${esc(reconnectExplanation(d))}</div><button class="secondary" data-device="${esc(d.device_id)}">Review connection</button></div>`));
  $('#attention').innerHTML=attention.join('');$('#attention-section').classList.toggle('hidden',attention.length===0);renderDevice();
}
// Merge bursts of SSE notifications and periodic fallbacks into a single in-flight
// snapshot request. Never allow an older authenticated request to overwrite newer state.
async function refresh(){
  if(!state.authenticated)return;
  if(state.refreshPromise){state.refreshPending=true;return state.refreshPromise;}
  const epoch=state.authEpoch;
  const pending=(async()=>{
    try{
      const {devices,sessions,activity}=await jsonFetch('/api/overview');
      if(epoch!==state.authEpoch)return;
      state.devices=devices;state.sessions=sessions;state.activity=activity;
      state.lastRefresh=Date.now();
      $('#gateway-dot').classList.add('online');
      $('#gateway-label').textContent='Gateway online';
      $('#gateway-label').removeAttribute('title');
      render();
      if(location.hash.startsWith('#session/')&&!state.attachment)route();
    }catch(error){
      if(epoch!==state.authEpoch)return;
      $('#gateway-dot').classList.remove('online');
      if(error.status===401){
        $('#gateway-label').textContent='Sign in required';
        $('#auth-state').textContent='Signed out';showSignedOut();
      }else{
        $('#gateway-label').textContent=state.lastRefresh?'Connection lost · showing last confirmed data':'Gateway unavailable';
        $('#gateway-label').title=error.message;
        console.error(error);
      }
    }
  })();
  state.refreshPromise=pending;
  try{await pending;}
  finally{
    if(state.refreshPromise===pending)state.refreshPromise=null;
    if(state.refreshPending&&state.authenticated){
      state.refreshPending=false;
      scheduleRefresh();
    }
  }
}
function scheduleRefresh(){
  if(!state.authenticated||state.refreshTimer)return;
  state.refreshTimer=setTimeout(()=>{state.refreshTimer=null;refresh();},125);
}
function connectEvents(){
  if(!state.authenticated||state.events)return;
  const events=new EventSource('/api/events');
  state.events=events;
  events.onopen=()=>{state.streamHealthy=true;scheduleRefresh();};
  events.onmessage=scheduleRefresh;
  events.onerror=()=>{
    state.streamHealthy=false;
    // EventSource reconnects itself; periodic polling remains the backup.
    scheduleRefresh();
  };
}
// Download to a local file so PowerShell reports HTTP failures and empty bodies
// instead of attempting to evaluate null content.
function powershellInstallerCommand(url, args=''){
  return "& { $ErrorActionPreference='Stop'; $f=Join-Path $env:TEMP ('infra-'+[guid]::NewGuid().ToString('N')+'.ps1'); try { Invoke-WebRequest -UseBasicParsing -Uri "+
    psQuote(url)+" -OutFile $f -ErrorAction Stop; if ((Get-Item -LiteralPath $f).Length -lt 200) { throw 'Infra installer download was empty' }; & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $f"+
    args+"; if ($LASTEXITCODE -ne 0) { throw 'Infra setup failed (exit '+$LASTEXITCODE+')' } } finally { Remove-Item -LiteralPath $f -Force -ErrorAction SilentlyContinue } }";
}
// The primary onboarding command is independent of legacy enrollment codes.
function updateQuickInstall(){
  const base=location.origin;
  // Never pipe a web response into Invoke-Expression. A missing/empty response
  // otherwise fails as a meaningless null-argument exception. Put the payload
  // on disk and terminate the whole block if downloading fails.
  // Only the downloaded file runs with process-scoped execution-policy bypass.
  const windows=powershellInstallerCommand(base+'/install.ps1');
  $('#quick-install-command').textContent=state.platform==='windows'
    ? windows
    : 'curl -fsSL '+shellQuote(base+'/install.sh')+' | sh';
}
$$('[data-quick-platform]').forEach(tab=>tab.addEventListener('click',()=>{
  state.platform=tab.dataset.quickPlatform;
  $$('[data-quick-platform]').forEach(t=>t.classList.toggle('active',t===tab));
  updateQuickInstall();
}));
$('#copy-quick-install').addEventListener('click',async()=>{
  try{
    await navigator.clipboard.writeText($('#quick-install-command').textContent);
    $('#copy-quick-install').textContent='Copied';
    setTimeout(()=>$('#copy-quick-install').textContent='Copy install command',1200);
  }catch(error){$('#pair-error').textContent='Clipboard unavailable. Select and copy the command above.';}
});
updateQuickInstall();
const PAIR_WAIT_MS=90000,PAIR_PROOF_MS=30000;
let pairFlow=null,pairPollRun=null,pairPollTimer=null;
function resetPairWait(flow){flow.until=performance.now()+PAIR_WAIT_MS;flow.untilWall=Date.now()+PAIR_WAIT_MS;}
function pairWaitExpired(flow){return performance.now()>=flow.until||Date.now()>=flow.untilWall;}
function pairProofFresh(flow){const wall=Date.now()-flow.verifiedWall,elapsed=performance.now()-flow.verifiedAt;return wall>=0&&wall<PAIR_PROOF_MS&&elapsed>=0&&elapsed<PAIR_PROOF_MS;}
function stopPairPolling(){clearTimeout(pairPollTimer);pairPollTimer=null;pairPollRun?.controller.abort();pairPollRun=null;}
function pairRunCurrent(run){return run===pairPollRun&&run.epoch===state.authEpoch&&state.authenticated&&$('#enroll-dialog').open;}
function renderPairFlow(){
  const flow=pairFlow,verified=flow?.phase==='verified';
  $('#quick-install').classList.toggle('hidden',!!flow?.approved);
  $('.legacy-enrollment').classList.toggle('hidden',!!flow);
  $('#pair-approval-copy').classList.toggle('hidden',!!flow);
  $('#pair-heading').textContent=verified?'3. Computer connected':flow?.approved?'3. Check connection':'2. Approve this computer';
  $('#pair-result').classList.toggle('pair-verified',verified);
  $('#pending-pairs').classList.toggle('hidden',!!flow);
  $('#pair-actions').classList.toggle('hidden',!flow);
  $('#pair-open').classList.toggle('hidden',!verified);
  $('#pair-check').classList.toggle('hidden',verified||flow?.phase==='pending');
  $('#pair-check').disabled=!!flow&&['approving','observing','waiting','checking'].includes(flow.phase);
  $('#pair-another').disabled=flow?.phase==='approving';
  $('#pair-another').textContent=flow?.phase==='pending'?'Review pairing request':'Add another device';
  if(!flow)return;
  const name=flow.name||'This computer';
  const messages={approving:'Approving this computer…',observing:'Checking whether approval completed…',
    waiting:`Approved. Waiting for ${name} to connect…`,checking:`Approved. Checking the connection to ${name}…`,
    verified:`${name} · Connection verified. The computer responded to a fresh check.`,
    paused:flow.approved?'Approved, but the connection could not be verified. Keep Infra running on that computer, check its network, then check again.':'Approval could not be confirmed. Check again to inspect the request without sending another approval.',
    expired:'This pairing request expired. Run setup again on that computer, then choose Add another device to review its request.',
    missing:'This pairing request is no longer available. Run setup again on that computer, then choose Add another device to review a new request.',
    pending:'Approval has not been recorded. Review the pairing request and compare the codes on both screens before approving.',
    stale:'The last connection check expired. Check the connection again before opening the device.'};
  $('#pair-result').textContent=messages[flow.phase]||messages.observing;
}
function startPairPolling(){
  stopPairPolling();
  if(!state.authenticated||!$('#enroll-dialog').open)return;
  const run={epoch:state.authEpoch,controller:new AbortController(),busy:false};pairPollRun=run;
  refreshPendingPairs(run);
}
async function refreshPendingPairs(run){
  if(!pairRunCurrent(run)||run.busy)return;
  run.busy=true;
  const flow=pairFlow,options={signal:run.controller.signal};
  const current=()=>pairRunCurrent(run)&&!run.paused&&pairFlow===flow;
  try{
    if(!flow){
      const pairs=await jsonFetch('/api/pair/requests',options);
      if(!current())return;
      $('#pair-error').textContent='';
      $('#pending-pairs').innerHTML=pairs.length?pairs.map(pair=>
        '<div class="pending-pair">'+
        '<div class="pending-pair-head"><strong>'+esc(pair.device_name)+'</strong><span class="muted small">'+esc(pair.platform)+' · '+esc(pair.arch)+'</span></div>'+
        '<div class="pending-code">Compare: <strong>'+esc(pair.verification_code)+'</strong></div>'+
        '<div class="row-actions"><button type="button" class="primary" data-pair-action="approve" data-pair-id="'+esc(pair.request_id)+'" data-pair-code="'+esc(pair.verification_code)+'">Approve this computer</button>'+
        '<button type="button" class="secondary" data-pair-action="reject" data-pair-id="'+esc(pair.request_id)+'">Reject</button></div></div>'
      ).join(''):'<p class="muted small">No computers are requesting approval yet.</p>';
      return;
    }
    if(flow.phase==='verified'){
      if(!pairProofFresh(flow)){flow.phase='stale';renderPairFlow();}
      return;
    }
    if(flow.phase==='approving'||['paused','expired','missing','stale','pending'].includes(flow.phase))return;
    if(pairWaitExpired(flow)){flow.phase='paused';renderPairFlow();return;}
    let record;
    try{record=await jsonFetch('/api/pair/'+encodeURIComponent(flow.id),options);}
    catch(error){if(current()&&error.status===404){flow.phase='missing';renderPairFlow();return;}throw error;}
    if(!current())return;
    if(record.request_id!==flow.id)throw new Error('Pairing request mismatch');
    if(record.state==='expired'){flow.phase='expired';renderPairFlow();return;}
    if(record.state==='pending'){flow.phase='pending';renderPairFlow();return;}
    if(record.state!=='approved'||typeof record.device_id!=='string'||!record.device_id)throw new Error('Invalid pairing status');
    if(flow.device&&flow.device!==record.device_id)throw new Error('Approved computer changed');
    flow.approved=true;flow.device=record.device_id;flow.name=record.device_name;flow.phase='waiting';renderPairFlow();
    const devices=await jsonFetch('/api/devices',options);
    if(!current())return;
    const device=devices.find(item=>item.device_id===flow.device);
    if(!device?.online||device.revoked)return;
    flow.phase='checking';renderPairFlow();
    const reply=await jsonFetch('/api/devices/'+encodeURIComponent(flow.device)+'/operations',
      {...options,method:'POST',body:JSON.stringify({op:'status'})});
    if(!current())return;
    if(reply.status!=='ok'||!Number.isInteger(reply.body?.pid)||reply.body.pid<=0||!Number.isInteger(reply.body?.protocol)||reply.body.protocol<=0)
      throw new Error('The computer did not return a valid connection check');
    // A registry row is insufficient. This response traversed the dispatcher and
    // its exact authenticated connection; it is a point-in-time check, not a lease.
    flow.phase='verified';flow.verifiedAt=performance.now();flow.verifiedWall=Date.now();$('#pair-error').textContent='';renderPairFlow();
    scheduleRefresh();
  }catch(error){
    if(current()){
      $('#pair-error').textContent='Connection check unavailable. Keep Infra running on the computer and check its network. Approval will not be sent again.';
      if(flow){flow.phase=pairWaitExpired(flow)?'paused':flow.approved?'waiting':'observing';renderPairFlow();}
    }
  }finally{
    run.busy=false;
    if(pairRunCurrent(run)&&!run.paused)pairPollTimer=setTimeout(()=>refreshPendingPairs(run),2500);
  }
}
$('#enroll-dialog').addEventListener('close',stopPairPolling);
$('#pending-pairs').addEventListener('click',async event=>{
  const button=event.target.closest('button[data-pair-action]');
  if(!button||pairFlow||!pairPollRun)return;
  const id=button.dataset.pairId,action=button.dataset.pairAction;
  const run=pairPollRun,options={signal:run.controller.signal};
  // Pause list polling during the mutation so its DOM cannot re-enable approval.
  clearTimeout(pairPollTimer);pairPollTimer=null;run.paused=true;
  if(action==='approve'){
    const flow={id,phase:'approving',approved:false};resetPairWait(flow);pairFlow=flow;
    $('#pair-error').textContent='';renderPairFlow();
    try{
      await jsonFetch('/api/pair/'+encodeURIComponent(id)+'/approve',
        {...options,method:'POST',body:JSON.stringify({verification_code:button.dataset.pairCode})});
    }catch(error){
      if(pairRunCurrent(run))$('#pair-error').textContent='The approval response was not confirmed. Checking its recorded state; approval will not be sent again.';
    }finally{
      if(pairFlow===flow){flow.phase='observing';if(pairRunCurrent(run)){renderPairFlow();startPairPolling();}}
    }
  }else{
    $$('#pending-pairs button').forEach(control=>control.disabled=true);
    try{await jsonFetch('/api/pair/'+encodeURIComponent(id)+'/reject',{...options,method:'POST'});if(pairRunCurrent(run))$('#pair-result').textContent='Pairing request rejected.';}
    catch(error){if(pairRunCurrent(run))$('#pair-error').textContent=error.message;}
    finally{if(pairRunCurrent(run))startPairPolling();}
  }
});
$('#pair-check').addEventListener('click',()=>{
  if(!pairFlow)return;
  pairFlow.phase='observing';resetPairWait(pairFlow);$('#pair-error').textContent='';renderPairFlow();startPairPolling();
});
$('#pair-another').addEventListener('click',()=>{
  pairFlow=null;$('#pair-result').textContent='';$('#pair-error').textContent='';renderPairFlow();startPairPolling();
});
$('#pair-open').addEventListener('click',()=>{
  if(pairFlow?.phase!=='verified')return;
  if(!pairProofFresh(pairFlow)){pairFlow.phase='stale';renderPairFlow();return;}
  const device=pairFlow.device;pairFlow=null;$('#enroll-dialog').close();navigateDevice(device);
  $('#view-title').tabIndex=-1;$('#view-title').focus();
});
$('#add-device').addEventListener('click',()=>{
  $('#enroll-start').classList.remove('hidden');$('#enroll-result').classList.add('hidden');
  $('#pair-result').textContent='';$('#pair-error').textContent='';
  if(pairFlow){pairFlow.phase='observing';resetPairWait(pairFlow);}
  $('#enroll-dialog').showModal();renderPairFlow();startPairPolling();
});
$('#create-enrollment').addEventListener('click',async()=>{const button=$('#create-enrollment');button.disabled=true;button.textContent='Creating…';try{state.enrollment=await jsonFetch('/api/enrollments',{method:'POST',body:JSON.stringify({ttl_seconds:300})});$('#enrollment-code').textContent=state.enrollment.code;$('#enrollment-expiry').textContent=`Expires ${new Date(state.enrollment.expires_at_unix_ms).toLocaleTimeString()}`;$('#enroll-start').classList.add('hidden');$('#enroll-result').classList.remove('hidden');updateInstallCommand();}catch(error){alert(error.message);}finally{button.disabled=false;button.textContent='Create enrollment code';}});
function gatewayWs(){const url=new URL(location.href);url.protocol=url.protocol==='https:'?'wss:':'ws:';url.pathname='/agent';url.search='';url.hash='';return url.toString();}
function updateInstallCommand(){if(!state.enrollment)return;const code=state.enrollment.code;const name=$('#device-name').value.trim();const nameArg=name?` --name ${shellQuote(name)}`:'';if(state.platform==='unix'){const script=`${location.origin}/downloads/install.sh`;$('#install-command').textContent=`curl -fsSL ${shellQuote(script)} | sh -s -- --gateway ${shellQuote(gatewayWs())} --code ${shellQuote(code)}${nameArg}`;}else{const script=`${location.origin}/downloads/install.ps1`;const psName=name?` -Name ${psQuote(name)}`:'';$('#install-command').textContent=powershellInstallerCommand(script,` -Gateway ${psQuote(gatewayWs())} -Code ${psQuote(code)}${psName}`);}}
function shellQuote(s){return `'${String(s).replaceAll("'","'\\''")}'`; }function psQuote(s){return `'${String(s).replaceAll("'","''")}'`;}
$$('.tab[data-platform]').forEach(tab=>tab.addEventListener('click',()=>{state.platform=tab.dataset.platform;$$('.tab').forEach(t=>t.classList.toggle('active',t===tab));updateInstallCommand();}));$('#device-name').addEventListener('input',updateInstallCommand);$('#copy-command').addEventListener('click',async()=>{await navigator.clipboard.writeText($('#install-command').textContent);$('#copy-command').textContent='Copied';setTimeout(()=>$('#copy-command').textContent='Copy',1200);});
$('#device-rename').addEventListener('click',()=>{
  const device=currentDevice();
  if(!device)return;
  $('#device-rename-name').value=device.device_name;
  $('#device-rename-error').textContent='';
  $('#device-rename-form').classList.remove('hidden');
  $('#device-rename-name').focus();
});
$('#device-rename-cancel').addEventListener('click',()=>$('#device-rename-form').classList.add('hidden'));
$('#device-rename-form').addEventListener('submit',async event=>{
  event.preventDefault();
  const device=currentDevice();
  if(!device)return;
  const button=$('#device-rename-form button[type="submit"]');
  button.disabled=true;$('#device-rename-error').textContent='';
  try{
    await jsonFetch('/api/devices/'+encodeURIComponent(device.device_id)+'/rename',{
      method:'POST',body:JSON.stringify({name:$('#device-rename-name').value.trim()||null})
    });
    $('#device-rename-form').classList.add('hidden');await refresh();
  }catch(error){$('#device-rename-error').textContent=error.message;}
  finally{button.disabled=false;}
});
$('#logout-session').addEventListener('click',async()=>{let failure='';try{await requestWithDeadline('/api/auth/session',{method:'DELETE',credentials:'same-origin',headers:{'X-Infra-CSRF':'1'}},async response=>{if(!response.ok)throw new Error('Sign-out could not be confirmed.');});}catch(error){failure='Local view cleared; server sign-out could not be confirmed. '+error.message;}finally{clearPrivateViews();$('#login-password').value='';$('#password-change').reset();showSignedOut();$('#login-error').textContent=failure;}});

function currentDevice(id=state.device){return state.devices.find(d=>d.device_id===id);}
function available(id){const d=currentDevice(id);return !!d?.online&&!d.revoked;}
function navigateDevice(id,tab='overview'){location.hash=`device/${encodeURIComponent(id)}/${tab}`;}
function navigateSession(device,kind,id){location.hash=`session/${encodeURIComponent(device)}/${kind}/${id}`;}
function route(){
  if(!state.authenticated)return;
  const parts=location.hash.slice(1).split('/').map(p=>{try{return decodeURIComponent(p);}catch{return '';}});
  state.routeVersion++;
  const key=parts.slice(0,4).join('/');
  if(state.attachment&&state.attachment.key!==key)detachSession();
  if(parts[0]==='add-device'){
    showView('devices','Devices','Reachability and capability across your fleet.');
    if(!$('#enroll-dialog').open)$('#add-device').click();
  }else if(parts[0]==='device'&&parts[1]){
    if(state.device!==parts[1]){state.health=null;$('#permission-result').textContent='';$('#device-diagnostics').open=false;state.file=null;clearFileListing();$('#file-content').textContent='';$('#file-preview').classList.add('hidden');$('#device-status-output').classList.add('hidden');}
    state.device=parts[1];state.tab=['overview','terminal','files','sessions','activity'].includes(parts[2])?parts[2]:'overview';
    showView('device-detail',currentDevice()?.device_name||'Device','Device health and persistent work.');renderDevice();
  }else if(parts[0]==='session'&&parts.length===4&&['terminal','job','transfer'].includes(parts[2])){
    showView('session-detail','Session','Attach to persistent work on your device.');
    if(!state.attachment&&state.devices.length)attachSession(parts[1],parts[2],parts[3],key);
  }else{
    const titles={overview:['Overview','Your private control plane at a glance.'],devices:['Devices','Reachability and capability across your fleet.'],sessions:['Sessions','Persistent work that survives navigation and reconnects.'],activity:['Activity','A compact operational trail from this gateway.'],settings:['Settings','Account and dashboard session controls.']};
    const name=titles[parts[0]]?parts[0]:'overview';showView(name,...titles[name]);
  }
}
window.addEventListener('hashchange',route);
document.addEventListener('click',event=>{
  if(event.target.closest('[data-add-device]')){$('#add-device').click();return;}
  const session=event.target.closest('[data-session]');
  if(session){navigateSession(session.dataset.device,session.dataset.kind,session.dataset.session);return;}
  const device=event.target.closest('[data-device]');if(device)navigateDevice(device.dataset.device);
});
$$('[data-device-tab]').forEach(b=>b.addEventListener('click',()=>navigateDevice(state.device,b.dataset.deviceTab)));

function applyDeviceSearch(){
  const search=$('#device-search');
  if(!search)return;
  const query=search.value.trim().toLocaleLowerCase();
  const rows=$$('#all-devices .device-row');
  let found=0;
  rows.forEach(row=>{
    const id=row.querySelector('[data-device]')?.dataset.device;
    const device=state.devices.find(d=>d.device_id===id);
    const text=[device?.device_name,device?.platform,device?.arch,...(device?.capabilities||[])].join(' ').toLocaleLowerCase();
    const match=!query||text.includes(query);
    row.classList.toggle('hidden',!match);
    if(match)found++;
  });
  $('#device-search-empty')?.remove();
  if(query&&!found){
    const empty=document.createElement('p');
    empty.id='device-search-empty';
    empty.className='empty';
    empty.textContent='No matching devices. Try another name or platform.';
    $('#all-devices').append(empty);
  }
}
$('#device-search').addEventListener('input',applyDeviceSearch);
document.addEventListener('keydown',event=>{
  if(event.key!=='/'||event.metaKey||event.ctrlKey||event.altKey||event.isComposing||!state.authenticated)return;
  if(event.target instanceof HTMLInputElement||event.target instanceof HTMLTextAreaElement||event.target.isContentEditable)return;
  // Never hijack typing in an active xterm session.
  if(state.attachment?.kind==='terminal')return;
  event.preventDefault();setView('devices');$('#device-search').focus();
});
function renderSessionList(){
  const select=$('#session-device'),chosen=select.value;
  select.innerHTML='<option value="">All devices</option>'+state.devices.map(d=>`<option value="${esc(d.device_id)}">${esc(d.device_name)}</option>`).join('');select.value=chosen;
  const kind=$('#session-kind').value;
  const rows=state.sessions.filter(s=>(!select.value||s.device_id===select.value)&&(!kind||s.kind===kind));
  $('#all-sessions').innerHTML=rows.length?rows.map(sessionRow).join(''):emptyState('No sessions match these filters.','inbox');
}
$('#session-device').addEventListener('change',renderSessionList);$('#session-kind').addEventListener('change',renderSessionList);
function renderDevice(){
  if(!$('#device-detail').classList.contains('active'))return;
  const d=currentDevice();
  if(!d){$('#device-summary').textContent='Device unavailable. Sign in or return to Devices.';$$('.device-pane').forEach(p=>p.classList.add('hidden'));return;}
  $('#view-title').textContent=d.device_name;
  const status=deviceState(d);
  $('#device-summary').innerHTML=`<div class="device-hero-head"><div class="device-hero-avatar ${d.online&&!d.revoked?'online':''}">${platformIcon(d.platform)}</div><div class="device-hero-copy"><span class="eyebrow">DEVICE</span><h2>${esc(d.device_name)}</h2><p>${esc(d.platform)} · ${esc(d.arch)} · Infra ${esc(d.agent_version)}</p></div><span class="status ${status}">${status}</span></div><div class="device-metrics"><div class="device-metric"><span>Last seen</span><strong>${relative(d.last_seen_unix_ms)}</strong></div><div class="device-metric"><span>Terminals</span><strong>${d.active_terminals||0}</strong></div><div class="device-metric"><span>Jobs</span><strong>${d.active_jobs||0}</strong></div><div class="device-metric"><span>Transfers</span><strong>${d.active_transfers||0}</strong></div><div class="device-metric"><span>Device ID</span><code>${esc(d.device_id)}</code></div></div>${d.online?'':`<p class="small muted" style="margin-top:12px">${esc(reconnectExplanation(d))} Session state shown here is the last reported state.</p>`}`;
  const caps=new Set(d.capabilities||[]);
  const defs=[['Shell','exec','terminal'],['Files','files','folder'],['Terminal','terminal','terminal'],['Jobs','jobs','jobs'],['Transfers','transfer','transfer'],['System','system','cpu'],['Network','network','network'],['Services','services','layers'],['Docker','containers','package'],['Git','git','git'],['Desktop','desktop','monitor']];
  $('#device-capabilities').innerHTML=defs.map(([name,cap,iconName])=>{const iconSvg=icon(iconName);const advertised=caps.has(cap),availableNow=advertised&&d.online;return `<div class="capability-card ${availableNow?'available':''}"><div class="cap-icon">${iconSvg}</div><strong>${name}</strong><p>${advertised?(d.online?(cap==='desktop'?'Permission check needed':'Available'):'Device offline'):'Not advertised'}</p></div>`;}).join('');
  $$('#device-tabs .tab').forEach(t=>{const active=t.dataset.deviceTab===state.tab;t.classList.toggle('active',active);t.setAttribute('aria-pressed',String(active));});
  renderConnectionHealth(d);
  $$('.device-pane').forEach(p=>p.classList.toggle('hidden',p.id!==`device-${state.tab}`));
  $('#terminal-open button').disabled=!available();$('#device-status').disabled=!available()||(state.health?.device===d.device_id&&state.health.status==='checking');$('#device-permissions').classList.toggle('hidden',!caps.has('desktop'));$('#device-permissions').disabled=!available();$$('#files-browse button').forEach(b=>b.disabled=!available()||fileBrowseBusy);renderFilePaging();
  if(!$('#terminal-command').value||$('#terminal-command').dataset.device!==d.device_id){$('#terminal-command').value=d.platform==='windows'?'powershell.exe -NoLogo':d.platform==='macos'||d.platform==='darwin'?'/bin/zsh -l':'/bin/sh -i';$('#terminal-command').dataset.device=d.device_id;}
  const sessions=state.sessions.filter(s=>s.device_id===d.device_id);
  $('#device-sessions').innerHTML=sessions.length?sessions.map(sessionRow).join(''):emptyState('No sessions on this device.','terminal');
  const terminals=sessions.filter(s=>s.kind==='terminal');$('#device-terminals').innerHTML=terminals.length?terminals.map(sessionRow).join(''):emptyState('No terminals are running. Open one above.','terminal');
  const activity=state.activity.filter(a=>a.device_id===d.device_id);$('#device-activity').innerHTML=activity.length?activity.map(activityRow).join(''):emptyState('No recent activity for this device.','activity');
}
async function operation(device,op){
  if(!available(device))throw new Error('Device is offline or revoked. Wait for it to reconnect.');
  const response=await jsonFetch(`/api/devices/${encodeURIComponent(device)}/operations`,{method:'POST',body:JSON.stringify(op)});
  if(response.status!=='ok')throw new Error(`${response.status}: ${response.error||'The operation did not complete.'}${response.status==='outcome_unknown'?' Check the session before retrying.':''}`);
  return response.body;
}
async function deviceAction(button,fn){
  const version=state.routeVersion;button.disabled=true;$('#device-error').textContent='';
  try{await fn(version);}catch(error){if(version===state.routeVersion)$('#device-error').textContent=error.message;}
  finally{button.disabled=false;renderDevice();}
}
function renderConnectionHealth(device){
  const check=state.health?.device===device.device_id?state.health:null;
  const fresh=check?.status==='verified'&&pairProofFresh(check)&&device.online&&!device.revoked&&check.generation===device.connection_generation;
  const title=$('#health-title'),guidance=$('#health-guidance'),panel=$('#device-health');
  panel.classList.toggle('verified',!!fresh);
  if(device.revoked){title.textContent='Access removed';guidance.textContent='This computer is no longer allowed to connect. Pair it again only if you want to restore access.';}
  else if(!device.online){title.textContent='Computer unavailable';guidance.textContent='Wake this computer, check its network, and make sure Infra is running on it. Your sessions remain listed below.';}
  else if(check?.status==='checking'){title.textContent='Checking the connection…';guidance.textContent='Waiting for a response from this computer.';}
  else if(fresh){title.textContent='Connection verified';guidance.textContent='This computer responded to a fresh check. You can open a terminal or browse its files.';}
  else if(check?.status==='failed'){title.textContent='Could not verify the connection';guidance.textContent='The computer did not respond to the check. Check its network and Infra status, then try again.';}
  else if(Date.now()-state.lastRefresh>30000){title.textContent='Status may be out of date';guidance.textContent='The gateway has not refreshed this computer’s status. Check the connection before starting work.';}
  else{title.textContent='Connected to the gateway';guidance.textContent='Check the connection to confirm this computer can respond to a new request.';}
}
$('#device-status').addEventListener('click',()=>deviceAction($('#device-status'),async version=>{
  const device=currentDevice(),epoch=state.authEpoch;
  if(!device)return;
  const current=()=>version===state.routeVersion&&epoch===state.authEpoch&&state.authenticated&&state.device===device.device_id;
  state.health={device:device.device_id,status:'checking'};renderConnectionHealth(device);
  try{
    const start=performance.now(),result=await operation(device.device_id,{op:'status'});
    if(!current())return;
    if(!Number.isInteger(result.pid)||result.pid<=0||!Number.isInteger(result.protocol)||result.protocol<=0)throw new Error('Invalid status response');
    state.health={device:device.device_id,status:'verified',generation:device.connection_generation,verifiedAt:performance.now(),verifiedWall:Date.now()};
    const proof=state.health;setTimeout(()=>{if(state.health===proof&&current())renderConnectionHealth(currentDevice());},PAIR_PROOF_MS);
    $('#device-status-output').textContent=`Response: ${Math.round(performance.now()-start)} ms\nPID: ${result.pid}\nProtocol: ${result.protocol}`;
    $('#device-status-output').classList.remove('hidden');
  }catch(error){if(current()){state.health={device:device.device_id,status:'failed'};$('#device-status-output').textContent=error.message;$('#device-status-output').classList.remove('hidden');}}
}));
$('#device-permissions').addEventListener('click',()=>deviceAction($('#device-permissions'),async version=>{
  const result=await operation(state.device,{op:'desktop',action:{action:'permissions'}});
  if(version!==state.routeVersion)return;
  const checks=Object.entries(result).filter(([,v])=>v&&typeof v==='object'&&'granted'in v);
  const lines=checks.map(([name,value])=>`${name.replaceAll('_',' ')}: ${value.granted?'Granted':'Needs permission'}`);
  if(result.ready===false)lines.push('Grant missing permissions locally in your OS privacy/security settings, then check again.');
  $('#permission-result').textContent=lines.join('\n')||'No permission information is available from this computer.';
}));
$('#terminal-open').addEventListener('submit',event=>{
  event.preventDefault();const button=$('#terminal-open button'),device=state.device;
  deviceAction(button,async version=>{
    const result=await operation(device,{op:'terminal',action:{action:'open',command:$('#terminal-command').value,cwd:$('#terminal-cwd').value.trim()||null,cols:100,rows:28}});
    await refresh();if(version===state.routeVersion)navigateSession(device,'terminal',result.session.id);
  });
});

function clearPrivateViews(){
  state.health=null;$('#permission-result').textContent='';$('#device-diagnostics').open=false;
  stopPairPolling();pairFlow=null;$('#pending-pairs').replaceChildren();$('#pair-result').textContent='';$('#pair-error').textContent='';renderPairFlow();
  state.authEpoch++;state.refreshPending=false;state.devices=[];state.sessions=[];state.activity=[];state.file=null;state.enrollment=null;detachSession();
  clearFileListing();
  ['#file-content','#files-list','#session-output','#device-status-output','#enrollment-code','#install-command'].forEach(s=>$(s).replaceChildren());
  $('#file-preview').classList.add('hidden');$('#enroll-dialog').close();render();
}
function detachSession(){
  const a=state.attachment;if(!a)return;
  a.disposed=true;clearTimeout(a.timer);clearTimeout(a.inputTimer);a.input='';a.term?.dispose();state.attachment=null;$('#terminal-screen').replaceChildren();$('#session-output').textContent='';
}
function sessionError(a,error){if(state.attachment!==a)return;$('#session-error').textContent=error.message;$('#session-resume').classList.remove('hidden');}
function attachSession(device,kind,id,key){
  const a={device,kind,id,key,offset:0,disposed:false,paused:false,input:'',writeBusy:false,term:null};state.attachment=a;
  $('#session-heading').textContent=`${kind} · ${currentDevice(device)?.device_name||device}`;
  $('#session-meta').textContent=id;$('#session-output').textContent='';$('#session-error').textContent='';$('#session-read-state').textContent='Connecting…';$('#session-resume').classList.add('hidden');
  $('#terminal-tools').classList.toggle('hidden',kind!=='terminal');$('#terminal-screen').classList.toggle('hidden',kind!=='terminal');$('#session-output').classList.toggle('hidden',kind==='terminal');
  $('#session-stop').textContent=kind==='terminal'?'Close terminal':kind==='job'?'Cancel job':'Cancel transfer';$('#session-stop').disabled=!available(device);
  if(kind==='terminal'){
    a.term=new Terminal({cols:100,rows:28,cursorBlink:true,fontSize:13,scrollback:3000,theme:{background:'#0b100e',foreground:'#e7eee8',cursor:'#91c3a1',selectionBackground:'#304c3a'},allowProposedApi:false});
    a.term.open($('#terminal-screen'));a.term.onData(data=>queueInput(a,data));a.term.focus();
  }
  readSession(a);
}
function sessionOperation(a,action,fields={}){
  return operation(a.device,{op:a.kind==='terminal'?'terminal':a.kind==='job'?'jobs':'transfer',action:{action,[a.kind==='terminal'?'session_id':a.kind==='job'?'job_id':'transfer_id']:a.id,...fields}});
}
async function readSession(a){
  if(a.disposed||state.attachment!==a||a.readBusy)return;
  a.readBusy=true;let delay=700;
  try{
    const result=await sessionOperation(a,a.kind==='transfer'?'status':'read',{offset:a.offset,length:32768});
    if(a.disposed||state.attachment!==a)return;
    const meta=result.session||result.job||result.transfer||result,finished=meta.status&&!['running','starting','canceling','in_progress','uploading','downloading','ready'].includes(meta.status);
    $('#session-meta').textContent=`${a.id} · ${meta.status||'active'}`;
    $('#session-stop').disabled=!!finished||!available(a.device);
    if(a.kind==='transfer'){$('#session-output').textContent=JSON.stringify(result,null,2);delay=2000;}
    else{
      if(result.offset_expired){a.term?.reset();$('#session-read-state').textContent='Earlier output expired; showing retained history.';}
      if(result.output){if(a.term)await new Promise(resolve=>a.term.write(result.output,resolve));else{$('#session-output').textContent=($('#session-output').textContent+result.output).slice(-262144);$('#session-output').scrollTop=$('#session-output').scrollHeight;}}
      a.offset=result.next_offset;delay=result.has_more?30:700;
    }
    if(!result.offset_expired)$('#session-read-state').textContent=finished?'Session finished. Retained output is available.':`Connected · ${a.kind==='transfer'?'status refreshed':`${a.offset} bytes read`}`;
    if(finished&&!result.has_more){a.paused=true;return;}
  }catch(error){sessionError(a,error);$('#session-read-state').textContent='Waiting for connection. Input is paused.';a.paused=true;a.input='';delay=3000;}
  finally{a.readBusy=false;}
  if(!a.disposed)a.timer=setTimeout(()=>readSession(a),delay);
}
function queueInput(a,data){
  if(a.disposed||a.paused||!available(a.device))return;
  if(a.input.length+data.length>65536){a.input='';a.paused=true;sessionError(a,new Error('Input is too large. Input was discarded; resume before typing again.'));return;}
  a.input+=data;clearTimeout(a.inputTimer);a.inputTimer=setTimeout(()=>flushInput(a),30);
}
async function flushInput(a){
  if(a.disposed||a.paused||a.writeBusy||!a.input)return;
  const data=a.input;a.input='';a.writeBusy=true;
  try{await sessionOperation(a,'write',{data});}
  catch(error){a.paused=true;a.input='';sessionError(a,error);}
  finally{a.writeBusy=false;if(a.input&&!a.disposed&&!a.paused)a.inputTimer=setTimeout(()=>flushInput(a),0);}
}
$('#session-resume').addEventListener('click',()=>{const a=state.attachment;if(!a)return;a.paused=false;$('#session-error').textContent='';$('#session-resume').classList.add('hidden');a.term?.focus();clearTimeout(a.timer);a.timer=setTimeout(()=>readSession(a),0);});
$('#terminal-interrupt').addEventListener('click',()=>{const a=state.attachment;if(a?.kind==='terminal')queueInput(a,'\x03');});
$('#terminal-resize').addEventListener('click',async()=>{
  const a=state.attachment;if(!a||a.kind!=='terminal'||a.paused)return;
  const cols=Number($('#terminal-cols').value),rows=Number($('#terminal-rows').value);
  if(!Number.isInteger(cols)||!Number.isInteger(rows)||cols<20||cols>300||rows<5||rows>100){sessionError(a,new Error('Use 20–300 columns and 5–100 rows.'));return;}
  try{await sessionOperation(a,'resize',{cols,rows});if(!a.disposed)a.term.resize(cols,rows);}catch(error){sessionError(a,error);}
});
$('#session-stop').addEventListener('click',async()=>{
  const a=state.attachment;if(!a)return;
  const verb=a.kind==='terminal'?'Close terminal':`Cancel ${a.kind}`;
  if(!confirm(`${verb} ${a.id} on ${currentDevice(a.device)?.device_name||a.device}? This ends its running work.`))return;
  $('#session-stop').disabled=true;
  try{await sessionOperation(a,a.kind==='terminal'?'close':'cancel');a.paused=true;await refresh();if(!a.disposed)$('#session-read-state').textContent='Stop requested. Waiting for device confirmation.';}
  catch(error){sessionError(a,error);if(!a.disposed)$('#session-stop').disabled=false;}
});

let fileRequest=0,fileBrowseBusy=false;
function clearFileListing(){
  fileRequest++;fileBrowseBusy=false;state.listing=null;$('#files-list').replaceChildren();$('#files-note').textContent='';$('#files-path').value='.';renderFilePaging();
}
function renderFilePaging(){
  const listing=state.listing,visible=listing?.device===state.device&&listing.paging;
  $('#files-pagination').classList.toggle('hidden',!visible);
  const enabled=visible&&available()&&!fileBrowseBusy&&(currentDevice()?.capabilities||[]).includes('files.list_page');
  $('#files-previous').disabled=!enabled||!listing?.history.length;
  $('#files-next').disabled=!enabled||!listing?.hasMore;
}
async function browseFiles(path,page={offset:0,history:[]}){
  const request=++fileRequest,device=state.device,version=state.routeVersion;
  const paging=(currentDevice()?.capabilities||[]).includes('files.list_page');
  fileBrowseBusy=true;$('#device-error').textContent='';$('#files-note').textContent='Loading…';renderDevice();
  try{
    const action={action:'list',path};if(paging){action.offset=page.offset;action.length=100;}
    const result=await operation(device,{op:'files',action});
    if(request!==fileRequest||version!==state.routeVersion)return;
    const entries=[...result.entries],offset=Number.isSafeInteger(result.offset)?result.offset:page.offset;
    const next=result.next_offset,canContinue=paging&&result.pagination_supported===true&&Number.isSafeInteger(next)&&next>offset&&next<=100000;
    state.listing={device,path:result.path,offset,history:page.history,next,hasMore:canContinue&&result.has_more===true,paging};
    const range=entries.length?`${offset+1}–${offset+entries.length}`:'0';
    $('#files-path').value=result.path;
    $('#files-note').textContent=paging?`Showing ${range}${Number.isSafeInteger(result.total_entries)?` of ${result.total_entries}`:''} entries. Browse again to refresh changed directories.${result.truncated&&!canContinue?' More entries exist, but continuation is unavailable; check the agent version.':''}`:`${entries.length} entries${result.truncated?' · Listing incomplete. Update this agent for page navigation or open a smaller directory.':''}`;
    if(!paging)entries.sort((a,b)=>(a.kind==='directory'?0:1)-(b.kind==='directory'?0:1)||a.name.localeCompare(b.name));
    $('#files-list').innerHTML=entries.length?entries.map(e=>`<div class="file-row"><button class="device-link" data-file-path="${esc(e.path)}" data-file-kind="${esc(e.kind)}">${icon(e.kind==='directory'?'folder':'file')}${esc(e.name)}</button><span class="muted">${esc(e.kind)}</span><span class="muted">${e.size==null?'':`${e.size.toLocaleString()} B`}</span></div>`).join(''):emptyState(offset?'No entries at this position. Browse again if the directory changed.':'This directory is empty.','folder');
  }catch(error){if(request===fileRequest&&version===state.routeVersion){$('#files-note').textContent=state.listing?`Showing the last confirmed page for ${state.listing.path}.`:'Directory listing unavailable.';$('#device-error').textContent=error.message;}}
  finally{if(request===fileRequest){fileBrowseBusy=false;if(version!==state.routeVersion)$('#files-note').textContent=state.listing?`Showing the last confirmed page for ${state.listing.path}. Browse again to refresh.`:'';renderDevice();}}
}
$('#files-next').addEventListener('click',()=>{const listing=state.listing;if(!fileBrowseBusy&&listing?.hasMore)browseFiles(listing.path,{offset:listing.next,history:[...listing.history,listing.offset]});});
$('#files-previous').addEventListener('click',()=>{const listing=state.listing;if(!fileBrowseBusy&&listing?.history.length)browseFiles(listing.path,{offset:listing.history.at(-1),history:listing.history.slice(0,-1)});});
$('#files-browse').addEventListener('submit',event=>{event.preventDefault();browseFiles($('#files-path').value);});
$('#files-up').addEventListener('click',()=>{const path=$('#files-path').value,separator=path.includes('\\')?'\\':'/';browseFiles(path.replace(/[\\/]$/,'')+separator+'..');});
$('#files-list').addEventListener('click',event=>{const b=event.target.closest('[data-file-path]');if(!b)return;if(b.dataset.fileKind==='directory')browseFiles(b.dataset.filePath);else{state.file={device:state.device,path:b.dataset.filePath,offset:0};previewFile(state.file);}});
async function previewFile(file){
  const version=state.routeVersion;$('#device-error').textContent='';
  try{
    const result=await operation(file.device,{op:'files',action:{action:'read',path:file.path,offset:file.offset,length:32768}});
    if(state.file!==file||version!==state.routeVersion)return;
    $('#file-preview').classList.remove('hidden');$('#file-name').textContent=file.path;$('#file-content').textContent=result.content;$('#file-range').textContent=`Bytes ${result.offset}–${result.next_offset} of ${result.size}`;file.offset=result.next_offset;$('#file-next').classList.toggle('hidden',!result.has_more);
  }catch(error){if(state.file===file&&version===state.routeVersion){$('#file-preview').classList.add('hidden');$('#file-content').textContent='';$('#device-error').textContent=error.message;}}
}
$('#file-close').addEventListener('click',()=>{state.file=null;$('#file-preview').classList.add('hidden');$('#file-content').textContent='';});
$('#file-next').addEventListener('click',()=>{if(state.file)previewFile(state.file);});
async function bootstrap(){if(await authStatus()){await refresh();route();connectEvents();}}
bootstrap();setInterval(()=>{if(!state.streamHealthy||Date.now()-state.lastRefresh>60000)refresh();},15000);
