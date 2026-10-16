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
auth_config() {
  printf 'header = "Authorization: Bearer %s"\n' "$ADMIN_TOKEN"
}

auth_curl() {
  curl --config <(auth_config) "$@"
}
if INFRA_GATEWAY_BIND="0.0.0.0:$PORT" INFRA_DB_PATH="$TMP/reject.db" target/debug/infra-gateway >"$TMP/reject.log" 2>&1; then
  echo "gateway unexpectedly allowed non-loopback bind without admin credential" >&2
  exit 1
fi
grep -q 'refusing non-loopback bind without INFRA_ADMIN_TOKEN' "$TMP/reject.log"
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

# Protected control surfaces reject unauthenticated callers when an admin credential exists.
[ "$(curl -sS -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/api/devices")" = 401 ]
[ "$(curl -sS -o /dev/null -w '%{http_code}' -X POST -H 'Content-Type: application/json' --data '{"jsonrpc":"2.0","id":0,"method":"tools/list","params":{}}' "http://127.0.0.1:$PORT/mcp")" = 401 ]

ENROLL_JSON="$(printf '%s' '{"ttl_seconds":120}' | auth_curl -fsS -X POST \
  -H 'Content-Type: application/json' \
  --data-binary @- \
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
  DEVICES="$(auth_curl -fsS "http://127.0.0.1:$PORT/api/devices")"
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

mcp_request() {
  printf '%s' "$1" | auth_curl -fsS -X POST \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -H 'MCP-Protocol-Version: 2025-06-18' \
    --data-binary @- "http://127.0.0.1:$PORT/mcp"
}

TOOLS_CALL='{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
TOOLS_RESULT="$(mcp_request "$TOOLS_CALL")"
python3 -c 'import json,sys; r=json.load(sys.stdin); names={x["name"] for x in r["result"]["tools"]}; required={"devices","exec","files","search","processes","jobs","terminal","transfer","system","network","services","containers","git"}; assert required <= names, (required-names,names)' <<<"$TOOLS_RESULT"

CALL="$(python3 - "$DEVICE_ID" <<'PYA'
import json,sys
print(json.dumps({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"exec","arguments":{"device":sys.argv[1],"command":"printf infra-smoke-ok"}}}))
PYA
)"
RESULT="$(mcp_request "$CALL")"
python3 -c 'import json,sys; r=json.load(sys.stdin); text=json.dumps(r); assert "infra-smoke-ok" in text and "ok" in text, text' <<<"$RESULT"

SYSTEM_CALL="$(python3 - "$DEVICE_ID" <<'PYB'
import json,sys
print(json.dumps({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"system","arguments":{"device":sys.argv[1],"action":"summary"}}}))
PYB
)"
SYSTEM_RESULT="$(mcp_request "$SYSTEM_CALL")"
python3 -c 'import json,sys; b=json.load(sys.stdin)["result"]["structuredContent"]["body"]; assert b["cpu_count"] > 0 and b["total_memory"] > 0, b' <<<"$SYSTEM_RESULT"

FILE_PATH="$TMP/structured-file.txt"
FILES_WRITE="$(python3 - "$DEVICE_ID" "$FILE_PATH" <<'PYC'
import json,sys
print(json.dumps({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"files","arguments":{"device":sys.argv[1],"action":"write","path":sys.argv[2],"content":"alpha beta alpha"}}}))
PYC
)"
mcp_request "$FILES_WRITE" >/dev/null
FILES_EDIT="$(python3 - "$DEVICE_ID" "$FILE_PATH" <<'PYD'
import json,sys
print(json.dumps({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"files","arguments":{"device":sys.argv[1],"action":"exact_edit","path":sys.argv[2],"old":"alpha","new":"omega","expected_replacements":2}}}))
PYD
)"
mcp_request "$FILES_EDIT" >/dev/null
[ "$(cat "$FILE_PATH")" = 'omega beta omega' ]
FILES_HASH="$(python3 - "$DEVICE_ID" "$FILE_PATH" <<'PYE'
import json,sys
print(json.dumps({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"files","arguments":{"device":sys.argv[1],"action":"hash","path":sys.argv[2]}}}))
PYE
)"
HASH_RESULT="$(mcp_request "$FILES_HASH")"
python3 -c 'import json,sys; b=json.load(sys.stdin)["result"]["structuredContent"]["body"]; assert len(b["sha256"])==64, b' <<<"$HASH_RESULT"

SEARCH_CALL="$(python3 - "$DEVICE_ID" "$TMP" <<'PYF'
import json,sys
print(json.dumps({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"search","arguments":{"device":sys.argv[1],"root":sys.argv[2],"query":"omega beta omega","mode":"content","literal":True,"max_results":10}}}))
PYF
)"
SEARCH_RESULT="$(mcp_request "$SEARCH_CALL")"
python3 -c 'import json,sys; b=json.load(sys.stdin)["result"]["structuredContent"]["body"]; assert b["results"], b' <<<"$SEARCH_RESULT"

NETWORK_CALL="$(python3 - "$DEVICE_ID" "$PORT" <<'PYG'
import json,sys
print(json.dumps({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"network","arguments":{"device":sys.argv[1],"action":"tcp_check","host":"127.0.0.1","port":int(sys.argv[2]),"timeout_ms":2000}}}))
PYG
)"
NETWORK_RESULT="$(mcp_request "$NETWORK_CALL")"
python3 -c 'import json,sys; b=json.load(sys.stdin)["result"]["structuredContent"]["body"]; assert b["reachable"] is True, b' <<<"$NETWORK_RESULT"

GIT_CALL="$(python3 - "$DEVICE_ID" "$ROOT" <<'PYH'
import json,sys
print(json.dumps({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"git","arguments":{"device":sys.argv[1],"action":"status","repo":sys.argv[2]}}}))
PYH
)"
GIT_RESULT="$(mcp_request "$GIT_CALL")"
python3 -c 'import json,sys; b=json.load(sys.stdin)["result"]["structuredContent"]["body"]; assert "changes" in b and "branch" in b, b' <<<"$GIT_RESULT"

TRANSFER_PATH="$TMP/transfer-smoke.bin"
TRANSFER_DATA='Infra binary transfer smoke payload'
TRANSFER_META="$(python3 - "$TRANSFER_DATA" <<'PYI'
import base64,hashlib,json,sys
b=sys.argv[1].encode(); print(json.dumps({"size":len(b),"sha256":hashlib.sha256(b).hexdigest(),"base64":base64.b64encode(b).decode()}))
PYI
)"
UPLOAD_BEGIN="$(python3 - "$DEVICE_ID" "$TRANSFER_PATH" "$TRANSFER_META" <<'PYJ'
import json,sys
m=json.loads(sys.argv[3]); print(json.dumps({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"transfer","arguments":{"device":sys.argv[1],"action":"upload_begin","path":sys.argv[2],"size":m["size"],"sha256":m["sha256"]}}}))
PYJ
)"
UPLOAD_BEGIN_RESULT="$(mcp_request "$UPLOAD_BEGIN")"
TRANSFER_ID="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["structuredContent"]["body"]["transfer"]["id"])' <<<"$UPLOAD_BEGIN_RESULT")"
UPLOAD_CHUNK="$(python3 - "$DEVICE_ID" "$TRANSFER_ID" "$TRANSFER_META" <<'PYK'
import json,sys
m=json.loads(sys.argv[3]); print(json.dumps({"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"transfer","arguments":{"device":sys.argv[1],"action":"upload_chunk","transfer_id":sys.argv[2],"offset":0,"data_base64":m["base64"]}}}))
PYK
)"
mcp_request "$UPLOAD_CHUNK" >/dev/null
mcp_request "$UPLOAD_CHUNK" >/dev/null
UPLOAD_COMMIT="$(python3 - "$DEVICE_ID" "$TRANSFER_ID" <<'PYL'
import json,sys
print(json.dumps({"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"transfer","arguments":{"device":sys.argv[1],"action":"upload_commit","transfer_id":sys.argv[2]}}}))
PYL
)"
UPLOAD_COMMIT_RESULT="$(mcp_request "$UPLOAD_COMMIT")"
python3 -c 'import json,sys; b=json.load(sys.stdin)["result"]["structuredContent"]["body"]; assert b["transfer"]["status"]=="completed" and len(b["sha256"])==64, b' <<<"$UPLOAD_COMMIT_RESULT"
[ "$(cat "$TRANSFER_PATH")" = "$TRANSFER_DATA" ]

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
  DEVICES="$(auth_curl -fsS "http://127.0.0.1:$PORT/api/devices")"
  ONLINE="$(python3 -c 'import json,sys; d=json.load(sys.stdin); print(any(x.get("device_id")==sys.argv[1] and x.get("online") for x in d))' "$DEVICE_ID" <<<"$DEVICES")"
  [ "$ONLINE" != True ] || break
  sleep .1
done
[ "$ONLINE" = True ] || { echo 'Signed identity reconnect failed.' >&2; exit 1; }

echo "Infra v1 smoke passed: enrollment/auth/reconnect plus exec, system, files, search, network, git and resumable transfer through MCP."
