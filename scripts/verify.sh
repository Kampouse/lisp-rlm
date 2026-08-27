#!/usr/bin/env bash
# verify.sh — run a deploy contract through the full 3-layer gauntlet.
#
#   scripts/verify.sh <deploy-dir> [scenario-file]
#
# Layers:
#   L1 near-mock   — fast semantic loop (calibrated gas, view traps)
#   L2 near-vm-run — production VMLogic (real fees, real contract validation)
#   L3 sandbox     — real node, receipts, whole-transaction gas
#
# Scenario file: one call per line, `method|args-json`, `#` comments.
# $HOLDER = the calling account (predecessor): owner.test.near on L1/L2,
# the contract account on L3. Default: scripts/scenarios/<name>.txt.
# All three layers must return IDENTICAL results (📄 diff) or verify fails.

set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIR="${1:?usage: verify.sh <deploy-dir> [scenario-file]}"
DIR="$(cd "$DIR" && pwd)"
NAME="$(basename "$DIR")"
SCEN="${2:-$ROOT/scripts/scenarios/$NAME.txt}"
[ -f "$SCEN" ] || { echo "❌ no scenario: $SCEN"; exit 2; }

MOCK="$ROOT/target/debug/near-mock"
ORACLE="$ROOT/near-vm-run/target/debug/near-vm-run"
SMOKE="$ROOT/sandbox-tests/target/debug/smoke"
for b in "$MOCK" "$ORACLE" "$SMOKE"; do
  [ -x "$b" ] || { echo "❌ missing binary: $b (build it first)"; exit 2; }
done

echo "── build ──────────────────────────────────────"
"$ROOT/target/debug/near-compile" build "$DIR" || { echo "❌ compile failed"; exit 1; }
WASM="$DIR/target/$NAME.wasm"
[ -f "$WASM" ] || { echo "❌ no wasm at $WASM"; exit 1; }
echo "✅ $WASM ($(wc -c < "$WASM" | tr -d ' ') bytes)"

# scenario → calls. $HOLDER handled per-layer.
LINES=()
while IFS= read -r l; do LINES+=("$l"); done < <(grep -vE '^\s*(#|$)' "$SCEN")
[ ${#LINES[@]} -gt 0 ] || { echo "❌ empty scenario"; exit 2; }

run_l1l2 () {  # $1=binary $2=holder
  local bin="$1" holder="$2"
  "$bin" "$WASM" reset >/dev/null 2>&1
  for line in "${LINES[@]}"; do
    local m="${line%%|*}" a="${line#*|}"
    a="${a//\$HOLDER/$holder}"
    "$bin" "$WASM" "$m" "$a" 2>/dev/null \
      | grep -E "📄|❌|↳ caused" | head -1 | sed "s/^/$m → /"
  done
}

run_l3 () {
  local specs=()
  for line in "${LINES[@]}"; do
    local m="${line%%|*}" a="${line#*|}"
    specs+=("{\"method\":\"$m\",\"args\":$a}")
  done
  "$SMOKE" "$WASM" "${specs[@]}" 2>/dev/null | grep -E "→ 📄|→ ❌"
}

run_l1l2 "$MOCK" "owner.test.near" > /tmp/verify-r1.txt
run_l1l2 "$ORACLE" "owner.test.near" > /tmp/verify-r2.txt
run_l3 > /tmp/verify-r3.txt
echo "── L1 near-mock ───────────────────────────────"; cat /tmp/verify-r1.txt
echo "── L2 near-vm-run (production VMLogic) ────────"; cat /tmp/verify-r2.txt
echo "── L3 sandbox (real node) ─────────────────────"; cat /tmp/verify-r3.txt

echo "── diff L1↔L2 ↔L3 (results must match) ────────"
rc=0
n=${#LINES[@]}
for ((i=0; i<n; i++)); do
  m="${LINES[$i]%%|*}"
  ra=$(sed -n "$((i+1))p" /tmp/verify-r1.txt | sed 's/^.* → //')
  rb=$(sed -n "$((i+1))p" /tmp/verify-r2.txt | sed 's/^.* → //')
  r3=$(sed -n "$((i+1))p" /tmp/verify-r3.txt | sed 's/^.* → //')
  r3="${r3%%  \[*}"   # strip L3 gas suffix
  if [ "$ra" != "$rb" ] || [ "$ra" != "$r3" ]; then
    echo "❌ $m: L1='$ra'  L2='$rb'  L3='$r3'"
    rc=1
  fi
done
if [ $rc -eq 0 ]; then
  echo "✅ all $n calls agree across mock / oracle / sandbox"
  echo "🎉 $NAME VERIFIED — deployable"
fi
exit $rc
