#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
command -v python3 >/dev/null
command -v curl >/dev/null
TMP="$(mktemp -d)"
GW_PID=''
AGENT_PID=''
cleanup() {
  [ -z "$AGENT_PID" ] || kill "$AGENT_PID" 2>/dev/null || true
  [ -z "$GW_PID" ] || kill "$GW_PID" 2>/dev/null || true
  [ -z "$AGENT_PID" ] || wait "$AGENT_PID" 2>/dev/null || true
  [ -z "$GW_PID" ] || wait "$GW_PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

cargo build -q -p infra-gateway -p infra-agent -p infra-cli
PORT="$(python3 - <<'PY'
import socket
s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()
PY
)"
ADMIN_TOKEN="$(python3 - <<'PY'
import secrets; print(secrets.token_urlsafe(32))
PY
)"
INFRA_GATEWAY_BIND="127.0.0.1:$PORT" \
INFRA_DB_PATH="$TMP/infra.db" \
INFRA_ADMIN_TOKEN="$ADMIN_TOKEN" \
RUST_LOG=warn \
  target/debug/infra-gateway >"$TMP/gateway.log" 2>&1 &
GW_PID=$!

for _ in $(seq 1 100); do
  curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && break
  sleep .05
done
curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null

# Dashboard and bootstrap scripts are served by the same gateway binary.
curl -fsS "http://127.0.0.1:$PORT/" | grep -q '<title>Infra</title>'
curl -fsS "http://127.0.0.1:$PORT/app.js" | grep -q 'create-enrollment'
curl -fsS "http://127.0.0.1:$PORT/downloads/install.sh" | grep -q 'Infra installer'
! grep -q 'sessionStorage' apps/dashboard/app.js

COOKIE_JAR="$TMP/browser.cookies"
[ "$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/api/devices")" = 401 ]
python3 - "$ADMIN_TOKEN" <<'PY' | curl -fsS -c "$COOKIE_JAR" -H 'Content-Type: application/json' --data-binary @- \
  "http://127.0.0.1:$PORT/api/auth/session" | \
  python3 -c 'import json,sys; r=json.load(sys.stdin); assert r["authenticated"] is True and r["mode"]=="browser_session", r'
import json,sys
print(json.dumps({"token":sys.argv[1]}))
PY
grep -q 'infra_session' "$COOKIE_JAR"
curl -fsS -b "$COOKIE_JAR" "http://127.0.0.1:$PORT/api/auth/session" | \
  python3 -c 'import json,sys; r=json.load(sys.stdin); assert r["authenticated"] is True, r'

# Signed browser sessions survive a gateway restart as long as the admin credential is unchanged.
kill "$GW_PID"
wait "$GW_PID" 2>/dev/null || true
INFRA_GATEWAY_BIND="127.0.0.1:$PORT" \
INFRA_DB_PATH="$TMP/infra.db" \
INFRA_ADMIN_TOKEN="$ADMIN_TOKEN" \
RUST_LOG=warn \
  target/debug/infra-gateway >"$TMP/gateway.log" 2>&1 &
GW_PID=$!
for _ in $(seq 1 100); do
  curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1 && break
  sleep .05
done
curl -fsS -b "$COOKIE_JAR" "http://127.0.0.1:$PORT/api/auth/session" | \
  python3 -c 'import json,sys; r=json.load(sys.stdin); assert r["authenticated"] is True and r["mode"]=="browser_session", r'

[ "$(curl -sS -b "$COOKIE_JAR" -o /dev/null -w '%{http_code}' -X POST -H 'Content-Type: application/json' --data '{"ttl_seconds":120}' "http://127.0.0.1:$PORT/api/enrollments")" = 403 ]
ENROLL_JSON="$(curl -fsS -b "$COOKIE_JAR" -X POST \
  -H 'Content-Type: application/json' \
  -H 'X-Infra-CSRF: 1' \
  --data '{"ttl_seconds":120}' \
  "http://127.0.0.1:$PORT/api/enrollments")"
CODE="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["code"])' <<<"$ENROLL_JSON")"

HOME="$TMP/home" ./scripts/install.sh \
  --gateway "ws://127.0.0.1:$PORT/agent" \
  --code "$CODE" \
  --name "Installer Smoke Device" \
  --install-dir "$TMP/bin" \
  --state-dir "$TMP/state" \
  --source-dir target/debug \
  --no-startup >"$TMP/install.log"

test -x "$TMP/bin/infra"
test -x "$TMP/bin/infra-agent"
test -f "$TMP/state/config.json"
test -f "$TMP/state/identity.json"
STATUS="$(HOME="$TMP/home" $TMP/bin/infra status --json --state-dir "$TMP/state")"
python3 - <<'PY' "$STATUS"
import json,sys
s=json.loads(sys.argv[1])
assert s["configured"] is True, s
assert s["enrolled"] is True, s
PY

# Start the same installed agent with no enrollment code; persisted config + identity must suffice.
RUST_LOG=warn "$TMP/bin/infra-agent" --state-dir "$TMP/state" >"$TMP/agent.log" 2>&1 &
AGENT_PID=$!
DEVICE_ID=''
for _ in $(seq 1 100); do
  DEVICES="$(curl -fsS -b "$COOKIE_JAR" "http://127.0.0.1:$PORT/api/devices")"
  DEVICE_ID="$(python3 -c 'import json,sys; d=json.load(sys.stdin); print(next((x["device_id"] for x in d if x.get("online")), ""))' <<<"$DEVICES")"
  [ -z "$DEVICE_ID" ] || break
  sleep .05
done
[ -n "$DEVICE_ID" ] || { cat "$TMP/agent.log" >&2; exit 1; }

# Dashboard session/activity APIs are live and activity is persisted.
curl -fsS -b "$COOKIE_JAR" "http://127.0.0.1:$PORT/api/sessions" | python3 -c 'import json,sys; assert isinstance(json.load(sys.stdin), list)'
curl -fsS -b "$COOKIE_JAR" "http://127.0.0.1:$PORT/api/activity" | python3 -c 'import json,sys; a=json.load(sys.stdin); assert any(x["action"]=="device.connect" for x in a), a'

# Browser controls share the actual agent PTY workers used by MCP.
OP_URL="http://127.0.0.1:$PORT/api/devices/$DEVICE_ID/operations"
dashboard_call() {
  curl -fsS -b "$COOKIE_JAR" -H 'Content-Type: application/json' \
    -H 'X-Infra-CSRF: 1' --data-binary "$1" "$OP_URL"
}
[ "$(curl -sS -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' --data '{"op":"status"}' "$OP_URL")" = 401 ]
[ "$(curl -sS -b "$COOKIE_JAR" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' --data '{"op":"status"}' "$OP_URL")" = 403 ]
[ "$(curl -sS -b "$COOKIE_JAR" -o /dev/null -w '%{http_code}' -H 'Content-Type: application/json' -H 'X-Infra-CSRF: 1' --data '{"op":"exec","command":"echo must-not-run"}' "$OP_URL")" = 400 ]
dashboard_call '{"op":"status"}' | python3 -c 'import json,sys; r=json.load(sys.stdin); assert r["status"]=="ok" and r["body"]["pid"]>0, r'
curl -fsS "http://127.0.0.1:$PORT/vendor/xterm.js" -o "$TMP/xterm.js"
grep -q 'Terminal' "$TMP/xterm.js"
OPEN="$(dashboard_call '{"op":"terminal","action":{"action":"open","command":"cat","rows":24,"cols":80}}')"
SESSION="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["body"]["session"]["id"])' <<<"$OPEN")"
dashboard_call "{\"op\":\"terminal\",\"action\":{\"action\":\"write\",\"session_id\":\"$SESSION\",\"data\":\"infra-dashboard-pty-marker\\n\"}}" >/dev/null
SEEN=0
for _ in $(seq 1 100); do
  READ="$(dashboard_call "{\"op\":\"terminal\",\"action\":{\"action\":\"read\",\"session_id\":\"$SESSION\",\"offset\":0,\"length\":32768}}")"
  if grep -q 'infra-dashboard-pty-marker' <<<"$READ"; then SEEN=1; break; fi
  sleep .05
done
[ "$SEEN" = 1 ]
# MCP sees the same session ID and transcript; no second dashboard terminal exists.
MCP_REQUEST="$(python3 -c 'import json,sys; print(json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"terminal","arguments":{"device":sys.argv[1],"action":"read","session_id":sys.argv[2],"offset":0}}}))' "$DEVICE_ID" "$SESSION")"
printf 'header = "Authorization: Bearer %s"\n' "$ADMIN_TOKEN" | curl --config - -fsS \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-06-18' --data-binary "$MCP_REQUEST" "http://127.0.0.1:$PORT/mcp" | \
  python3 -c 'import json,sys; r=json.load(sys.stdin)["result"]["structuredContent"]; assert "infra-dashboard-pty-marker" in r["body"]["output"], r'
dashboard_call "{\"op\":\"terminal\",\"action\":{\"action\":\"resize\",\"session_id\":\"$SESSION\",\"rows\":30,\"cols\":100}}" >/dev/null
dashboard_call "{\"op\":\"terminal\",\"action\":{\"action\":\"close\",\"session_id\":\"$SESSION\"}}" >/dev/null
# Text preview is bounded and metadata auditing does not persist its content.
printf 'dashboard-private-file-content' > "$TMP/preview.txt"
FILE_REQUEST="$(python3 -c 'import json,sys; print(json.dumps({"op":"files","action":{"action":"read","path":sys.argv[1],"length":9}}))' "$TMP/preview.txt")"
dashboard_call "$FILE_REQUEST" | python3 -c 'import json,sys; r=json.load(sys.stdin)["body"]; assert r["content"]=="dashboard" and r["has_more"] is True, r'
! curl -fsS -b "$COOKIE_JAR" "http://127.0.0.1:$PORT/api/activity" | grep -q 'dashboard-private-file-content'

DOCTOR="$(HOME="$TMP/home" $TMP/bin/infra doctor --json --state-dir "$TMP/state" || true)"
python3 - <<'PY' "$DOCTOR"
import json,sys
r=json.loads(sys.argv[1])
checks={x["name"]:x for x in r["checks"]}
assert checks["config"]["status"] == "ok", r
assert checks["identity"]["status"] == "ok", r
assert checks["agent_process"]["status"] == "ok", r
assert checks["gateway"]["status"] == "ok", r
# Startup is intentionally skipped in this isolated smoke environment.
assert checks["startup"]["status"] == "problem", r
PY

curl -fsS -b "$COOKIE_JAR" -c "$COOKIE_JAR" -X DELETE -H 'X-Infra-CSRF: 1' "http://127.0.0.1:$PORT/api/auth/session" >/dev/null
[ "$(curl -sS -b "$COOKIE_JAR" -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/api/devices")" = 401 ]

echo "Infra installer/dashboard smoke passed: dashboard HttpOnly session auth, enrollment, native install, persisted identity/config, online agent, status/doctor, shared browser/MCP PTY controls and bounded file preview."
