#!/usr/bin/env bash
# near-compile verification: probe contracts → wasm → executed under near-mock.
# Hermetic: no network. Requires a near-mock binary (crate or lisp-rlm build).
set -u
DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$DIR/../../.." && pwd)"   # repo root (workspace target/ lives there)
NC="$ROOT/target/release/near-compile"
[ -x "$NC" ] || NC="$ROOT/target/debug/near-compile"
[ -x "$NC" ] || { echo "FAIL: near-compile not built — cargo build"; exit 1; }
NM="${NEAR_MOCK:-}"
if [ -z "$NM" ]; then
  for c in "$ROOT/../near-mock/target/release/near-mock" "$ROOT/../near-mock/target/debug/near-mock" "$HOME/.cargo/bin/near-mock"; do
    [ -x "$c" ] && NM="$c" && break
  done
fi
[ -n "$NM" ] || { echo "FAIL: no near-mock binary (set NEAR_MOCK=/path)"; exit 1; }

WORK="${TMPDIR:-/tmp}/ncverify.$$"
mkdir -p "$WORK"; trap 'rm -rf "$WORK"' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); echo "PASS: $1"; }
bad() { fail=$((fail+1)); echo "FAIL: $1"; }
check() { if printf '%s' "$3" | grep -q "$2"; then ok "$1"; else bad "$1 | wanted '$2' got: $(printf '%s' "$3" | tr '\n' '|' | head -c 200)"; fi }

echo "== compile probes =="
cat > "$WORK/ts.lisp" <<'EOF'
(define (main) (near/block_timestamp))
EOF
cat > "$WORK/scen.lisp" <<'EOF'
(define (whoami) (near/return_str (near/predecessor_account_id)))
(define (clock) (near/return_str (near/block_timestamp)))
(define (gate)
  (begin
    (near/store-bytes "breach" (near/signer_account_id))
    (if (str= (near/signer_account_id) "alice.test.near")
        (near/return_str "allowed")
        (near/panic "forbidden"))))
(export "whoami" whoami)
(export "clock" clock)
(export "gate" gate)
EOF
"$NC" "$WORK/ts.lisp" "$WORK/ts.wasm" >/dev/null 2>&1
[ -s "$WORK/ts.wasm" ] && ok "ts.wasm compiles" || bad "ts.wasm compile"
"$NC" "$WORK/scen.lisp" "$WORK/scen.wasm" >/dev/null 2>&1
[ -s "$WORK/scen.wasm" ] && ok "scen.wasm compiles" || bad "scen.wasm compile"

echo "== execute under near-mock =="
out=$("$NM" "$WORK/ts.wasm" _run '{}' --now 1700000000 --state "$WORK/ts.bin" 2>/dev/null)
check "block_timestamp pinned" '📄 1700000000000000000' "$out"
out=$("$NM" "$WORK/scen.wasm" whoami --state "$WORK/s.bin" 2>/dev/null)
check "predecessor_account_id" '📄 owner.test.near' "$out"
out=$("$NM" "$WORK/scen.wasm" clock --now 1700000000 --state "$WORK/s.bin" 2>/dev/null)
check "clock in compiled contract" '📄 1700000000000000000' "$out"
out=$("$NM" "$WORK/scen.wasm" gate --state "$WORK/s.bin" 2>&1)
check "gate traps for default signer (forbidden)" 'forbidden' "$out"
out=$("$NM" "$WORK/scen.wasm" gate --state "$WORK/s2.bin" --view 2>&1)
check "gate via --view refuses storage write" 'ProhibitedInView' "$out"

echo "== TS frontend + stitched schnorr (hermetic, deterministic vector) =="
# TS-compiled contract verifying a BIP-340 signature through the stitched
# crypto lib — locks both the TS lowering and the schnorr stitcher. Vector:
# fixed key 0xAA*32, zeros aux (deterministic per RFC 6979-style derivation
# in the reference bip340 test lib).
cat > "$WORK/sigverify.ts" <<'EOF'
export function verify(pk: string, sig: string, msg: string): number {
  return schnorrVerify(hexDecode(pk), hexDecode(sig), hexDecode(sha256Hash(msg)));
}
EOF
"$NC" "$WORK/sigverify.ts" "$WORK/sigverify.wasm" >/dev/null 2>&1
[ -s "$WORK/sigverify.wasm" ] && ok "TS schnorr contract compiles" || bad "TS schnorr compile"
GOODSIG=52956857d31db0869d3ede6623ec62c5dbebf1bc6b6fc36d8ebacec566c4fbf9f00c1a9c48f6185676ee1181cd9a087ae37035d07fd835aac7b463ba2a93c574
BADSIG=52956857d31db0869d3ede6623ec62c5dbebf1bc6b6fc36d8ebacec566c4fbf9f00c1a9c48f6185676ee1181cd9a087ae37035d07fd835aac7b463ba2a93c570
PK=6a04ab98d9e4774ad806e302dddeb63bea16b5cb5f223ee77478e861bb583eb3
out=$("$NM" "$WORK/sigverify.wasm" verify "{\"pk\":\"$PK\",\"sig\":\"$GOODSIG\",\"msg\":\"near-mock hermetic schnorr vector\"}" --view --state "$WORK/sv.bin" 2>/dev/null)
check "valid BIP-340 sig verifies in TS contract" '📄 1' "$out"
out=$("$NM" "$WORK/sigverify.wasm" verify "{\"pk\":\"$PK\",\"sig\":\"$BADSIG\",\"msg\":\"near-mock hermetic schnorr vector\"}" --view --state "$WORK/sv2.bin" 2>&1)
check "tampered BIP-340 sig rejected" '📄 0' "$out"

echo
echo "RESULT: $pass passed, $fail failed"
[ $fail -eq 0 ]
