#!/usr/bin/env bash
# End-to-end check of the built binary: serves the file list, highlights a
# blob, and keeps the path-traversal guard.
set -euo pipefail

BIN="$TEST_SRCDIR/$TEST_WORKSPACE/wview"

"$BIN" --help | grep -q '^Usage: wview \[ROOT\] \[PORT\]$'
# --version must report the Cargo.toml version, not rules_rust's 0.0.0 default.
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$TEST_SRCDIR/$TEST_WORKSPACE/Cargo.toml")
test -n "$VERSION"
"$BIN" --version | grep -qx "wview $VERSION"

FIXTURE="$TEST_TMPDIR/fixture"
mkdir -p "$FIXTURE/sub"
printf 'def foo():\n    pass\n' > "$FIXTURE/f.py"
printf 'fn main() {}\n' > "$FIXTURE/sub/lib.rs"
# Exists but sits outside the served root: reaching it must fail on the
# confinement check, not on canonicalization.
printf 'secret\n' > "$TEST_TMPDIR/outside"

# Port 0: the server picks an ephemeral port and prints the bound address.
"$BIN" "$FIXTURE" 0 > "$TEST_TMPDIR/server.log" &
SERVER=$!
trap 'kill "$SERVER" 2>/dev/null || true' EXIT

BASE=""
for _ in $(seq 1 50); do
  BASE=$(sed -n 's|^wview: .* at \(http://[0-9.:]*\)/$|\1|p' "$TEST_TMPDIR/server.log")
  if [ -n "$BASE" ] && curl -fsS "$BASE/" >/dev/null 2>&1; then break; fi
  sleep 0.1
done
test -n "$BASE"

curl -fsS "$BASE/" | grep -q 'data-p="f.py"'
curl -fsS "$BASE/" | grep -q 'data-p="sub/lib.rs"'
curl -fsS "$BASE/blob/f.py" | grep -q 'id="L1"'
# Embedded vite-built bundle is served and referenced by pages.
curl -fsS "$BASE/web/wview.js" | grep -q 'wviewComments'
curl -fsS "$BASE/blob/f.py" | grep -q 'src="/web/wview.js"'
STATUS=$(curl -s -o /dev/null -w '%{http_code}' --path-as-is "$BASE/blob/../outside")
test "$STATUS" = 404
BODY=$(curl -s --path-as-is "$BASE/blob/../outside")
[[ "$BODY" != *secret* ]]

echo PASS
