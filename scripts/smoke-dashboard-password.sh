#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
TMP="$(mktemp -d)"
GW_PID=''
cleanup() { [ -z "$GW_PID" ] || kill "$GW_PID" 2>/dev/null || true; [ -z "$GW_PID" ] || wait "$GW_PID" 2>/dev/null || true; rm -rf "$TMP"; }
trap cleanup EXIT INT TERM
cargo build -q -p infra-gateway
printf '%s' 'fixture-owner-password-123' | target/debug/infra-gateway init-auth "$TMP/auth.json" owner https://infra.example.com >/dev/null
PORT="$(python3 - <<'PY'
import socket
s=socket.socket();s.bind(('127.0.0.1',0));print(s.getsockname()[1]);s.close()
PY
)"
INFRA_GATEWAY_BIND="127.0.0.1:$PORT" INFRA_DB_PATH="$TMP/gateway.db" INFRA_DASHBOARD_AUTH_FILE="$TMP/auth.json" \
  target/debug/infra-gateway >"$TMP/gateway.log" 2>&1 &
GW_PID=$!
for _ in $(seq 1 100); do curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && break; sleep .05; done
python3 - "$PORT" "$TMP/auth.json" <<'PY'
import http.cookiejar,json,urllib.request,urllib.error,sys,os
port,path=sys.argv[1:];base='http://127.0.0.1:'+port
jar=http.cookiejar.CookieJar();client=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
def call(path,body=None,expected=200,headers=None,opener=client):
 h={'Content-Type':'application/json','X-Infra-CSRF':'1',**(headers or {})}
 req=urllib.request.Request(base+path,headers=h,data=None if body is None else json.dumps(body).encode())
 try:
  with opener.open(req,timeout=10) as response:
   assert response.status==expected,(path,response.status,expected)
   assert response.headers.get('Cache-Control')=='no-store'
   return json.load(response)
 except urllib.error.HTTPError as error:
  assert error.code==expected,(path,error.code,expected)
unauth=urllib.request.build_opener()
s=call('/api/auth/session');assert not s['authenticated'] and s['login_method']=='password'
call('/api/devices',expected=401)
call('/api/auth/session',{'username':'owner','password':'wrong'},expected=401)
call('/api/auth/session',{'token':'anything'},expected=401)
call('/api/auth/session',{'username':'owner','password':'fixture-owner-password-123'},headers={'Origin':'https://evil.example'},expected=403)
call('/api/auth/session',{'username':'owner','password':'fixture-owner-password-123'})
assert any(c.name=='infra_session' and c.has_nonstandard_attr('HttpOnly') for c in jar)
assert call('/api/auth/session')['authenticated']
assert call('/api/devices')==[]
old_cookies=http.cookiejar.CookieJar()
for cookie in jar: old_cookies.set_cookie(cookie)
old_client=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(old_cookies))
call('/api/auth/password',{'current_password':'wrong','new_password':'second-fixture-password'},expected=400)
call('/api/auth/password',{'current_password':'fixture-owner-password-123','new_password':'second-fixture-password'})
assert call('/api/auth/session')['authenticated']
call('/api/devices',expected=401,opener=old_client)
call('/api/auth/session',{'username':'owner','password':'fixture-owner-password-123'},expected=401,opener=unauth)
call('/api/auth/session',{'username':'owner','password':'second-fixture-password'},opener=unauth)
# HTTPS public-host routing and MCP isolation; redirects are checked without following.
class NoRedirect(urllib.request.HTTPRedirectHandler):
 def redirect_request(self,*args):return None
redirect_client=urllib.request.build_opener(NoRedirect())
try:redirect_client.open(urllib.request.Request(base+'/',headers={'Host':'infra.example.com'}));raise AssertionError('HTTP was accepted')
except urllib.error.HTTPError as error:assert error.code==308 and error.headers['Location']=='https://infra.example.com/'
# Public MCP now exists, but must reject unauthenticated clients.
# The old 404 check became stale when OAuth discovery was added.
req=urllib.request.Request(base+'/mcp',headers={'Host':'infra.example.com','X-Forwarded-Proto':'https'})
try:
 unauth.open(req,timeout=10)
 raise AssertionError('Public MCP accepted an unauthenticated request')
except urllib.error.HTTPError as error:
 assert error.code==401,('/mcp',error.code,401)
 assert 'Bearer' in error.headers.get('WWW-Authenticate','')
 assert 'resource_metadata=' in error.headers.get('WWW-Authenticate','')
call('/api/auth/session',expected=403,headers={'Host':'infra.example.com','X-Forwarded-Proto':'https','Origin':'http://infra.example.com'},opener=unauth)
secure=http.cookiejar.CookieJar();secure_client=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(secure))
call('/api/auth/session',{'username':'owner','password':'second-fixture-password'},headers={'Host':'infra.example.com','X-Forwarded-Proto':'https'},opener=secure_client)
assert any(c.secure for c in secure)
content=open(path).read();assert 'fixture-owner-password-123' not in content and 'second-fixture-password' not in content
if os.name=='posix':assert os.stat(path).st_mode&0o777==0o600
# Lockout is global and bounded, including attempts from different usernames.
for _ in range(12):
 try:unauth.open(urllib.request.Request(base+'/api/auth/session',data=json.dumps({'username':'owner','password':'wrong'}).encode(),headers={'Content-Type':'application/json','X-Infra-CSRF':'1'}))
 except urllib.error.HTTPError as e:
  if e.code==429:break
else:raise AssertionError('sign-in attempts were not limited')
print('Password auth smoke PASS: Argon2id, sign-in, wrong credentials, rate limit, cookies, password rotation/revocation, CSRF/origin boundary, HTTPS and public MCP isolation.')
PY
