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
RELEASE_BASE_URL=""
ALLOW_CHECKSUM_ONLY=0
# Publisher key for the signed release manifest. It is public; the private
# counterpart never leaves the owner's offline environment and the GitHub
# Actions secret. Confirm it matches docs/RELEASE_PUBLISHER_KEY.md before
# trusting a first install, and update it deliberately on key rotation.
RELEASE_PUBLIC_KEY_B64="DItK3Wlg98cSI7ptTNOd0oTq1I42paHDygEnEFc3eVs="
if [ -n "${INFRA_BOOTSTRAP_PUBLIC_KEY_B64:-}" ]; then
  RELEASE_PUBLIC_KEY_B64=$INFRA_BOOTSTRAP_PUBLIC_KEY_B64
  echo "Warning: verifying against INFRA_BOOTSTRAP_PUBLIC_KEY_B64 instead of the pinned publisher key (local acceptance only)." >&2
fi

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
  --release-base-url URL   Local acceptance: read release files from URL instead of GitHub
  --allow-checksum-only    UNSAFE: skip the signed manifest and trust a checksum sidecar
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
    --release-base-url) RELEASE_BASE_URL=${2:?}; shift 2 ;;
    --allow-checksum-only) ALLOW_CHECKSUM_ONLY=1; shift ;;
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

release_base() {
  if [ -n "$RELEASE_BASE_URL" ]; then
    printf '%s' "$RELEASE_BASE_URL"
  elif [ "$VERSION" = latest ]; then
    printf 'https://github.com/%s/releases/latest/download' "$REPO"
  else
    printf 'https://github.com/%s/releases/download/%s' "$REPO" "$VERSION"
  fi
}

# The verifier travels with this script so a compromised release host cannot
# substitute both the manifest and the check. It is byte-identical to
# scripts/verify-release-manifest.py; scripts/test-install-manifest.py fails
# when the two drift apart.
write_manifest_verifier() {
  cat > "$TMP_DIR/verify-release-manifest.py" <<'INFRA_MANIFEST_VERIFIER'
#!/usr/bin/env python3
"""Verify a signed Infra release manifest using the standard library only.

The bootstrap installer runs this before executing any downloaded binary, so
it must not need a package manager, a network call or a third-party module.
Ed25519 verification follows RFC 8032 section 6 (the reference implementation
from the RFC appendix, adapted to reject non-canonical inputs).

Exit status 0 means the manifest signature is valid for the pinned public key,
the manifest version matches --expect-version when given, and every requested
asset digest matches the signed manifest. Anything else exits non-zero with a
single actionable line on stderr.
"""
import argparse
import base64
import hashlib
import sys

P = 2**255 - 19
D = -121665 * pow(121666, P - 2, P) % P
Q = 2**252 + 27742317777372353535851937790883648493
I = pow(2, (P - 1) // 4, P)


def inv(value):
    return pow(value, P - 2, P)


def x_recover(y):
    xx = (y * y - 1) * inv(D * y * y + 1)
    x = pow(xx, (P + 3) // 8, P)
    if (x * x - xx) % P != 0:
        x = (x * I) % P
    if x % 2 != 0:
        x = P - x
    return x


def point_add(left, right):
    a = (left[1] - left[0]) * (right[1] - right[0]) % P
    b = (left[1] + left[0]) * (right[1] + right[0]) % P
    c = 2 * left[3] * right[3] * D % P
    d = 2 * left[2] * right[2] % P
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % P, g * h % P, f * g % P, e * h % P)


def point_mul(scalar, point):
    result = (0, 1, 1, 0)
    while scalar > 0:
        if scalar & 1:
            result = point_add(result, point)
        point = point_add(point, point)
        scalar >>= 1
    return result


def point_equal(left, right):
    if (left[0] * right[2] - right[0] * left[2]) % P != 0:
        return False
    return (left[1] * right[2] - right[1] * left[2]) % P == 0


def on_curve(point):
    x, y, z, t = point
    return (-x * x + y * y - z * z - D * t * t) % P == 0


def point_decompress(data):
    if len(data) != 32:
        return None
    y = int.from_bytes(data, 'little')
    sign = y >> 255
    y &= (1 << 255) - 1
    x = x_recover(y)
    if (x & 1) != sign:
        x = P - x
    point = (x, y, 1, x * y % P)
    return point if on_curve(point) else None


G_Y = 4 * inv(5) % P
G = (x_recover(G_Y), G_Y, 1, x_recover(G_Y) * G_Y % P)


def verify(signature, message, public_key):
    if len(signature) != 64 or len(public_key) != 32:
        return False
    r = point_decompress(signature[:32])
    a = point_decompress(public_key)
    if r is None or a is None:
        return False
    s = int.from_bytes(signature[32:], 'little')
    if s >= Q:
        return False
    digest = hashlib.sha512(signature[:32] + public_key + message).digest()
    h = int.from_bytes(digest, 'little') % Q
    return point_equal(point_mul(s, G), point_add(r, point_mul(h, a)))


def fail(message):
    print(f'infra: {message}', file=sys.stderr)
    raise SystemExit(1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', required=True)
    parser.add_argument('--signature', required=True)
    parser.add_argument('--public-key-b64', required=True)
    parser.add_argument('--directory', required=True,
                        help='directory the requested assets are read from')
    parser.add_argument('--asset', action='append', default=[],
                        help='asset name to digest-check; repeatable')
    parser.add_argument('--expect-version',
                        help='refuse a manifest whose version differs from this tag')
    args = parser.parse_args()

    try:
        public_key = base64.b64decode(args.public_key_b64.strip(), validate=True)
    except ValueError:
        fail('the pinned release public key is not valid base64')
    if len(public_key) != 32:
        fail('the pinned release public key must be 32 bytes')
    try:
        with open(args.manifest, 'rb') as handle:
            manifest_bytes = handle.read()
        with open(args.signature, 'rb') as handle:
            signature = base64.b64decode(handle.read().strip(), validate=True)
    except (OSError, ValueError) as error:
        fail(f'cannot read the signed release manifest: {error}')
    if not verify(signature, manifest_bytes, public_key):
        fail('the release manifest is not signed by the pinned publisher key; '
             'no downloaded binary was executed')

    import json
    try:
        manifest = json.loads(manifest_bytes)
        version = manifest['version']
        assets = manifest['assets']
    except (ValueError, KeyError, TypeError):
        fail('the signed release manifest is malformed')
    if not isinstance(version, str) or not version:
        fail('the signed release manifest has no version')
    if args.expect_version and args.expect_version != 'latest' and version != args.expect_version:
        fail(f'the signed manifest is for {version}, not the requested {args.expect_version}')
    for name in args.asset:
        expected = assets.get(name) if isinstance(assets, dict) else None
        if not isinstance(expected, str) or len(expected) != 64:
            fail(f'the signed manifest does not list {name}')
        try:
            with open(f'{args.directory}/{name}', 'rb') as handle:
                digest = hashlib.sha256(handle.read()).hexdigest()
        except OSError as error:
            fail(f'cannot read {name}: {error}')
        if digest != expected:
            fail(f'SHA-256 mismatch for {name}: the signed manifest lists {expected}')
    print(version)


if __name__ == '__main__':
    main()
INFRA_MANIFEST_VERIFIER
}

stage_binary() {
  name=$1
  staged="$TMP_DIR/$name"
  if [ -n "$SOURCE_DIR" ]; then
    [ -f "$SOURCE_DIR/$name" ] || { echo "Missing development binary $SOURCE_DIR/$name" >&2; exit 1; }
    cp "$SOURCE_DIR/$name" "$staged"
  else
    asset="$name-$PLATFORM-$ARCH"
    base=$(release_base)
    download "$base/$asset" "$TMP_DIR/$asset"
    download "$base/$asset.sha256" "$TMP_DIR/$asset.sha256"
    verify_sha256 "$TMP_DIR/$asset" "$TMP_DIR/$asset.sha256"
    cp "$TMP_DIR/$asset" "$staged"
  fi
  chmod 755 "$staged"
}

# The signed manifest binds the version, the release index and every asset
# digest to the publisher key, so a substituted binary is refused before it
# is ever executed.
stage_signed_binaries() {
  base=$(release_base)
  download "$base/infra-release.json" "$TMP_DIR/infra-release.json"
  download "$base/infra-release.json.sig" "$TMP_DIR/infra-release.json.sig"
  for name in infra infra-agent; do
    download "$base/$name-$PLATFORM-$ARCH" "$TMP_DIR/$name-$PLATFORM-$ARCH"
  done
  command -v python3 >/dev/null 2>&1 || {
    echo "python3 is required to verify the signed release manifest before any downloaded binary runs." >&2
    echo "Install python3, or pass --allow-checksum-only to accept an unauthenticated install (unsafe)." >&2
    exit 1
  }
  write_manifest_verifier
  python3 "$TMP_DIR/verify-release-manifest.py" \
    --manifest "$TMP_DIR/infra-release.json" \
    --signature "$TMP_DIR/infra-release.json.sig" \
    --public-key-b64 "$RELEASE_PUBLIC_KEY_B64" \
    --directory "$TMP_DIR" \
    --expect-version "$VERSION" \
    --asset "infra-$PLATFORM-$ARCH" \
    --asset "infra-agent-$PLATFORM-$ARCH" >/dev/null || {
    echo "The signed release manifest did not verify; no downloaded binary was executed." >&2
    exit 1
  }
  cp "$TMP_DIR/infra-$PLATFORM-$ARCH" "$TMP_DIR/infra"
  cp "$TMP_DIR/infra-agent-$PLATFORM-$ARCH" "$TMP_DIR/infra-agent"
  chmod 755 "$TMP_DIR/infra" "$TMP_DIR/infra-agent"
}

printf '%s\n' "Infra · downloading native binaries"
if [ -n "$SOURCE_DIR" ]; then
  stage_binary infra
  stage_binary infra-agent
elif [ "$ALLOW_CHECKSUM_ONLY" -eq 1 ]; then
  echo "Warning: --allow-checksum-only skips the signed release manifest. A checksum served from the same host is an integrity check, not publisher authentication." >&2
  stage_binary infra
  stage_binary infra-agent
else
  stage_signed_binaries
fi
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
