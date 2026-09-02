#!/usr/bin/env bash
# run-gauntlet.sh — drive near-mock through gen-vectors.py steps and diff
# against expected outcomes. Usage: tests/run-gauntlet.sh [ts_ns]
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
MOCK="$ROOT/target/release/near-mock"
W="$HERE/../target/nostr-gov-lisp.wasm"
PY=python3

pass=0; fail=0; i=0
fails=()
"$MOCK" "$W" reset >/dev/null 2>&1
TSNS="$($PY -c 'import time;print(int(time.time()*1e9))')"
while IFS= read -r line; do
  i=$((i+1))
  M=$($PY -c 'import json,sys;print(json.loads(sys.argv[1])["method"])' "$line")
  A=$($PY -c 'import json,sys;print(json.dumps(json.loads(sys.argv[1])["args"],separators=(",",":")))' "$line")
  E=$($PY -c 'import json,sys;print(json.loads(sys.argv[1])["expect"])' "$line")
  D=$($PY -c 'import json,sys;print(json.loads(sys.argv[1]).get("deposit",0))' "$line")
  V=""
  case "$M" in get_owner_nonce|is_paused|get_version|get_wallet|get_proposal|get_approvers) V="--view";; esac
  export NEAR_MOCK_ATTACH="$D"
  OUT=$("$MOCK" "$W" "$M" "$A" $V 2>&1)
  unset NEAR_MOCK_ATTACH
  ERR=$(echo "$OUT" | grep -oE 'LOG: ERR_[A-Z_]+' | head -1 | sed 's/^LOG: //')
  RET=$(echo "$OUT" | grep -oE '📄 .*' | head -1 | sed 's/^📄 //')
  RVAL=$($PY -c 'import json,sys
s=sys.argv[1]
try:
  d=json.loads(s); print(d.get("result",""))
except Exception: print("")' "$RET")
  if [ -n "$ERR" ]; then R="$ERR"; else R="$RVAL"; fi
  if [ "$E" = "ok" ]; then
    OK=$([ -z "$ERR" ] && echo 1 || echo 0)
  elif [ "$E" = "active" ] || [ "$E" = "approved" ] || [ "$E" = "executed" ]; then
    OK=$(echo "$RET" | grep -q "\"st\":\"$E\"" && echo 1 || echo 0)
  else
    OK=$([ "$E" = "$R" ] && echo 1 || echo 0)
  fi
  if [ "$OK" = "1" ]; then
    pass=$((pass+1))
  else
    fail=$((fail+1))
    fails+=("#$i $M expect=[$E] got=[$R]")
  fi
done < <($PY "$HERE/gen-vectors.py" "${1:-$TSNS}")

echo "── gauntlet: $pass pass / $fail fail / $i total"
for f in "${fails[@]:-}"; do [ -n "$f" ] && echo "  ✗ $f"; done
[ "$fail" = "0" ]
