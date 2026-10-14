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
download() { curl -fsSL --retry 2 --connect-timeout 15 --max-time 180 "$1" -o "$2"; }
digest() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi; }
echo 'Infra MCP · device agent'
mkdir -p "$BASE"
NODE="$(command -v node || true)"
if [ -z "$NODE" ] || ! "$NODE" -e 'process.exit(Number(process.versions.node.split(".")[0])>=22?0:1)' >/dev/null 2>&1; then
  ARCH="$(uname -m)"
  case "$ARCH" in x86_64) ARCH=x64 ;; arm64|aarch64) ARCH=arm64 ;; *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;; esac
  PLATFORM=linux; [ "$OS" != Darwin ] || PLATFORM=darwin
  if [ "$OS" = Linux ] && ldd --version 2>&1 | head -n 1 | grep -qi musl; then PLATFORM=linux; ARCH="$ARCH-musl"; fi
  ARCHIVE="node-$NODE_VERSION-$PLATFORM-$ARCH.tar.gz"
  echo 'Installing a private Node runtime. Administrator access is not required.'
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
  export INFRA_PAIR_BASE="$BASE_URL" INFRA_PAIR_CODE="$PAIR_CODE" INFRA_PAIR_NAME="$DEVICE_NAME" INFRA_PAIR_FILE="$WORK/config.json"
  "$NODE" --input-type=module <<'JS'
import fs from 'node:fs';import os from 'node:os';
const r=await fetch(process.env.INFRA_PAIR_BASE+'/pair',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({code:process.env.INFRA_PAIR_CODE,device:process.env.INFRA_PAIR_NAME,hostname:os.hostname(),platform:process.platform}),signal:AbortSignal.timeout(20000)});
const config=await r.json();if(!r.ok)throw new Error(config.error||'Pairing failed');fs.writeFileSync(process.env.INFRA_PAIR_FILE,JSON.stringify(config),{mode:0o600});
JS
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
export INFRA_SERVICE_BASE="$BASE" INFRA_SERVICE_NODE="$NODE" INFRA_SERVICE_SYSTEM="$SYSTEM" INFRA_SERVICE_OS="$OS"
if [ "$OS" = Darwin ]; then
  mkdir -p "$(dirname "$PLIST")"
  export INFRA_SERVICE_FILE="$PLIST"
  "$NODE" --input-type=module <<'JS'
import fs from 'node:fs';
const e=process.env,esc=s=>s.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
fs.writeFileSync(e.INFRA_SERVICE_FILE,`<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>Label</key><string>to.phloraxx.infra.agent</string><key>ProgramArguments</key><array><string>${esc(e.INFRA_SERVICE_NODE)}</string><string>${esc(e.INFRA_SERVICE_BASE)}/agent.mjs</string></array><key>WorkingDirectory</key><string>${esc(e.INFRA_SERVICE_BASE)}</string><key>EnvironmentVariables</key><dict><key>PATH</key><string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin</string><key>INFRA_STARTUP</key><string>${e.INFRA_SERVICE_SYSTEM==='1'?'system-boot':'user-login'}</string></dict><key>RunAtLoad</key><true/><key>KeepAlive</key><true/><key>ThrottleInterval</key><integer>10</integer><key>StandardOutPath</key><string>${esc(e.INFRA_SERVICE_BASE)}/agent.log</string><key>StandardErrorPath</key><string>${esc(e.INFRA_SERVICE_BASE)}/agent.err.log</string></dict></plist>`,{mode:0o600});
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
export INFRA_CHECK_FILE="$BASE/config.json" INFRA_CHECK_BASE="$BASE_URL"
"$NODE" --input-type=module <<'JS'
import fs from 'node:fs';import {setTimeout} from 'node:timers/promises';
const c=JSON.parse(fs.readFileSync(process.env.INFRA_CHECK_FILE));
for(let i=0;i<8;i++){const r=await fetch(process.env.INFRA_CHECK_BASE+'/agent-status',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({device:c.device,token:c.token}),signal:AbortSignal.timeout(10000)});const d=await r.json();if(d.online){console.log(`Connected as '${c.device}'.`);process.exit(0);}await setTimeout(1500);}
console.error('Installed, but connection is not confirmed. Inspect the agent log and gateway URL.');process.exit(1);
JS
echo "Installed in $BASE"
if [ "$SYSTEM" = 1 ]; then echo 'Startup: system boot.'
elif [ "$OS" = Darwin ]; then echo 'Startup: after this Mac user signs in.'
else
  LINGER="$(loginctl show-user "$(id -un)" -p Linger --value 2>/dev/null || true)"
  if [ "$LINGER" = yes ]; then echo 'Startup: user service; lingering is enabled, so it can start at boot.'
  else echo 'Startup: after this Linux user signs in. For access while logged out, enable lingering with loginctl enable-linger, or install with --system.'; fi
fi
