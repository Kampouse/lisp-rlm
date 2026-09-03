#!/usr/bin/env bash
# diff-ts.sh — differential: lisp twin vs TS port, legacy-auth Phase-1 vectors.
# Same step sequence, fresh state per wasm, compare per-step outcome
# (abort ERR_* or view return) and the full log trace.
set -u
# Isolated state: never share /tmp/near-mock-state.bin across runs/sessions.
STATE_DIR="$(mktemp -d)"
trap 'rm -rf "$STATE_DIR"' EXIT
export NEAR_MOCK_STATE="$STATE_DIR/state.bin"
cd "$(dirname "$0")/../../.."   # lisp-rlm root
LISP_W=projects/nostr-gov-lisp/target/nostr-gov-lisp.wasm
TS_W=/tmp/gov-ts.wasm
MOCK=./target/release/near-mock
PY=python3

[ -x "$MOCK" ] || { echo "no near-mock"; exit 1; }

TSNS=$($PY -c "import time; print(int(time.time()*1e9))")

# legacy-only steps: drop event-auth (ev key) and test_verify_nostr (not ported)
$PY projects/nostr-gov-lisp/tests/gen-vectors.py $TSNS \
  | $PY -c '
import json,sys
for line in sys.stdin:
    d=json.loads(line)
    if d["method"]=="test_verify_nostr": continue
    if "ev" in d["args"]: continue
    print(json.dumps(d))' > /tmp/diff-steps.jsonl

N=$($PY -c 'print(sum(1 for _ in open("/tmp/diff-steps.jsonl")))')

run_all () {  # $1=wasm $2=outfile
  local W=$1 OUT=$2 i=0
  : > "$OUT"
  "$MOCK" "$W" reset >/dev/null 2>&1
  while IFS= read -r line; do
    i=$((i+1))
    M=$($PY -c 'import json,sys;print(json.loads(sys.argv[1])["method"])' "$line")
    A=$($PY -c 'import json,sys;print(json.dumps(json.loads(sys.argv[1])["args"],separators=(",",":")))' "$line")
    V=""
    case "$M" in get_owner_nonce|is_paused|get_version|get_wallet) V="--view";; esac
    OUT1=$("$MOCK" "$W" "$M" "$A" $V 2>&1)
    ERR=$(echo "$OUT1" | grep -oE 'ERR_[A-Z_]+' | head -1)
    RET=$(echo "$OUT1" | grep -oE '📄 .*' | head -1 | sed 's/^📄 //')
    RVAL=$($PY -c 'import json,sys
s=sys.argv[1]
try:
  d=json.loads(s); print(d.get("result",""))
except Exception: print("")' "$RET")
    if [ -n "$ERR" ]; then R="$ERR"; else R="$RVAL"; fi
    echo "#$i $M -> $R" >> "$OUT"
    echo "$OUT1" | grep -E 'LOG:|📄' | sed "s|^|#$i |" >> "$OUT.full"
  done < /tmp/diff-steps.jsonl
}

run_all "$LISP_W" /tmp/diff-lisp.txt
run_all "$TS_W"    /tmp/diff-ts.txt

echo "── differential: $N steps"
if diff -u /tmp/diff-lisp.txt /tmp/diff-ts.txt > /tmp/diff-out.txt; then
  echo "✅ TRACE-EQUIVALENT — all $N steps identical"
  cat /tmp/diff-lisp.txt
else
  echo "❌ DIVERGENCE:"
  cat /tmp/diff-out.txt
  exit 1
fi
