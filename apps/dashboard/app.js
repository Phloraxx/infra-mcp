const state={devices:[],sessions:[],activity:[],platform:'unix',enrollment:null,device:null,tab:'overview',attachment:null,file:null,routeVersion:0,authEpoch:0};
const $=s=>document.querySelector(s);const $$=s=>[...document.querySelectorAll(s)];
const esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
async function jsonFetch(url,options={}){const response=await fetch(url,{...options,credentials:'same-origin',headers:{'Content-Type':'application/json','X-Infra-CSRF':'1',...(options.headers||{})}});if(response.status===401){clearPrivateViews();setView('settings');const error=new Error('Authentication required. Sign in under Settings.');error.status=401;throw error;}if(!response.ok){const text=await response.text();let message=text;try{message=JSON.parse(text).error||text;}catch{}throw new Error(message||`HTTP ${response.status}`);}return response.status===204?null:response.json();}
async function authStatus(){try{const r=await fetch('/api/auth/session',{credentials:'same-origin'});const v=await r.json();$('#auth-state').textContent=v.authenticated?`Authenticated · ${v.mode.replaceAll('_',' ')}`:'Authentication required';return v.authenticated;}catch{$('#auth-state').textContent='Authentication status unavailable';return false;}}
function showView(name,title,subtitle){$$('.view').forEach(v=>v.classList.toggle('active',v.id===name));$$('.nav').forEach(v=>v.classList.toggle('active',v.dataset.view===name));$('#view-title').textContent=title;$('#view-subtitle').textContent=subtitle;}
function setView(name){location.hash=name;route();}
$$('.nav').forEach(b=>b.addEventListener('click',()=>setView(b.dataset.view)));
$$('[data-view-target]').forEach(b=>b.addEventListener('click',()=>setView(b.dataset.viewTarget)));
function relative(ms){if(!ms)return 'never';const sec=Math.max(0,Math.floor((Date.now()-ms)/1000));if(sec<5)return 'just now';if(sec<60)return `${sec}s ago`;if(sec<3600)return `${Math.floor(sec/60)}m ago`;if(sec<86400)return `${Math.floor(sec/3600)}h ago`;return `${Math.floor(sec/86400)}d ago`;}
function deviceRow(d){const status=d.revoked?'revoked':d.online?'online':'offline';const work=(d.active_jobs||0)+(d.active_terminals||0)+(d.active_transfers||0);return `<div class="device-row"><div><button class="device-link" data-device="${esc(d.device_id)}">${esc(d.device_name)}</button><div class="device-meta">${esc(d.platform)} · ${esc(d.arch)} · ${d.online?'connected':`last seen ${relative(d.last_seen_unix_ms)}`}</div></div><div class="caps">${(d.capabilities||[]).slice(0,6).map(c=>`<span class="pill">${esc(c)}</span>`).join('')}</div><div class="work-count device-meta">${work?`${work} active`:''}</div><div class="status ${status}">${status[0].toUpperCase()+status.slice(1)}</div></div>`;}
function sessionRow(s){return `<div class="session-row"><div><button class="device-link" data-session="${esc(s.id)}" data-device="${esc(s.device_id)}" data-kind="${esc(s.kind)}">${esc(s.kind)} · ${esc(s.device_name)}</button><div class="session-meta">${esc(s.id)} · ${esc(s.status)} · started ${relative(s.started_at_unix_ms)}${s.device_online?'':' · device offline'}</div></div><span class="pill">${esc(s.status)}</span></div>`;}
function activityRow(a){return `<div class="activity-row"><div><div class="session-title">${esc(a.action)}${a.device_name?` · ${esc(a.device_name)}`:''}</div><div class="session-meta">${relative(a.at_unix_ms)} · ${esc(a.outcome)}${a.request_id?` · ${esc(a.request_id)}`:''}</div></div></div>`;}
function activeSession(s){return ['running','starting','canceling','uploading','downloading','ready'].includes(s.status);}
function render(){const online=state.devices.filter(d=>d.online&&!d.revoked).length,offline=state.devices.filter(d=>!d.online&&!d.revoked).length,revoked=state.devices.filter(d=>d.revoked).length,active=state.sessions.filter(activeSession).length;$('#summary').innerHTML=[['Online',online],['Offline',offline],['Active work',active],['Enrolled',state.devices.length-revoked]].map(([k,v])=>`<div class="summary-card"><span>${k}</span><strong>${v}</strong></div>`).join('');const devices=state.devices.length?state.devices.map(deviceRow).join(''):'<div class="empty">No devices enrolled yet.</div>';$('#all-devices').innerHTML=devices;$('#overview-devices').innerHTML=state.devices.length?state.devices.slice(0,5).map(deviceRow).join(''):'<div class="empty">Add your first device to get started.</div>';const sessions=state.sessions.filter(activeSession);renderSessionList();$('#overview-sessions').innerHTML=sessions.length?sessions.slice(0,6).map(sessionRow).join(''):'<div class="empty">Nothing is running right now.</div>';$('#activity-list').innerHTML=state.activity.length?state.activity.map(activityRow).join(''):'<div class="empty">No activity recorded yet.</div>';const attention=[];state.devices.filter(d=>!d.revoked&&!d.online).forEach(d=>attention.push(`<div class="attention-row"><div><div class="session-title">${esc(d.device_name)} is offline</div><div class="session-meta">Last seen ${relative(d.last_seen_unix_ms)}. Infra will reconnect automatically when the agent is reachable.</div></div></div>`));$('#attention').innerHTML=attention.join('');$('#attention-section').classList.toggle('hidden',attention.length===0);renderDevice();}
async function refresh(){const epoch=state.authEpoch;try{const [devices,sessions,activity]=await Promise.all([jsonFetch('/api/devices'),jsonFetch('/api/sessions'),jsonFetch('/api/activity')]);if(epoch!==state.authEpoch)return;state.devices=devices;state.sessions=sessions;state.activity=activity;$('#gateway-dot').classList.add('online');$('#gateway-label').textContent='Gateway online';render();if(location.hash.startsWith('#session/')&&!state.attachment)route();}catch(error){$('#gateway-dot').classList.remove('online');if(error.status===401){$('#gateway-label').textContent='Sign in required';$('#auth-state').textContent='Authentication required';setView('settings');}else{$('#gateway-label').textContent='Gateway unavailable';console.error(error);}}}
function connectEvents(){const events=new EventSource('/api/events');events.onmessage=()=>refresh();events.onerror=()=>{$('#gateway-dot').classList.remove('online');setTimeout(()=>refresh(),1500);};}
$('#add-device').addEventListener('click',()=>{$('#enroll-start').classList.remove('hidden');$('#enroll-result').classList.add('hidden');$('#enroll-dialog').showModal();});
$('#create-enrollment').addEventListener('click',async()=>{const button=$('#create-enrollment');button.disabled=true;button.textContent='Creating…';try{state.enrollment=await jsonFetch('/api/enrollments',{method:'POST',body:JSON.stringify({ttl_seconds:300})});$('#enrollment-code').textContent=state.enrollment.code;$('#enrollment-expiry').textContent=`Expires ${new Date(state.enrollment.expires_at_unix_ms).toLocaleTimeString()}`;$('#enroll-start').classList.add('hidden');$('#enroll-result').classList.remove('hidden');updateInstallCommand();}catch(error){alert(error.message);}finally{button.disabled=false;button.textContent='Create enrollment';}});
function gatewayWs(){const url=new URL(location.href);url.protocol=url.protocol==='https:'?'wss:':'ws:';url.pathname='/agent';url.search='';url.hash='';return url.toString();}
function updateInstallCommand(){if(!state.enrollment)return;const code=state.enrollment.code;const name=$('#device-name').value.trim();const nameArg=name?` --name ${shellQuote(name)}`:'';if(state.platform==='unix'){const script=`${location.origin}/downloads/install.sh`;$('#install-command').textContent=`curl -fsSL ${shellQuote(script)} | sh -s -- --gateway ${shellQuote(gatewayWs())} --code ${shellQuote(code)}${nameArg}`;}else{const script=`${location.origin}/downloads/install.ps1`;const psName=name?` -Name ${psQuote(name)}`:'';$('#install-command').textContent=`& ([scriptblock]::Create((irm ${psQuote(script)}))) -Gateway ${psQuote(gatewayWs())} -Code ${psQuote(code)}${psName}`;}}
function shellQuote(s){return `'${String(s).replaceAll("'","'\\''")}'`; }function psQuote(s){return `'${String(s).replaceAll("'","''")}'`;}
$$('.tab').forEach(tab=>tab.addEventListener('click',()=>{state.platform=tab.dataset.platform;$$('.tab').forEach(t=>t.classList.toggle('active',t===tab));updateInstallCommand();}));$('#device-name').addEventListener('input',updateInstallCommand);$('#copy-command').addEventListener('click',async()=>{await navigator.clipboard.writeText($('#install-command').textContent);$('#copy-command').textContent='Copied';setTimeout(()=>$('#copy-command').textContent='Copy install command',1200);});
$('#save-token').addEventListener('click',async()=>{const button=$('#save-token'),input=$('#admin-token'),value=input.value.trim();if(!value){$('#auth-state').textContent='Enter the admin credential.';return;}button.disabled=true;button.textContent='Signing in…';try{await jsonFetch('/api/auth/session',{method:'POST',body:JSON.stringify({token:value})});input.value='';await authStatus();await refresh();}catch(error){$('#auth-state').textContent=error.message;}finally{button.disabled=false;button.textContent='Sign in';}});
$('#logout-session').addEventListener('click',async()=>{try{await fetch('/api/auth/session',{method:'DELETE',credentials:'same-origin',headers:{'X-Infra-CSRF':'1'}});}finally{clearPrivateViews();$('#admin-token').value='';await authStatus();await refresh();}});


function currentDevice(id=state.device){return state.devices.find(d=>d.device_id===id);}
function available(id){const d=currentDevice(id);return !!d?.online&&!d.revoked;}
function navigateDevice(id,tab='overview'){location.hash=`device/${encodeURIComponent(id)}/${tab}`;}
function navigateSession(device,kind,id){location.hash=`session/${encodeURIComponent(device)}/${kind}/${id}`;}
function route(){
  const parts=location.hash.slice(1).split('/').map(p=>{try{return decodeURIComponent(p);}catch{return '';}});
  state.routeVersion++;
  const key=parts.slice(0,4).join('/');
  if(state.attachment&&state.attachment.key!==key)detachSession();
  if(parts[0]==='device'&&parts[1]){
    if(state.device!==parts[1]){state.file=null;$('#file-content').textContent='';$('#file-preview').classList.add('hidden');$('#files-list').replaceChildren();$('#files-path').value='.';$('#device-status-output').classList.add('hidden');}
    state.device=parts[1];state.tab=['overview','terminal','files','sessions','activity'].includes(parts[2])?parts[2]:'overview';
    showView('device-detail',currentDevice()?.device_name||'Device','Device health and persistent work.');renderDevice();
  }else if(parts[0]==='session'&&parts.length===4&&['terminal','job','transfer'].includes(parts[2])){
    showView('session-detail','Session','Attach to persistent work on your device.');
    if(!state.attachment&&state.devices.length)attachSession(parts[1],parts[2],parts[3],key);
  }else{
    const titles={overview:['Overview','Your devices and active work.'],devices:['Devices','Reachability, capabilities and persistent state.'],sessions:['Sessions','Jobs, terminals and transfers across devices.'],activity:['Activity','Bounded operational metadata.'],settings:['Settings','Gateway and dashboard authentication.']};
    const name=titles[parts[0]]?parts[0]:'overview';showView(name,...titles[name]);
  }
}
window.addEventListener('hashchange',route);
document.addEventListener('click',event=>{
  const session=event.target.closest('[data-session]');
  if(session){navigateSession(session.dataset.device,session.dataset.kind,session.dataset.session);return;}
  const device=event.target.closest('[data-device]');if(device)navigateDevice(device.dataset.device);
});
$$('[data-device-tab]').forEach(b=>b.addEventListener('click',()=>navigateDevice(state.device,b.dataset.deviceTab)));

function renderSessionList(){
  const select=$('#session-device'),chosen=select.value;
  select.innerHTML='<option value="">All devices</option>'+state.devices.map(d=>`<option value="${esc(d.device_id)}">${esc(d.device_name)}</option>`).join('');select.value=chosen;
  const kind=$('#session-kind').value;
  const rows=state.sessions.filter(s=>(!select.value||s.device_id===select.value)&&(!kind||s.kind===kind));
  $('#all-sessions').innerHTML=rows.length?rows.map(sessionRow).join(''):'<div class="empty">No sessions match these filters.</div>';
}
$('#session-device').addEventListener('change',renderSessionList);$('#session-kind').addEventListener('change',renderSessionList);
function renderDevice(){
  if(!$('#device-detail').classList.contains('active'))return;
  const d=currentDevice();
  if(!d){$('#device-summary').textContent='Device unavailable. Sign in or return to Devices.';$$('.device-pane').forEach(p=>p.classList.add('hidden'));return;}
  $('#view-title').textContent=d.device_name;
  const status=d.revoked?'revoked':d.online?'online':'offline';
  $('#device-summary').innerHTML=`<div class="section-head"><div><h2>${esc(d.device_name)}</h2><p class="small">${esc(d.platform)} · ${esc(d.arch)} · Infra ${esc(d.agent_version)}</p></div><span class="status ${status}">${status}</span></div><dl class="detail-grid"><div><dt>Last seen</dt><dd>${relative(d.last_seen_unix_ms)}</dd></div><div><dt>Terminals</dt><dd>${d.active_terminals||0}</dd></div><div><dt>Jobs</dt><dd>${d.active_jobs||0}</dd></div><div><dt>Transfers</dt><dd>${d.active_transfers||0}</dd></div><div><dt>Device ID</dt><dd class="mono">${esc(d.device_id)}</dd></div></dl>${d.online?'':'<p class="small">Device controls become available when the agent reconnects. Session state is last reported.</p>'}`;
  const caps=new Set(d.capabilities||[]);
  $('#device-capabilities').innerHTML=[['Shell','exec'],['Files','files'],['Terminal','terminal'],['Jobs','jobs'],['Transfers','transfer'],['Desktop','desktop'],['Elevated control','privileged_helper']].map(([name,cap])=>`<div><span class="muted">${name}</span><p>${caps.has(cap)?d.online?'Available':'Device offline':'Not advertised'}</p></div>`).join('');
  $$('#device-tabs .tab').forEach(t=>t.classList.toggle('active',t.dataset.deviceTab===state.tab));
  $$('.device-pane').forEach(p=>p.classList.toggle('hidden',p.id!==`device-${state.tab}`));
  $('#terminal-open button').disabled=!available();$('#device-status').disabled=!available();
  $$('#files-browse button').forEach(b=>b.disabled=!available());
  if(!$('#terminal-command').value||$('#terminal-command').dataset.device!==d.device_id){$('#terminal-command').value=d.platform==='windows'?'powershell.exe -NoLogo':d.platform==='macos'||d.platform==='darwin'?'/bin/zsh -l':'/bin/sh -i';$('#terminal-command').dataset.device=d.device_id;}
  const sessions=state.sessions.filter(s=>s.device_id===d.device_id);
  $('#device-sessions').innerHTML=sessions.length?sessions.map(sessionRow).join(''):'<div class="empty">No active sessions on this device.</div>';
  const terminals=sessions.filter(s=>s.kind==='terminal');$('#device-terminals').innerHTML=terminals.length?terminals.map(sessionRow).join(''):'<div class="empty">No active terminals. Open one above.</div>';
  const activity=state.activity.filter(a=>a.device_id===d.device_id);$('#device-activity').innerHTML=activity.length?activity.map(activityRow).join(''):'<div class="empty">No recent activity for this device.</div>';
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
$('#device-status').addEventListener('click',()=>deviceAction($('#device-status'),async version=>{
  const start=performance.now(),result=await operation(state.device,{op:'status'});
  if(version!==state.routeVersion)return;
  $('#device-status-output').textContent=`Agent responded in ${Math.round(performance.now()-start)} ms\nPID: ${result.pid}\nProtocol: ${result.protocol}`;$('#device-status-output').classList.remove('hidden');
}));
$('#terminal-open').addEventListener('submit',event=>{
  event.preventDefault();const button=$('#terminal-open button'),device=state.device;
  deviceAction(button,async version=>{
    const result=await operation(device,{op:'terminal',action:{action:'open',command:$('#terminal-command').value,cwd:$('#terminal-cwd').value.trim()||null,cols:100,rows:28}});
    await refresh();if(version===state.routeVersion)navigateSession(device,'terminal',result.session.id);
  });
});

function clearPrivateViews(){
  state.authEpoch++;state.devices=[];state.sessions=[];state.activity=[];state.file=null;state.enrollment=null;detachSession();
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
    a.term=new Terminal({cols:100,rows:28,cursorBlink:true,fontSize:13,scrollback:3000,theme:{background:'#151515',foreground:'#eeeeee'},allowProposedApi:false});
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

let fileRequest=0;
async function browseFiles(path){
  const request=++fileRequest,device=state.device,version=state.routeVersion;$('#device-error').textContent='';$('#files-note').textContent='Loading…';
  try{
    const result=await operation(device,{op:'files',action:{action:'list',path}});
    if(request!==fileRequest||version!==state.routeVersion)return;
    $('#files-path').value=result.path;$('#files-note').textContent=`${result.entries.length} entries${result.truncated?' · Listing limited to 1,000 entries. Open a smaller directory.':''}`;
    const entries=[...result.entries].sort((a,b)=>(a.kind==='directory'?0:1)-(b.kind==='directory'?0:1)||a.name.localeCompare(b.name));
    $('#files-list').innerHTML=entries.length?entries.map(e=>`<div class="file-row"><button class="device-link" data-file-path="${esc(e.path)}" data-file-kind="${esc(e.kind)}">${esc(e.name)}</button><span class="muted">${esc(e.kind)}</span><span class="muted">${e.size==null?'':`${e.size.toLocaleString()} B`}</span></div>`).join(''):'<div class="empty">This directory is empty.</div>';
  }catch(error){if(request===fileRequest&&version===state.routeVersion){$('#files-note').textContent='';$('#files-list').replaceChildren();$('#device-error').textContent=error.message;}}
}
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
route();authStatus();refresh();connectEvents();setInterval(refresh,15000);
