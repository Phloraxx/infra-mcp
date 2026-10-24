#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
TMP="$(mktemp -d)"
GW_PID=''
SETUP_PID=''
AGENT_PID=''
cleanup(){
  for pid in "$SETUP_PID" "$AGENT_PID" "$GW_PID"; do
    [ -z "$pid" ] || kill "$pid" 2>/dev/null || true
    [ -z "$pid" ] || wait "$pid" 2>/dev/null || true
  done
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

cargo build -q -p infra-gateway -p infra-agent -p infra-cli
PORT="$(python3 - <<'PY'
import socket
s=socket.socket();s.bind(('127.0.0.1',0));print(s.getsockname()[1]);s.close()
PY
)"
TOKEN="$(python3 - <<'PY'
import secrets
print(secrets.token_urlsafe(32))
PY
)"
mkdir -p "$TMP/home"
INFRA_GATEWAY_BIND="127.0.0.1:$PORT" INFRA_DB_PATH="$TMP/infra.db" INFRA_ADMIN_TOKEN="$TOKEN" \
 target/debug/infra-gateway >"$TMP/gateway.log" 2>&1 &
GW_PID=$!
for _ in $(seq 1 120); do
  curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && break
  sleep .05
done
curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null

HOME="$TMP/home" target/debug/infra setup \
 --gateway "ws://127.0.0.1:$PORT/agent" \
 --agent "$ROOT/target/debug/infra-agent" \
 --state-dir "$TMP/state" --no-startup >"$TMP/setup.log" 2>&1 &
SETUP_PID=$!

python3 - "$PORT" "$TOKEN" <<'PY'
import json,sys,time,urllib.request,urllib.error
port,token=sys.argv[1:]
url=f"http://127.0.0.1:{port}"
h={'Content-Type':'application/json','Authorization':f'Bearer {token}','X-Infra-CSRF':'1'}
def call(path,method='GET',data=None,auth=True):
    req=urllib.request.Request(url+path,method=method,headers=h if auth else {'Content-Type':'application/json'},
                               data=json.dumps(data).encode() if data is not None else None)
    with urllib.request.urlopen(req,timeout=4) as r: return json.load(r)
try:
    call('/api/pair/requests',auth=False)
    raise AssertionError('unprotected owner pairing requests!')
except urllib.error.HTTPError as e:
    assert e.code==401,e
deadline=time.monotonic()+35
while True:
    pairs=call('/api/pair/requests')
    if pairs:break
    if time.monotonic()>deadline:raise AssertionError('device did not request pairing')
    time.sleep(.2)
assert len(pairs)==1,pairs
pair=pairs[0]
assert pair['state']=='pending'
try:
    call('/api/pair/'+pair['request_id']+'/approve','POST',{'verification_code':pair['verification_code']},auth=False)
    raise AssertionError('unauthenticated approval accepted!')
except urllib.error.HTTPError as e:assert e.code==401,e
try:
    call('/api/pair/'+pair['request_id']+'/approve','POST',{'verification_code':'00000x'})
    raise AssertionError('invalid code accepted!')
except urllib.error.HTTPError as e:assert e.code==400,e
wrong=('9' if pair['verification_code'][0]!='9' else '8')+pair['verification_code'][1:]
try:
    call('/api/pair/'+pair['request_id']+'/approve','POST',{'verification_code':wrong})
    raise AssertionError('incorrect comparison code accepted!')
except urllib.error.HTTPError as e:assert e.code==403,e
try:
    call('/pair/status','POST',{'request_id':pair['request_id'],'public_key':'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA','signature':'not-a-valid-signature'},auth=False)
    raise AssertionError('unsigned device status accepted!')
except urllib.error.HTTPError as e:assert e.code==401,e
approved=call('/api/pair/'+pair['request_id']+'/approve','POST',{'verification_code':pair['verification_code']})
assert approved['state']=='approved' and approved['device_id'],approved
retry=call('/api/pair/'+pair['request_id']+'/approve','POST',{'verification_code':pair['verification_code']})
assert retry['device_id']==approved['device_id']
print('Pairing owner approval PASS (protected API, correct code, idempotency).')
PY

for _ in $(seq 1 120); do
  if ! kill -0 "$SETUP_PID" 2>/dev/null; then break; fi
  sleep .1
done
if kill -0 "$SETUP_PID" 2>/dev/null; then
  echo "Pairing CLI failed to finish" >&2
  cat "$TMP/setup.log" >&2
  exit 1
fi
wait "$SETUP_PID" || { cat "$TMP/setup.log" >&2; exit 1; }
SETUP_PID=''
test -f "$TMP/state/identity.json"
test -f "$TMP/state/config.json"
cp "$TMP/state/identity.json" "$TMP/identity-before.json"
HOME="$TMP/home" target/debug/infra setup \
 --gateway "ws://127.0.0.1:$PORT/agent" \
 --agent "$ROOT/target/debug/infra-agent" \
 --state-dir "$TMP/state" --no-startup >"$TMP/setup2.log" 2>&1
cmp "$TMP/state/identity.json" "$TMP/identity-before.json"
HOME="$TMP/home" target/debug/infra-agent --state-dir "$TMP/state" >"$TMP/agent.log" 2>&1 &
AGENT_PID=$!

python3 - "$PORT" "$TOKEN" "$TMP/state/identity.json" <<'PY'
import json,sys,time,urllib.request
port,token,identity_path=sys.argv[1:]
identity=json.load(open(identity_path))
base=f"http://127.0.0.1:{port}"
h={'Authorization':f'Bearer {token}'}
deadline=time.monotonic()+20
while True:
  request=urllib.request.Request(base+'/api/devices',headers=h)
  with urllib.request.urlopen(request,timeout=4) as response: devices=json.load(response)
  matching=[d for d in devices if d['device_id']==identity['device_id'] and d['online']]
  if matching:
    assert len(devices)==1,devices
    assert matching[0]['connection_generation'] is not None
    print('End-to-end pairing PASS: same identity, live signed agent, one device, current generation.')
    break
  if time.monotonic()>deadline:raise AssertionError(f'agent never connected: {devices}')
  time.sleep(.25)
PY
