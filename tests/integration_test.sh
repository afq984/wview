#!/usr/bin/env bash
# End-to-end check of the built binary: serves the file list, highlights a
# blob, and keeps the path-traversal guard.
set -euo pipefail

BIN="$TEST_SRCDIR/$TEST_WORKSPACE/wview"
PORT=18997
BASE="http://127.0.0.1:$PORT"

FIXTURE="$TEST_TMPDIR/fixture"
mkdir -p "$FIXTURE/sub"
printf 'def foo():\n    pass\n' > "$FIXTURE/f.py"
printf 'fn main() {}\n' > "$FIXTURE/sub/lib.rs"
# Exists but sits outside the served root: reaching it must fail on the
# confinement check, not on canonicalization.
printf 'secret\n' > "$TEST_TMPDIR/outside"

"$BIN" "$FIXTURE" "$PORT" &
SERVER=$!
trap 'kill "$SERVER" 2>/dev/null || true' EXIT

for _ in $(seq 1 50); do
  if curl -fsS "$BASE/" >/dev/null 2>&1; then break; fi
  sleep 0.1
done

curl -fsS "$BASE/" | grep -q 'data-p="f.py"'
curl -fsS "$BASE/" | grep -q 'data-p="sub/lib.rs"'
curl -fsS "$BASE/blob/f.py" | grep -q 'id="L1"'
STATUS=$(curl -s -o /dev/null -w '%{http_code}' --path-as-is "$BASE/blob/../outside")
test "$STATUS" = 404
BODY=$(curl -s --path-as-is "$BASE/blob/../outside")
[[ "$BODY" != *secret* ]]

echo PASS
