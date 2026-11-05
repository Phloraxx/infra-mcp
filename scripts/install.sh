#!/bin/sh
set -eu

REPO="Phloraxx/infra-mcp"
VERSION="v0.1.1-rc.2"
VERSION_REQUESTED=0
# Prereleases are excluded by GitHub releases/latest. Pin the published test tag.
GATEWAY=""
CODE=""
NAME=""
INSTALL_DIR="${INFRA_INSTALL_DIR:-$HOME/.local/bin}"
STATE_DIR="${INFRA_STATE_DIR:-$HOME/.infra-mcp-v1}"
SOURCE_DIR="${INFRA_INSTALL_SOURCE_DIR:-}"
NO_STARTUP=0

usage() {
  cat <<'EOF'
Infra installer

Usage:
  install.sh [options]   # open dashboard, approve the matching device code

Options:
  --gateway WS_URL        Advanced: alternate gateway; default: existing config or Infra public gateway
  --code CODE             Legacy manual enrollment only
  --name NAME             Legacy manual enrollment label
  --version VERSION        GitHub release tag; default: v0.1.1-rc.2 (test prerelease)
  --install-dir PATH       default: ~/.local/bin
  --state-dir PATH         default: ~/.infra-mcp-v1
  --source-dir PATH        copy locally built infra + infra-agent (development)
  --no-startup             configure/enroll only; do not register startup
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --gateway) GATEWAY=${2:?}; shift 2 ;;
    --code) CODE=${2:?}; shift 2 ;;
    --name) NAME=${2:?}; shift 2 ;;
    --version) VERSION=${2:?}; VERSION_REQUESTED=1; shift 2 ;;
    --install-dir) INSTALL_DIR=${2:?}; shift 2 ;;
    --state-dir) STATE_DIR=${2:?}; shift 2 ;;
    --source-dir) SOURCE_DIR=${2:?}; shift 2 ;;
    --no-startup) NO_STARTUP=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

# Preserve an enrolled machine's configured gateway on ordinary setup/repair.
# The native CLI loads that endpoint when --gateway is omitted.
if [ -z "$GATEWAY" ] && [ ! -f "$STATE_DIR/config.json" ]; then
  GATEWAY="wss://infra.drbijoy.in/agent"
fi
case "$(uname -s)" in
  Linux) PLATFORM=linux ;;
  Darwin) PLATFORM=macos ;;
  *) echo "Unsupported operating system: $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64|amd64) ARCH=x86_64 ;;
  arm64|aarch64) ARCH=aarch64 ;;
  *) echo "Unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

# Repeat setup is a repair, not an unrequested upgrade of running binaries.
# The CLI checks the existing identity and verifies a fresh online connection.
if [ "$VERSION_REQUESTED" -eq 0 ] && [ -z "$CODE" ] && [ -z "$SOURCE_DIR" ] &&
   [ -x "$INSTALL_DIR/infra" ] && [ -x "$INSTALL_DIR/infra-agent" ] &&
   "$INSTALL_DIR/infra" --build-info 2>/dev/null | grep -q '"activation_protocol":1'; then
  echo "Infra is already installed. Checking pairing and startup..."
  set -- setup --agent "$INSTALL_DIR/infra-agent" --state-dir "$STATE_DIR"
  [ -n "$GATEWAY" ] && set -- "$@" --gateway "$GATEWAY"
  [ "$NO_STARTUP" -eq 1 ] && set -- "$@" --no-startup
  exec "$INSTALL_DIR/infra" "$@"
fi

TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/infra-install.XXXXXX")
cleanup() {
  result=$?
  trap - EXIT INT TERM HUP
  rm -rf "$TMP_DIR"
  exit "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

download() {
  url=$1
  destination=$2
  if command -v curl >/dev/null 2>&1; then
    curl -fL --retry 3 --connect-timeout 10 --max-time 90 "$url" -o "$destination"
  elif command -v wget >/dev/null 2>&1; then
    wget --tries=3 --timeout=20 -O "$destination" "$url"
  else
    echo "Neither curl nor wget is available" >&2
    exit 1
  fi
}

verify_sha256() {
  file=$1
  expected_file=$2
  expected=$(awk '{print $1}' "$expected_file")
  [ -n "$expected" ] || { echo "Empty checksum for $file" >&2; exit 1; }
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$file" | awk '{print $1}')
  elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$file" | awk '{print $1}')
  else
    echo "No SHA-256 utility available" >&2
    exit 1
  fi
  [ "$actual" = "$expected" ] || { echo "SHA-256 verification failed for $file" >&2; exit 1; }
}

stage_binary() {
  name=$1
  staged="$TMP_DIR/$name"
  if [ -n "$SOURCE_DIR" ]; then
    [ -f "$SOURCE_DIR/$name" ] || { echo "Missing development binary $SOURCE_DIR/$name" >&2; exit 1; }
    cp "$SOURCE_DIR/$name" "$staged"
  else
    asset="$name-$PLATFORM-$ARCH"
    if [ "$VERSION" = latest ]; then
      base="https://github.com/$REPO/releases/latest/download"
    else
      base="https://github.com/$REPO/releases/download/$VERSION"
    fi
    download "$base/$asset" "$TMP_DIR/$asset"
    download "$base/$asset.sha256" "$TMP_DIR/$asset.sha256"
    verify_sha256 "$TMP_DIR/$asset" "$TMP_DIR/$asset.sha256"
    cp "$TMP_DIR/$asset" "$staged"
  fi
  chmod 755 "$staged"
}

printf '%s\n' "Infra · downloading native binaries"
stage_binary infra
stage_binary infra-agent
# Both must exist before an existing installation is changed.
[ -s "$TMP_DIR/infra" ] && [ -s "$TMP_DIR/infra-agent" ] || {
  echo "Infra binaries are missing or empty; the previous install is unchanged" >&2
  exit 1
}
if [ -n "$CODE" ]; then
  [ -n "$GATEWAY" ] || { echo "Legacy enrollment needs --gateway" >&2; exit 2; }
  set -- activate --source-dir "$TMP_DIR" --install-dir "$INSTALL_DIR" --state-dir "$STATE_DIR" --gateway "$GATEWAY"
else
  set -- activate --source-dir "$TMP_DIR" --install-dir "$INSTALL_DIR" --state-dir "$STATE_DIR"
  [ -n "$GATEWAY" ] && set -- "$@" --gateway "$GATEWAY"
fi
[ -n "$CODE" ] && set -- "$@" --code "$CODE"
[ -n "$NAME" ] && [ -n "$CODE" ] && set -- "$@" --name "$NAME"
[ "$NO_STARTUP" -eq 1 ] && set -- "$@" --no-startup
# The staged CLI owns locks, pair activation, rollback and native setup.
# Older release binaries must fail before any installed file is replaced.
"$TMP_DIR/infra" --build-info | grep -q '"activation_protocol":1' || {
  echo "This release predates safe native activation; use a compatible candidate release." >&2
  exit 1
}
"$TMP_DIR/infra" "$@"
printf '\nInstalled:\n  CLI:   %s\n  Agent: %s\n  State: %s\n' "$INSTALL_DIR/infra" "$INSTALL_DIR/infra-agent" "$STATE_DIR"
