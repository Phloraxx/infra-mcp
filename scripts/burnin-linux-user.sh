#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-status}"
ROOT="${INFRA_BURNIN_ROOT:-$HOME/.local/share/infra-v1-burnin}"
PORT="${INFRA_BURNIN_PORT:-18789}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/bin"
GW_STATE="$ROOT/gateway"
AGENT_STATE="$ROOT/agent"
GW_ENV="$GW_STATE/gateway.env"
GW_UNIT="$HOME/.config/systemd/user/infra-v1-burnin-gateway.service"
GW_URL="http://127.0.0.1:$PORT"
AGENT_URL="ws://127.0.0.1:$PORT/agent"
DEVICE_NAME="${INFRA_BURNIN_DEVICE_NAME:-$(hostname)-infra-v1-burnin}"

need() { command -v "$1" >/dev/null || { echo "missing required command: $1" >&2; exit 1; }; }
need systemctl
need curl
need python3

# Remote/non-login sessions often omit the user-bus environment even when the
# systemd user manager is healthy. Reconstruct the standard runtime path so the
# burn-in exercises the same situation the CLI must recover from.
if [ -z "${XDG_RUNTIME_DIR:-}" ]; then
  export XDG_RUNTIME_DIR="/run/user/$(id -u)"
fi
if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ] && [ -S "$XDG_RUNTIME_DIR/bus" ]; then
  export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
fi

admin_token() {
  sed -n 's/^INFRA_ADMIN_TOKEN=//p' "$GW_ENV"
}

auth_config() {
  printf 'header = "Authorization: Bearer %s"\n' "$(admin_token)"
}

auth_curl() {
  curl --config <(auth_config) "$@"
}

wait_health() {
  for _ in $(seq 1 120); do
    curl -fsS "$GW_URL/healthz" >/dev/null 2>&1 && return 0
    sleep .1
  done
  echo "burn-in gateway did not become healthy" >&2
  return 1
}

online_device() {
  auth_curl -fsS "$GW_URL/api/devices" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(next((x["device_id"] for x in d if x.get("online")), ""))'
}

wait_device() {
  local id=''
  for _ in $(seq 1 150); do
    id="$(online_device || true)"
    [ -z "$id" ] || { printf '%s\n' "$id"; return 0; }
    sleep .1
  done
  echo "burn-in agent did not become online" >&2
  return 1
}

mcp_call() {
  local name="$1" args_json="$2"
  python3 - "$name" "$args_json" <<'PY' | auth_curl -fsS -X POST \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json, text/event-stream' \
    -H 'MCP-Protocol-Version: 2025-06-18' \
    --data-binary @- "$GW_URL/mcp"
import json,sys
print(json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":sys.argv[1],"arguments":json.loads(sys.argv[2])}}))
PY
}

install_burnin() {
  [ -x "$REPO/target/release/infra-gateway" ] || { echo "build release binaries first: cargo build --release -p infra-gateway -p infra-agent -p infra-cli" >&2; exit 1; }
  mkdir -p "$BIN" "$GW_STATE" "$AGENT_STATE" "$HOME/.config/systemd/user"
  install -m 0755 "$REPO/target/release/infra-gateway" "$BIN/infra-gateway"
  install -m 0755 "$REPO/target/release/infra-agent" "$BIN/infra-agent"
  install -m 0755 "$REPO/target/release/infra" "$BIN/infra"

  if [ ! -f "$GW_ENV" ]; then
    local token
    token="$(python3 -c 'import secrets; print(secrets.token_urlsafe(48))')"
    umask 077
    cat > "$GW_ENV" <<ENV
INFRA_ADMIN_TOKEN=$token
INFRA_GATEWAY_BIND=127.0.0.1:$PORT
INFRA_DB_PATH=$GW_STATE/infra.db
RUST_LOG=info
ENV
    chmod 600 "$GW_ENV"
  fi

  cat > "$GW_UNIT" <<UNIT
[Unit]
Description=Infra v1 local burn-in gateway
After=network.target

[Service]
Type=simple
EnvironmentFile=$GW_ENV
ExecStart=$BIN/infra-gateway
Restart=always
RestartSec=2

[Install]
WantedBy=default.target
UNIT
  systemctl --user daemon-reload
  systemctl --user enable --now infra-v1-burnin-gateway.service
  wait_health

  local code_args=()
  if [ ! -f "$AGENT_STATE/identity.json" ]; then
    local code
    code="$(printf '%s' '{"ttl_seconds":180}' | auth_curl -fsS -X POST -H 'Content-Type: application/json' --data-binary @- "$GW_URL/api/enrollments" | python3 -c 'import json,sys; print(json.load(sys.stdin)["code"])')"
    code_args=(--code "$code")
  fi

  "$BIN/infra" --state-dir "$AGENT_STATE" install \
    --gateway "$AGENT_URL" \
    --name "$DEVICE_NAME" \
    --agent "$BIN/infra-agent" \
    "${code_args[@]}"
  wait_device >/dev/null
  "$BIN/infra" --state-dir "$AGENT_STATE" doctor
  echo "Infra v1 burn-in installed beside v0.3 at $ROOT"
}

pty_write_expect() {
  local device="$1" session="$2" marker="$3" result=''
  mcp_call terminal "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"write","session_id":sys.argv[2],"data":sys.argv[3]+"\n"}))' "$device" "$session" "$marker")" >/dev/null
  for _ in $(seq 1 100); do
    result="$(mcp_call terminal "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"read","session_id":sys.argv[2],"offset":0,"length":16384}))' "$device" "$session")")"
    grep -q "$marker" <<<"$result" && return 0
    sleep .1
  done
  echo "PTY marker $marker never arrived" >&2
  echo "$result" >&2
  return 1
}

wait_no_active_work() {
  local device="$1" summary=''
  for _ in $(seq 1 150); do
    summary="$(auth_curl -fsS "$GW_URL/api/devices" | python3 -c 'import json,sys; d=json.load(sys.stdin); target=sys.argv[1]; x=next((v for v in d if v.get("device_id")==target), None); print("missing" if x is None else "{}:{}:{}".format(x.get("active_jobs",0),x.get("active_terminals",0),x.get("active_transfers",0)))' "$device")"
    [ "$summary" = "0:0:0" ] && return 0
    sleep .1
  done
  echo "active work did not reconcile to zero for $device (last=$summary)" >&2
  return 1
}

job_read_expect() {
  local device="$1" job="$2" marker="$3" result=''
  for _ in $(seq 1 120); do
    result="$(mcp_call jobs "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"read","job_id":sys.argv[2],"offset":0,"length":16384}))' "$device" "$job")")"
    grep -q "$marker" <<<"$result" && return 0
    sleep .1
  done
  echo "job marker $marker never arrived" >&2
  echo "$result" >&2
  return 1
}

run_test() {
  wait_health
  local device marker exec_result open_result session write_result read_result
  device="$(wait_device)"
  marker="infra-burnin-$(date +%s)"

  exec_result="$(mcp_call exec "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"command":"printf "+sys.argv[2]}))' "$device" "$marker")")"
  grep -q "$marker" <<<"$exec_result"

  open_result="$(mcp_call terminal "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"open","command":"cat"}))' "$device")")"
  session="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["structuredContent"]["body"]["session"]["id"])' <<<"$open_result")"
  pty_write_expect "$device" "$session" "$marker"

  echo "gateway restart test..."
  systemctl --user restart infra-v1-burnin-gateway.service
  wait_health
  device="$(wait_device)"
  local gateway_marker="${marker}-after-gateway"
  pty_write_expect "$device" "$session" "$gateway_marker" || {
    echo "PTY was not live after gateway restart" >&2
    return 1
  }

  echo "agent service restart test..."
  local job_before="${marker}-job-before" job_after="${marker}-job-after" job_result job
  local job_command="printf '${marker}-job-before\n'; sleep 3; printf '${marker}-job-after\n'; sleep 30"
  job_result="$(mcp_call jobs "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"start","command":sys.argv[2],"timeout_ms":60000}))' "$device" "$job_command")")"
  job="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["structuredContent"]["body"]["job"]["id"])' <<<"$job_result")"
  job_read_expect "$device" "$job" "$job_before"

  systemctl --user restart infra-agent.service
  device="$(wait_device)"
  local agent_marker="${marker}-after-agent"
  if ! pty_write_expect "$device" "$session" "$agent_marker"; then
    echo "PTY did not survive agent service restart" >&2
    return 2
  fi
  if ! job_read_expect "$device" "$job" "$job_after"; then
    echo "background job did not survive agent service restart" >&2
    return 3
  fi

  mcp_call jobs "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"cancel","job_id":sys.argv[2]}))' "$device" "$job")" >/dev/null || true
  mcp_call terminal "$(python3 -c 'import json,sys; print(json.dumps({"device":sys.argv[1],"action":"close","session_id":sys.argv[2]}))' "$device" "$session")" >/dev/null || true
  wait_no_active_work "$device"
  "$BIN/infra" --state-dir "$AGENT_STATE" doctor
  echo "Infra v1 real-service burn-in passed: exec, PTY, durable job, gateway restart and agent-service restart."
}

status_burnin() {
  systemctl --user --no-pager --full status infra-v1-burnin-gateway.service infra-agent.service || true
  [ ! -x "$BIN/infra" ] || "$BIN/infra" --state-dir "$AGENT_STATE" status
  [ ! -f "$GW_STATE/infra.db" ] || auth_curl -fsS "$GW_URL/api/devices" || true
  printf '\nUser linger: '
  loginctl show-user "$USER" -p Linger --value 2>/dev/null || echo unknown
}

remove_burnin() {
  [ ! -x "$BIN/infra" ] || "$BIN/infra" --state-dir "$AGENT_STATE" uninstall --remove-state || true
  systemctl --user disable --now infra-v1-burnin-gateway.service >/dev/null 2>&1 || true
  rm -f "$GW_UNIT"
  systemctl --user daemon-reload
  rm -rf "$ROOT"
  echo "Infra v1 burn-in removed; v0.3 was not touched."
}

case "$MODE" in
  install) install_burnin ;;
  test) run_test ;;
  status) status_burnin ;;
  remove) remove_burnin ;;
  *) echo "usage: $0 {install|test|status|remove}" >&2; exit 2 ;;
esac
