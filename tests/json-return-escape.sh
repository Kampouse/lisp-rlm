#!/usr/bin/env bash
# json_return_str must JSON-ESCAPE the returned string (quotes, backslash,
# control chars). Regression for the naive-concat bug that made every view
# returning embedded JSON invalid (found live on testnet 2026-09-03).
set -euo pipefail
cd "$(dirname "$0")/.."
[ -x ./target/release/compile ] || cargo build --release --bin compile
[ -x ./target/release/near-mock ] || cargo build --release --bin near-mock
T=$(mktemp -d)
cat > "$T/t.lisp" <<'LISP'
(define (get1) (near/json_return_str "{\"id\":\"1\",\"st\":\"executed\"}"))
(define (get2) (near/json_return_str "a\\b\"c"))
(export "get1" get1 #t)
(export "get2" get2 #t)
LISP
./target/release/compile "$T/t.lisp" "$T/t.wasm" >/dev/null 2>&1
OUT1=$(./target/release/near-mock "$T/t.wasm" get1 '{}' --view 2>&1)
OUT2=$(./target/release/near-mock "$T/t.wasm" get2 '{}' --view 2>&1)
python3 - "$OUT1" "$OUT2" <<'PY'
import json, sys
def result_of(out):
    line = next(l for l in out.split("\n") if "\U0001f4c4" in l)
    return json.loads(line.split("\U0001f4c4", 1)[1].strip())["result"]
# 1) embedded JSON must survive round-trip
inner = json.loads(result_of(sys.argv[1]))
assert inner == {"id": "1", "st": "executed"}, inner
# 2) raw backslash+quote must be escaped (a\b"c → a\\b\"c)
assert result_of(sys.argv[2]) == 'a\\b"c', result_of(sys.argv[2])
print("json-return-escape: PASS")
PY
rm -rf "$T"
