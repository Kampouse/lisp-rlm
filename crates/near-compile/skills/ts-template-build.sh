#!/usr/bin/env bash
# build.sh — compile the lisp-rlm-dialect TypeScript contract to NEAR wasm.
#
# Requires the lisp-rlm toolchain (github.com/Kampouse/lisp-rlm):
#   LISP_RLM_ROOT=<path>   required — point at your lisp-rlm checkout
#
# Or install near-compile globally: cargo install near-compile
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
PROJECT="<name>"
OUT="${1:-$HERE/target/${PROJECT}.wasm}"
SRC="$HERE/src/main.ts"

# resolve lisp-rlm root
LR="${LISP_RLM_ROOT:-}"
if [ -z "$LR" ]; then
  for c in "$HERE/../lisp-rlm" "$HERE/../../lisp-rlm"; do
    [ -d "$c" ] && LR="$c" && break
  done
fi

# pick a compiler binary: release → debug → installed
NC=""
if [ -n "$LR" ]; then
  NC="$LR/target/release/near-compile"
  [ -x "$NC" ] || NC="$LR/target/debug/near-compile"
fi
if [ -z "$NC" ] || [ ! -x "$NC" ]; then
  if command -v near-compile >/dev/null 2>&1; then
    NC="$(command -v near-compile)"
  elif [ -n "$LR" ]; then
    echo "→ building near-compile at $LR (cargo build --release)"
    cargo build --manifest-path "$LR/Cargo.toml" --release --bin near-compile
    NC="$LR/target/release/near-compile"
  fi
fi
[ -n "$NC" ] && [ -x "$NC" ] || { echo "✗ near-compile not found"; echo "  Set LISP_RLM_ROOT or run: cargo install near-compile"; exit 1; }

mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"
"$NC" "$SRC" "$OUT" 2>&1 | grep -vE '^(START|Reading|Parsed)' || true
[ -f "$OUT" ] || { echo "✗ compile failed — $OUT not produced"; exit 1; }

# optional wasm-opt shrink
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt --enable-bulk-memory-opt -g -Oz "$OUT" -o "$OUT.opt" \
    && wasm-tools validate "$OUT.opt" 2>/dev/null || true
  if [ -f "$OUT.opt" ]; then mv "$OUT.opt" "$OUT"; fi
fi

# sync to frontend if it exists
if [ -d "$HERE/../frontend/public" ]; then
  cp "$OUT" "$HERE/../frontend/public/${PROJECT}.wasm"
  echo "📋 synced → ../frontend/public/${PROJECT}.wasm ($(wc -c < "$HERE/../frontend/public/${PROJECT}.wasm" | tr -d ' ') bytes)"
fi
echo "✅ contract ready: $OUT ($(wc -c < "$OUT" | tr -d ' ') bytes)"