#!/bin/sh
# CLMM v5 release build: trio → near-compile → wasm-opt -Oz.
#
# The module emits memory.copy, so bulk-memory must be enabled for the
# optimizer (NEAR's runtime supports it). -Oz measured −26% size and
# −29% gas on grid; every artifact that ships MUST have passed the full
# gate (CLMM_POOL_WASM=... python3 scripts/clmm_v5_ts_parity.py) first.
set -e
cd "$(dirname "$0")/.."
NC=target/release/near-compile
OPT=target/pool_v5_raw.wasm
OUT=target/pool_v5_opt.wasm

cat lib/limb_math.lisp lib/clmm_v5_kernel.lisp lib/clmm_pool_v5.lisp > /tmp/pool_v5_trio.lisp
"$NC" /tmp/pool_v5_trio.lisp "$OPT"
wasm-opt --enable-bulk-memory -Oz "$OPT" -o "$OUT"
echo "raw: $(wc -c < "$OPT" | tr -d ' ')  opt: $(wc -c < "$OUT" | tr -d ' ')"
echo "gate it: CLMM_POOL_WASM=$OUT python3 scripts/clmm_v5_ts_parity.py"
