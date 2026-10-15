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
  wait "$AGENT_PID" 2>/dev/null || true
  wait "$GW_PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

cargo build -q -p infra-gateway -p infra-agent
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

for _ in $(seq 1 80); do
  if curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1; then break; fi
  sleep .1
done
curl -fsS "http://127.0.0.1:$PORT/healthz" >/dev/null

ENROLL_JSON="$(curl -fsS -X POST \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  --data '{"ttl_seconds":120}' \
  "http://127.0.0.1:$PORT/api/enrollments")"
CODE="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["code"])' <<<"$ENROLL_JSON")"

INFRA_GATEWAY_URL="ws://127.0.0.1:$PORT/agent" \
INFRA_ENROLLMENT_CODE="$CODE" \
INFRA_DEVICE_NAME="Infra Smoke Device" \
INFRA_STATE_DIR="$TMP/agent" \
RUST_LOG=warn \
  target/debug/infra-agent >"$TMP/agent.log" 2>&1 &
AGENT_PID=$!

DEVICE_ID=''
for _ in $(seq 1 100); do
  DEVICES="$(curl -fsS "http://127.0.0.1:$PORT/api/devices")"
  DEVICE_ID="$(python3 -c 'import json,sys; d=json.load(sys.stdin); print(next((x["device_id"] for x in d if x.get("online")), ""))' <<<"$DEVICES")"
  [ -z "$DEVICE_ID" ] || break
  sleep .1
done
if [ -z "$DEVICE_ID" ]; then
  echo 'Agent did not become online.' >&2
  cat "$TMP/gateway.log" >&2 || true
  cat "$TMP/agent.log" >&2 || true
  exit 1
fi

CALL="$(python3 - "$DEVICE_ID" <<'PY'
import json,sys
print(json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"exec","arguments":{"device":sys.argv[1],"command":"printf infra-smoke-ok"}}}))
PY
)"
RESULT="$(curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-06-18' \
  --data "$CALL" "http://127.0.0.1:$PORT/mcp")"
python3 -c 'import json,sys; r=json.load(sys.stdin); text=json.dumps(r); assert "infra-smoke-ok" in text and "ok" in text, text' <<<"$RESULT"

# Kill only the controller. Its persisted identity must be reusable without the enrollment code.
kill "$AGENT_PID" 2>/dev/null || true
wait "$AGENT_PID" 2>/dev/null || true
AGENT_PID=''
INFRA_GATEWAY_URL="ws://127.0.0.1:$PORT/agent" \
INFRA_DEVICE_NAME="Infra Smoke Device" \
INFRA_STATE_DIR="$TMP/agent" \
RUST_LOG=warn \
  target/debug/infra-agent >"$TMP/agent-reconnect.log" 2>&1 &
AGENT_PID=$!
for _ in $(seq 1 100); do
  DEVICES="$(curl -fsS "http://127.0.0.1:$PORT/api/devices")"
  ONLINE="$(python3 -c 'import json,sys; d=json.load(sys.stdin); print(any(x.get("device_id")==sys.argv[1] and x.get("online") for x in d))' "$DEVICE_ID" <<<"$DEVICES")"
  [ "$ONLINE" != True ] || break
  sleep .1
done
[ "$ONLINE" = True ] || { echo 'Signed identity reconnect failed.' >&2; exit 1; }

echo "Infra v1 smoke test passed: enrollment, signed auth, MCP exec, identity reconnect."
