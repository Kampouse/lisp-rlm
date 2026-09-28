#!/usr/bin/env bash
# mock regression: grad_th overflow fix + per-token fee split (v3.1)
# build first:  <lisp-rlm>/target/release/near-compile projects/launchpad/pool.ts /tmp/pool_v31.wasm
set -e
HERE="$(cd "$(dirname "$0")" && pwd)"; M="$HERE/../../../target/release/near-mock"
[ -x "$M" ] || M="$HOME/dev/lisp-rlm/target/release/near-mock"
for s in grad_th_differential fee_split slippage_guard; do
  echo "== $s =="; "$M" scenario "$HERE/$s.json" | tail -1
done
