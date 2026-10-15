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

ENROLL_JSON="$(curl -fsS -X POST \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
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
  DEVICES="$(curl -fsS "http://127.0.0.1:$PORT/api/devices")"
  DEVICE_ID="$(python3 -c 'import json,sys; d=json.load(sys.stdin); print(next((x["device_id"] for x in d if x.get("online")), ""))' <<<"$DEVICES")"
  [ -z "$DEVICE_ID" ] || break
  sleep .05
done
[ -n "$DEVICE_ID" ] || { cat "$TMP/agent.log" >&2; exit 1; }

# Dashboard session/activity APIs are live and activity is persisted.
curl -fsS "http://127.0.0.1:$PORT/api/sessions" | python3 -c 'import json,sys; assert isinstance(json.load(sys.stdin), list)'
curl -fsS "http://127.0.0.1:$PORT/api/activity" | python3 -c 'import json,sys; a=json.load(sys.stdin); assert any(x["action"]=="device.connect" for x in a), a'

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

echo "Infra installer/dashboard smoke passed: dashboard, enrollment, native install, persisted identity/config, online agent, status and doctor."
