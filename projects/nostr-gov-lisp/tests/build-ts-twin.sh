#!/usr/bin/env bash
# build-ts-twin.sh — compile the TypeScript twin (projects/nostr-gov-lisp/src/main.ts)
# to /tmp/gov-ts.wasm, the reference artifact consumed by diff-ts.sh,
# diff-ts-full.sh and diff-phase2.py.
#
# Pipeline (same as tests/test_ts_lending.rs): ts_frontend::ts_to_lisp_source
# lowers TS → lisp source, then the standard near pipeline parses, type-checks
# and emits. Reproducible from a fresh clone; no /tmp artifact needed.
set -eu
cd "$(dirname "$0")/../../.."   # lisp-rlm root

SRC=projects/nostr-gov-lisp/src/main.ts
OUT=${1:-/tmp/gov-ts.wasm}

[ -f "$SRC" ] || { echo "✗ $SRC not found"; exit 1; }
[ -x ./target/release/compile ] || cargo build --release --bin compile

rm -f "$OUT"   # never pass on a stale artifact
./target/release/compile "$SRC" "$OUT" 2>&1 | grep -vE '^(START|Reading|Parsed)' || true
[ -f "$OUT" ] || { echo "✗ compile failed — $OUT not produced"; exit 1; }

# Optional wasm-opt shrink pass (--enable-bulk-memory-opt required: emitted
# code uses memory.copy). Trace-equivalence re-verified after enabling.
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt --enable-bulk-memory-opt -Oz "$OUT" -o "$OUT.opt" \
    && wasm-tools validate "$OUT.opt" 2>/dev/null || true
  if [ -f "$OUT.opt" ]; then
    mv "$OUT.opt" "$OUT"
  fi
fi
echo "✅ TS twin ready: $OUT ($(wc -c < "$OUT" | tr -d ' ') bytes)"
