#!/usr/bin/env bash
set -euo pipefail

GW_URL="${INFRA_BURNIN_GATEWAY_HTTP:?set INFRA_BURNIN_GATEWAY_HTTP, e.g. http://100.x.y.z:18790}"
DEVICE="${INFRA_BURNIN_DEVICE_ID:?set INFRA_BURNIN_DEVICE_ID}"
REMOTE_RECONNECT_COMMAND="${INFRA_BURNIN_REMOTE_RECONNECT_COMMAND:?set INFRA_BURNIN_REMOTE_RECONNECT_COMMAND}"
MCP_HOST_HEADER="${INFRA_BURNIN_MCP_HOST_HEADER:-localhost}"
GATEWAY_RESTART_COMMAND="${INFRA_BURNIN_GATEWAY_RESTART_COMMAND:-}"

need() { command -v "$1" >/dev/null || { echo "missing required command: $1" >&2; exit 1; }; }
need curl
need python3

mcp_call() {
  local name="$1" args_json="$2" max_time="${3:-12}"
  python3 - "$name" "$args_json" <<'PY' | curl --max-time "$max_time" -fsS -X POST \
    -H "Host: $MCP_HOST_HEADER" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -H 'MCP-Protocol-Version: 2025-06-18' \
    --data-binary @- "$GW_URL/mcp"
import json,sys
print(json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":sys.argv[1],"arguments":json.loads(sys.argv[2])}}))
PY
}

device_field() {
  local field="$1"
  curl -fsS "$GW_URL/api/devices" | python3 -c 'import json,sys; device,field=sys.argv[1:3]; rows=json.load(sys.stdin); row=next((x for x in rows if x.get("device_id")==device), None); print("" if row is None else ("1" if field == "online" and row.get("online") else "0" if field == "online" else "" if row.get(field) is None else row.get(field)))' "$DEVICE" "$field"
}


wait_online() {
  local online=''
  for _ in $(seq 1 200); do
    online="$(device_field online || true)"
    [ "$online" = 1 ] && return 0
    sleep .1
  done
  echo "device did not become online" >&2
  return 1
}

wait_online_generation() {
  local previous_generation="$1" online='' generation=''
  for _ in $(seq 1 200); do
    online="$(device_field online || true)"
    generation="$(device_field connection_generation || true)"
    if [ "$online" = 1 ] && [ -n "$generation" ] && [ "$generation" -gt "$previous_generation" ] 2>/dev/null; then
      printf '%s\n' "$generation"
      return 0
    fi
    sleep .1
  done
  echo "device did not reconnect with a newer connection generation" >&2
  return 1
}

pty_write_expect() {
  local session="$1" marker="$2" result=''
  mcp_call terminal "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"write","session_id":sys.argv[2],"data":sys.argv[3]+"\n"}))' "$DEVICE" "$session" "$marker")" >/dev/null
  for _ in $(seq 1 120); do
    result="$(mcp_call terminal "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"read","session_id":sys.argv[2],"offset":0,"length":32768}))' "$DEVICE" "$session")")"
    grep -q "$marker" <<<"$result" && return 0
    sleep .1
  done
  echo "PTY marker never arrived: $marker" >&2
  echo "$result" >&2
  return 1
}

job_read_expect() {
  local job="$1" marker="$2" result=''
  for _ in $(seq 1 160); do
    result="$(mcp_call jobs "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"read","job_id":sys.argv[2],"offset":0,"length":32768}))' "$DEVICE" "$job")")"
    grep -q "$marker" <<<"$result" && return 0
    sleep .1
  done
  echo "job marker never arrived: $marker" >&2
  echo "$result" >&2
  return 1
}

online="$(device_field online)"
[ "$online" = 1 ] || { echo "target device is not online" >&2; exit 1; }
generation="$(device_field connection_generation)"
[ -n "$generation" ] || { echo "target device has no connection generation" >&2; exit 1; }

marker="infra-remote-burnin-$(date +%s)"
open="$(mcp_call terminal "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"open","command":"cat"}))' "$DEVICE")")"
session="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["structuredContent"]["body"]["session"]["id"])' <<<"$open")"
pty_write_expect "$session" "$marker-before-controller"

job_before="$marker-job-before"
job_after="$marker-job-after"
job_command="printf '$job_before\\n'; sleep 3; printf '$job_after\\n'; sleep 30"
job_result="$(mcp_call jobs "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"start","command":sys.argv[2],"timeout_ms":60000}))' "$DEVICE" "$job_command")")"
job="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["structuredContent"]["body"]["job"]["id"])' <<<"$job_result")"
job_read_expect "$job" "$job_before"

echo "controller restart test..."
restart_args="$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"command":sys.argv[2],"timeout_ms":10000}))' "$DEVICE" "$REMOTE_RECONNECT_COMMAND")"
set +e
mcp_call exec "$restart_args" 5 >/dev/null 2>&1
set -e
generation="$(wait_online_generation "$generation")"
pty_write_expect "$session" "$marker-after-controller"
job_read_expect "$job" "$job_after"

if [ -n "$GATEWAY_RESTART_COMMAND" ]; then
  echo "gateway restart test..."
  bash -lc "$GATEWAY_RESTART_COMMAND"
  for _ in $(seq 1 120); do
    curl -fsS "$GW_URL/healthz" >/dev/null 2>&1 && break
    sleep .1
  done
  wait_online
  generation="$(device_field connection_generation)"
  pty_write_expect "$session" "$marker-after-gateway"
fi

mcp_call jobs "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"cancel","job_id":sys.argv[2]}))' "$DEVICE" "$job")" >/dev/null 2>&1 || true
mcp_call terminal "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"close","session_id":sys.argv[2]}))' "$DEVICE" "$session")" >/dev/null 2>&1 || true

echo "Infra remote POSIX burn-in passed: same PTY and durable job survived controller restart${GATEWAY_RESTART_COMMAND:+ and the same PTY survived gateway restart}."
