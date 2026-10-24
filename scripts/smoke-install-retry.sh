#!/bin/sh
# Portable, offline failure injection: no live identities, services or gateway.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/new" "$tmp/old" "$tmp/installed" "$tmp/state"
printf '#!/bin/sh\nexit 23\n' > "$tmp/new/infra"
printf '#!/bin/sh\nexit 0\n' > "$tmp/new/infra-agent"
printf '#!/bin/sh\necho old-cli\n' > "$tmp/installed/infra"
printf '#!/bin/sh\necho old-agent\n' > "$tmp/installed/infra-agent"
chmod +x "$tmp"/new/* "$tmp"/installed/*
cp "$tmp/installed/infra" "$tmp/old/infra"
cp "$tmp/installed/infra-agent" "$tmp/old/infra-agent"

# Failed activation/setup may never clobber a previously installed two-binary set.
if "$root/scripts/install.sh" --gateway ws://127.0.0.1:1/agent --source-dir "$tmp/new" --install-dir "$tmp/installed" --state-dir "$tmp/state" --no-startup > "$tmp/output" 2>&1; then
  echo "Expected injected setup failure" >&2; exit 1
fi
cmp "$tmp/old/infra" "$tmp/installed/infra"
cmp "$tmp/old/infra-agent" "$tmp/installed/infra-agent"
test -z "$(find "$tmp/installed" -name '.*stage*' -o -name '*rollback*')"

# On fresh installs retain both binaries so a timed-out approval is resumable.
mkdir -p "$tmp/fresh"
if "$root/scripts/install.sh" --gateway ws://127.0.0.1:1/agent --source-dir "$tmp/new" --install-dir "$tmp/fresh" --state-dir "$tmp/state" --no-startup > "$tmp/output" 2>&1; then
  echo "Expected fresh setup failure" >&2; exit 1
fi
cmp "$tmp/new/infra" "$tmp/fresh/infra"
cmp "$tmp/new/infra-agent" "$tmp/fresh/infra-agent"

# A retry of a valid installation never redownloads and does not prompt for a name/code.
cat > "$tmp/installed/infra" <<'SH'
#!/bin/sh
if [ "$1" = help ]; then echo "infra setup [--gateway]"; exit 0; fi
[ "$1" = setup ] || exit 24
printf '%s\n' "$@" > "$INFRA_TEST_SETUP_ARGS"
SH
chmod +x "$tmp/installed/infra"
INFRA_TEST_SETUP_ARGS="$tmp/args" "$root/scripts/install.sh" --install-dir "$tmp/installed" --state-dir "$tmp/state" --no-startup > "$tmp/output"
grep -qx -- 'setup' "$tmp/args"
grep -qx -- '--no-startup' "$tmp/args"
! grep -q -- '--code' "$tmp/args"
! grep -q -- '--name' "$tmp/args"
cmp "$tmp/old/infra-agent" "$tmp/installed/infra-agent"
echo "Installer failure/retry acceptance passed."
