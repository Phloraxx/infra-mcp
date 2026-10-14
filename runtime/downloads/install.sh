#!/usr/bin/env bash
set -euo pipefail
umask 077
BASE_URL="${INFRA_BASE_URL:-https://mcp.phloraxx.us.to}"
PAIR_CODE="${INFRA_PAIR_CODE:-}"
DEVICE_NAME=""
SYSTEM=0
MANUAL=0
REMOVE=0
NODE_VERSION=v22.23.3
while [ "$#" -gt 0 ]; do
  case "$1" in
    --base) BASE_URL="$2"; shift 2 ;;
    --code) PAIR_CODE="$2"; shift 2 ;;
    --name) DEVICE_NAME="$2"; shift 2 ;;
    --system) SYSTEM=1; shift ;;
    --manual) MANUAL=1; shift ;;
    --uninstall) REMOVE=1; shift ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done
OS="$(uname -s)"
case "$OS" in Darwin|Linux) ;; *) echo 'This installer supports macOS and Linux.' >&2; exit 1 ;; esac
if [ "$SYSTEM" = 1 ] && [ "$(id -u)" != 0 ]; then echo 'System installation needs root. Run this downloaded installer with sudo --system.' >&2; exit 1; fi
if [ "$SYSTEM" = 0 ] && [ "$(id -u)" = 0 ]; then echo 'For a root installation, explicitly use --system.' >&2; exit 1; fi
BASE="${INFRA_INSTALL_DIR:-$HOME/.infra-mcp}"
[ "$SYSTEM" = 0 ] || BASE="${INFRA_INSTALL_DIR:-/var/lib/infra-mcp}"
LABEL="${INFRA_SERVICE_NAME:-to.phloraxx.infra.agent}"
SERVICE="${INFRA_SERVICE_NAME:-infra-mcp-agent}"
case "$SERVICE$LABEL" in *[!a-zA-Z0-9._-]*) echo 'Invalid service name.' >&2; exit 1 ;; esac
if [ "$OS" = Darwin ]; then
  PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
  [ "$SYSTEM" = 0 ] || PLIST="/Library/LaunchDaemons/$LABEL.plist"
else
  UNIT="$HOME/.config/systemd/user/$SERVICE.service"
  [ "$SYSTEM" = 0 ] || UNIT=/etc/systemd/system/$SERVICE.service
fi
stop_service() {
  if [ "$OS" = Darwin ]; then
    if [ "$SYSTEM" = 1 ]; then launchctl bootout system "$PLIST" >/dev/null 2>&1 || true
    else launchctl bootout "gui/$(id -u)" "$PLIST" >/dev/null 2>&1 || true; fi
  elif command -v systemctl >/dev/null 2>&1; then
    if [ "$SYSTEM" = 1 ]; then systemctl stop "$SERVICE.service" >/dev/null 2>&1 || true
    else systemctl --user stop "$SERVICE.service" >/dev/null 2>&1 || true; fi
  fi
}
if [ "$REMOVE" = 1 ]; then
  stop_service
  if [ "$OS" = Darwin ]; then rm -f "$PLIST"
  elif [ "$SYSTEM" = 1 ]; then systemctl disable "$SERVICE.service" >/dev/null 2>&1 || true; rm -f "$UNIT"; systemctl daemon-reload
  else systemctl --user disable "$SERVICE.service" >/dev/null 2>&1 || true; rm -f "$UNIT"; systemctl --user daemon-reload >/dev/null 2>&1 || true; fi
  echo "Startup removed. Configuration and job logs remain in $BASE. Remove the device from the dashboard to revoke its credential."
  exit 0
fi
case "$BASE_URL" in https://*) ;; http://127.0.0.1:*|http://localhost:*) ;; *) echo 'Use HTTPS for the gateway URL.' >&2; exit 1 ;; esac
command -v curl >/dev/null || { echo 'curl is required.' >&2; exit 1; }
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
CURL_NETWORK_FLAGS=()
if [ "${INFRA_CURL_COMPAT:-0}" = 1 ]; then CURL_NETWORK_FLAGS=(--ipv4 --http1.1); fi
download() {
  local attempt http rc name
  name="${1##*/}"
  echo "Downloading ${name}..."
  for attempt in 1 2 3; do
    rc=0
    http="$(curl -fsSL ${CURL_NETWORK_FLAGS[@]+"${CURL_NETWORK_FLAGS[@]}"} --connect-timeout 15 --max-time 180 "$1" -o "$2" -w '%{http_code}')" || rc=$?
    if [ "$rc" = 0 ]; then return 0; fi
    case "$rc" in
      5|6|7|16|18|28|35|52|55|56|92)
        # Keep certificate checks enabled; only change transport negotiation.
        if [ "$attempt" = 1 ] && [ "${#CURL_NETWORK_FLAGS[@]}" = 0 ]; then CURL_NETWORK_FLAGS=(--http1.1)
        else CURL_NETWORK_FLAGS=(--ipv4 --http1.1); fi ;;
      22) case "$http" in 408|429|500|502|503|504) ;; *) echo "Download failed: $name (HTTP $http)." >&2; return 1 ;; esac ;;
      *) echo "Download failed: $name (curl error $rc)." >&2; return 1 ;;
    esac
    if [ "$attempt" != 3 ]; then echo "Retrying ${name} ($((attempt+1))/3)..." >&2; sleep "$attempt"; fi
  done
  echo "Could not download $name after 3 attempts. Check the Mac's network, VPN or proxy, then retry. Pairing has not started." >&2
  return 1
}
# Pairing has a request identifier, so transport failures can be retried safely.
post_json() {
  local attempt http
  for attempt in 1 2 3; do
    if http="$(curl -sS ${CURL_NETWORK_FLAGS[@]+"${CURL_NETWORK_FLAGS[@]}"} --connect-timeout 10 --max-time 20 -H 'Content-Type: application/json' --data-binary "@$2" -o "$3" -w '%{http_code}' "$BASE_URL/$1")"; then
      case "$http" in
        2??) return 0 ;;
        408|500|502|503|504) ;;
        *) return 2 ;;
      esac
    fi
    if [ "$attempt" != 3 ]; then echo 'Connection interrupted. Retrying…' >&2; sleep "$attempt"; fi
  done
  return 1
}
digest() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi; }
echo 'Infra MCP · device agent'
mkdir -p "$BASE"
NODE="$(command -v node || true)"
if [ -x "$BASE/node/bin/node" ]; then NODE="$BASE/node/bin/node"; fi
# macOS uses Node 22, the runtime exercised by the integration suite.
if [ -z "$NODE" ] || ! "$NODE" -e 'const major=Number(process.versions.node.split(".")[0]);process.exit(major>=22&&(process.platform!=="darwin"||major===22)?0:1)' >/dev/null 2>&1; then
  ARCH="$(uname -m)"
  case "$ARCH" in x86_64) ARCH=x64 ;; arm64|aarch64) ARCH=arm64 ;; *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;; esac
  PLATFORM=linux; [ "$OS" != Darwin ] || PLATFORM=darwin
  if [ "$OS" = Linux ] && ldd --version 2>&1 | head -n 1 | grep -qi musl; then PLATFORM=linux; ARCH="$ARCH-musl"; fi
  ARCHIVE="node-$NODE_VERSION-$PLATFORM-$ARCH.tar.gz"
  echo 'Installing a private Node 22 runtime. Administrator access is not required.'
  download "https://nodejs.org/dist/$NODE_VERSION/$ARCHIVE" "$WORK/node.tar.gz"
  download "https://nodejs.org/dist/$NODE_VERSION/SHASUMS256.txt" "$WORK/SHASUMS256.txt"
  EXPECTED="$(awk -v file="$ARCHIVE" '$2==file {print $1}' "$WORK/SHASUMS256.txt")"
  [ -n "$EXPECTED" ] && [ "$(digest "$WORK/node.tar.gz")" = "$EXPECTED" ] || { echo 'Node runtime checksum failed.' >&2; exit 1; }
  mkdir -p "$BASE/node"
  tar -xzf "$WORK/node.tar.gz" -C "$BASE/node" --strip-components=1
  NODE="$BASE/node/bin/node"
fi
download "$BASE_URL/downloads/manifest.json" "$WORK/manifest.json"
download "$BASE_URL/downloads/agent.mjs" "$WORK/agent.mjs"
download "$BASE_URL/downloads/jobs.mjs" "$WORK/jobs.mjs"
"$NODE" --input-type=module - "$WORK" <<'JS'
import fs from 'node:fs';import crypto from 'node:crypto';import path from 'node:path';
const root=process.argv[2],manifest=JSON.parse(fs.readFileSync(path.join(root,'manifest.json')));
for(const name of ['agent.mjs','jobs.mjs']){const actual=crypto.createHash('sha256').update(fs.readFileSync(path.join(root,name))).digest('hex');if(actual!==manifest.files[name])throw new Error(`Checksum failed for ${name}`);}
JS
"$NODE" --check "$WORK/agent.mjs"
"$NODE" --check "$WORK/jobs.mjs"
if [ ! -f "$BASE/config.json" ] || [ -n "$PAIR_CODE" ]; then
  if [ -z "$PAIR_CODE" ]; then read -r -p 'Pairing code from your dashboard: ' PAIR_CODE </dev/tty; fi
  if [ -z "$DEVICE_NAME" ]; then
    DEFAULT_NAME="$(hostname -s | tr '[:upper:] ' '[:lower:]-' | tr -cd 'a-z0-9._-')"
    read -r -p "Device name [$DEFAULT_NAME]: " DEVICE_NAME </dev/tty
    DEVICE_NAME="${DEVICE_NAME:-$DEFAULT_NAME}"
  fi
  export INFRA_PAIR_CODE="$PAIR_CODE" INFRA_PAIR_NAME="$DEVICE_NAME" INFRA_PAIR_FILE="$WORK/config.json" INFRA_PAIR_REQUEST="$WORK/pair-request.json"
  "$NODE" --input-type=module <<'JS'
import fs from 'node:fs';import os from 'node:os';import crypto from 'node:crypto';
fs.writeFileSync(process.env.INFRA_PAIR_REQUEST,JSON.stringify({code:process.env.INFRA_PAIR_CODE,device:process.env.INFRA_PAIR_NAME,hostname:os.hostname(),platform:process.platform,requestId:crypto.randomUUID()}),{mode:0o600});
JS
  PAIR_RESULT=0
  post_json pair "$WORK/pair-request.json" "$WORK/config.json" || PAIR_RESULT=$?
  if [ "$PAIR_RESULT" = 1 ]; then echo 'Could not reach the gateway to pair this device. Check your connection, then get a fresh code from the dashboard and retry.' >&2; exit 1; fi
  export INFRA_PAIR_RESULT="$PAIR_RESULT"
  "$NODE" --input-type=module <<'JS'
import fs from 'node:fs';
try {
  const c=JSON.parse(fs.readFileSync(process.env.INFRA_PAIR_FILE));
  if(process.env.INFRA_PAIR_RESULT!=='0')throw new Error(c.error||'Pairing was refused by the gateway.');
  if(!c.device||!c.token||!/^wss?:\/\//.test(c.server||''))throw new Error('The gateway returned an invalid pairing response.');
}catch(e){console.error('Pairing failed: '+(e instanceof SyntaxError?'The gateway returned an unreadable response.':e.message));process.exit(1);}
JS
  unset INFRA_PAIR_CODE PAIR_CODE
fi
stop_service
cp "$WORK/agent.mjs" "$BASE/agent.mjs"
cp "$WORK/jobs.mjs" "$BASE/jobs.mjs"
if [ -f "$WORK/config.json" ]; then mv "$WORK/config.json" "$BASE/config.json"; fi
chmod 700 "$BASE"; chmod 600 "$BASE/config.json"
# Stop the previous product's user startup, without deleting its files.
if [ "$SYSTEM" = 0 ]; then
  if [ "$OS" = Darwin ]; then launchctl bootout "gui/$(id -u)" "$HOME/Library/LaunchAgents/to.phloraxx.mcp.agent.plist" >/dev/null 2>&1 || true
  else systemctl --user stop phloraxx-mcp-agent.service >/dev/null 2>&1 || true; fi
fi
if [ "$MANUAL" = 1 ]; then
  echo "Installed. Manual start: $NODE $BASE/agent.mjs"
  echo 'Automatic startup was not configured (--manual).'
  exit 0
fi
export INFRA_SERVICE_BASE="$BASE" INFRA_SERVICE_NODE="$NODE" INFRA_SERVICE_SYSTEM="$SYSTEM" INFRA_SERVICE_OS="$OS" INFRA_SERVICE_IPV4FIRST="${INFRA_CURL_COMPAT:-0}"
if [ "$OS" = Darwin ]; then
  mkdir -p "$(dirname "$PLIST")"
  export INFRA_SERVICE_FILE="$PLIST"
  "$NODE" --input-type=module <<'JS'
import fs from 'node:fs';
const e=process.env,esc=s=>s.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
fs.writeFileSync(e.INFRA_SERVICE_FILE,`<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>Label</key><string>to.phloraxx.infra.agent</string><key>ProgramArguments</key><array><string>${esc(e.INFRA_SERVICE_NODE)}</string>${e.INFRA_SERVICE_IPV4FIRST==='1'?'<string>--dns-result-order=ipv4first</string>':''}<string>${esc(e.INFRA_SERVICE_BASE)}/agent.mjs</string></array><key>WorkingDirectory</key><string>${esc(e.INFRA_SERVICE_BASE)}</string><key>EnvironmentVariables</key><dict><key>PATH</key><string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string><key>INFRA_STARTUP</key><string>${e.INFRA_SERVICE_SYSTEM==='1'?'system-boot':'user-login'}</string></dict><key>RunAtLoad</key><true/><key>KeepAlive</key><true/><key>ThrottleInterval</key><integer>10</integer><key>StandardOutPath</key><string>${esc(e.INFRA_SERVICE_BASE)}/agent.log</string><key>StandardErrorPath</key><string>${esc(e.INFRA_SERVICE_BASE)}/agent.err.log</string></dict></plist>`,{mode:0o600});
JS
  if [ "$SYSTEM" = 1 ]; then launchctl bootstrap system "$PLIST"; else launchctl bootstrap "gui/$(id -u)" "$PLIST"; fi
else
  command -v systemctl >/dev/null || { echo 'No systemd found. Rerun with --manual, or configure your supervisor.' >&2; exit 1; }
  mkdir -p "$(dirname "$UNIT")"
  # systemd uses its own quoting rules; reject control characters in paths.
  case "$BASE$NODE" in *$'\n'*|*'"'*|*'%'*) echo 'Installation paths contain unsupported characters.' >&2; exit 1 ;; esac
  cat > "$UNIT" <<EOF
[Unit]
Description=Infra MCP device agent
After=network-online.target
[Service]
Type=simple
WorkingDirectory=$BASE
ExecStart="$NODE" "$BASE/agent.mjs"
Environment=INFRA_STARTUP=$([ "$SYSTEM" = 1 ] && echo system-boot || echo user-login)
Restart=always
RestartSec=10
KillMode=process
[Install]
WantedBy=$([ "$SYSTEM" = 1 ] && echo multi-user.target || echo default.target)
EOF
  if [ "$SYSTEM" = 1 ]; then systemctl daemon-reload; systemctl enable --now "$SERVICE.service"
  else systemctl --user daemon-reload; systemctl --user enable --now "$SERVICE.service"; fi
fi
export INFRA_CHECK_FILE="$BASE/config.json" INFRA_CHECK_REQUEST="$WORK/status-request.json" INFRA_CHECK_RESPONSE="$WORK/status-response.json"
"$NODE" --input-type=module <<'JS'
import fs from 'node:fs';
const c=JSON.parse(fs.readFileSync(process.env.INFRA_CHECK_FILE));
fs.writeFileSync(process.env.INFRA_CHECK_REQUEST,JSON.stringify({device:c.device,token:c.token}),{mode:0o600});
JS
CONNECTED=0
for attempt in 1 2 3 4 5 6 7 8; do
  if curl -fsS ${CURL_NETWORK_FLAGS[@]+"${CURL_NETWORK_FLAGS[@]}"} --connect-timeout 3 --max-time 8 -H 'Content-Type: application/json' --data-binary "@$WORK/status-request.json" -o "$WORK/status-response.json" "$BASE_URL/agent-status"; then
    if "$NODE" --input-type=module <<'JS'
import fs from 'node:fs';
try{const d=JSON.parse(fs.readFileSync(process.env.INFRA_CHECK_RESPONSE));if(d.online){console.log(`Connected as '${d.device}'.`);process.exit(0);}}catch{}
process.exit(1);
JS
    then CONNECTED=1; break; fi
  fi
  [ "$attempt" = 8 ] || sleep 1.5
done
if [ "$CONNECTED" != 1 ]; then echo "Installed, but connection is not confirmed. Startup is configured; the agent will reconnect automatically. Inspect $BASE/agent.err.log on Mac or the systemd journal on Linux." >&2; exit 1; fi
echo "Installed in $BASE"
if [ "$SYSTEM" = 1 ]; then echo 'Startup: system boot.'
elif [ "$OS" = Darwin ]; then echo 'Startup: after this Mac user signs in.'
else
  LINGER="$(loginctl show-user "$(id -un)" -p Linger --value 2>/dev/null || true)"
  if [ "$LINGER" = yes ]; then echo 'Startup: user service; lingering is enabled, so it can start at boot.'
  else echo 'Startup: after this Linux user signs in. For access while logged out, enable lingering with loginctl enable-linger, or install with --system.'; fi
fi
