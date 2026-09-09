#!/usr/bin/env bash
# Functional verification of the near-mock improvement batches.
# Expectations match ACTUAL product output.
#
# Portable (2026-09-08): paths derive from the script's own location — no
# machine-specific absolutes; binaries fall back release → debug so a plain
# `cargo build --bin near-mock --bin near-compile` is enough; the guestbook
# fixture is vendored in ../fixtures (override with GUESTBOOK_WASM=...);
# the workdir respects $TMPDIR (sandboxed macOS / some CI deny /tmp).
set -u
DIR="$(cd "$(dirname "$0")" && pwd)"   # .../scripts
ROOT="$(cd "$DIR/.." && pwd)"          # repo root

bin_pick() { # prefer release, fall back to debug
  if [ -x "$ROOT/target/release/$1" ]; then echo "$ROOT/target/release/$1"
  elif [ -x "$ROOT/target/debug/$1" ]; then echo "$ROOT/target/debug/$1"
  else echo ""; fi
}
NM=$(bin_pick near-mock)
NC=$(bin_pick near-compile)
[ -n "$NM" ] || { echo "FAIL: near-mock binary not built — cargo build --bin near-mock"; exit 1; }
[ -n "$NC" ] || { echo "FAIL: near-compile binary not built — cargo build --bin near-compile"; exit 1; }

# near-sdk guestbook contract — the same fixture the standalone near-mock
# crate ships; the expectations below (non-payable sign, JSON-arg panics,
# unicode state) need a real near-sdk binary, which the lisp probes can't
# replicate. Override with GUESTBOOK_WASM=/path/to/guestbook.wasm.
WASM="${GUESTBOOK_WASM:-$ROOT/fixtures/guestbook.wasm}"
[ -s "$WASM" ] || { echo "FAIL: guestbook fixture missing: $WASM"; exit 1; }

WORK="${TMPDIR:-/tmp}/nmverify.$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

pass=0; fail=0
ok()   { pass=$((pass+1)); echo "PASS: $1"; }
bad()  { fail=$((fail+1)); echo "FAIL: $1"; }
check() { if printf '%s' "$3" | grep -q "$2"; then ok "$1"; else bad "$1 | wanted '$2' got: $(printf '%s' "$3" | tr '\n' '|' | head -c 200)"; fi }

cat > "$WORK/ts.lisp" <<'EOF'
(define (main) (near/block_timestamp))
EOF
cat > "$WORK/rs.lisp" <<'EOF'
(define (main) (near/random_seed))
EOF
cat > "$WORK/ev.lisp" <<'EOF'
(define (main)
  (near/log "EVENT_JSON:{\"standard\":\"nep171\",\"version\":\"1.0.0\",\"event\":\"nft_mint\",\"data\":[{\"owner_id\":\"jean.near\"}]}")
  0)
EOF
"$NC" "$WORK/ts.lisp" "$WORK/ts.wasm" >/dev/null 2>&1
"$NC" "$WORK/rs.lisp" "$WORK/rs.wasm" >/dev/null 2>&1
"$NC" "$WORK/ev.lisp" "$WORK/ev.wasm" >/dev/null 2>&1
[ -s "$WORK/ts.wasm" ] && ok "lisp probes compile" || { bad "lisp probe compile"; exit 1; }

echo "== help =="
$NM --help >/dev/null 2>&1 && ok "--help exit 0" || bad "--help exit"

echo "== method-not-found lists exports =="
out=$($NM "$WASM" nosuchmethod '{}' 2>&1)
check "hint header" "Available exports" "$out"
check "lists get_signatures" "get_signatures" "$out"

echo "== --json =="
out=$($NM "$WASM" get_signature_count '{}' --json --state "$WORK/s1.json" 2>/dev/null)
check "outcome ok" '"ok"' "$out"
check "gas" 'gas_burnt_tgas' "$out"
check "storage diff" '"added"' "$out"

echo "== --dry-run does not persist =="
S="$WORK/dry.json"
out=$($NM "$WASM" sign '{"message":"dry probe"}' --dry-run --state "$S" 2>&1)
check "dry-run note" "NOT persisted" "$out"
count=$($NM "$WASM" get_signature_count '{}' --state "$S" 2>/dev/null)
check "count still 0" '📄 0' "$count"

echo "== --now / --advance =="
out=$($NM "$WORK/ts.wasm" _run '{}' --now 1700000000 --advance 60 --state "$WORK/ts.bin" 2>/dev/null)
check "ts = (1700000000+60)e9" "1700000060000000000" "$out"
out=$($NM "$WORK/ts.wasm" _run '{}' --now 1234567890 --state "$WORK/ts2.bin" 2>/dev/null)
check "bare --now" "1234567890000000000" "$out"

echo "== random_seed determinism =="
seedof() { $NM "$WORK/rs.wasm" _run '{}' --now "$1" --state "$2" 2>/dev/null | grep -oE '[0-9a-f]{64}'; }
a=$(seedof 1000 "$WORK/r1.bin"); rm -f "$WORK/r1.bin"
b=$(seedof 1000 "$WORK/r2.bin"); rm -f "$WORK/r2.bin"
c=$(seedof 2000 "$WORK/r3.bin"); rm -f "$WORK/r3.bin"
{ [ -n "$a" ] && [ "$a" = "$b" ]; } && ok "same --now ⇒ same seed" || bad "seed stability: [$a] vs [$b]"
{ [ -n "$c" ] && [ "$a" != "$c" ]; } && ok "diff --now ⇒ diff seed" || bad "seed variation: [$a] vs [$c]"
d=$($NM "$WORK/rs.wasm" _run '{}' --state "$WORK/r4.bin" 2>/dev/null | grep -o 'cafe01' | head -1)
NEAR_MOCK_SEED=cafe01 $NM "$WORK/rs.wasm" _run '{}' --state "$WORK/r4.bin" 2>/dev/null | grep -q cafe01 && ok "NEAR_MOCK_SEED pin" || bad "NEAR_MOCK_SEED pin"

echo "== staking =="
S="$WORK/stake.json"
# sign without deposit (guestbook's sign is non-payable → deposit traps, correct
# NEAR semantics) + --staking: first STATE write must lock 1e20 yocto/byte.
out=$($NM "$WASM" sign '{"message":"stake probe"}' --staking --state "$S" 2>&1)
check "staking lock note" "staking: locked" "$out"
out=$($NM "$WASM" get_signature_count '{}' --state "$S" --json 2>/dev/null)
check "locked_yocto in json" 'locked_yocto' "$out"

echo "== gas schedule =="
$NM --gas-schedule-help > "$WORK/help2.txt" 2>&1
cat > "$WORK/sched.json" <<'EOF'
{"log_base": 13181732, "log_byte": 19335348, "read_register_base": 24108449, "read_register_byte": 3574166, "storage_has_key_base": 56356995, "storage_has_key_key_byte": 81569, "storage_read_base": 56356995, "storage_read_key_byte": 81569, "storage_read_value_byte": 3574166, "storage_remove_base": 64000000, "storage_remove_key_byte": 90563, "storage_write_base": 64000000, "storage_write_key_byte": 90563, "storage_write_value_byte": 3548576, "trie_node": 2280000000, "trie_walk_nodes": 16, "value_return_base": 4141250, "value_return_byte": 3574166}
EOF
out=$($NM "$WASM" sign '{"message":"sched probe"}' --json --gas-schedule "$WORK/sched.json" --state "$WORK/sch.json" 2>&1)
check "--gas-schedule flag" '"gas_burnt_tgas"' "$out"
echo '{"storage_write_base": "not-a-number"}' > "$WORK/bad.json"
$NM "$WASM" get_signature_count '{}' --gas-schedule "$WORK/bad.json" >/dev/null 2>&1
[ $? -ne 0 ] && ok "invalid schedule rejected (exit 1)" || bad "invalid schedule accepted"

echo "== NEP-297 EVENT_JSON =="
out=$($NM "$WORK/ev.wasm" _run '{}' --state "$WORK/ev1.bin" 2>/dev/null)
check "event banner" "📣 EVENT nep171" "$out"
out=$($NM "$WORK/ev.wasm" _run '{}' --json --state "$WORK/ev2.bin" 2>/dev/null | grep '^JSON')
check "events[] in --json" 'nep171' "$out"

echo "== catch_unwind printer safety =="
out=$($NM "$WASM" sign '{"message":"héllo wörld ünïcode ☕ exile"}' --state "$WORK/u.json" 2>&1); rc=$?
check "unicode sign ok" "Success" "$out"
[ $rc -eq 0 ] && ok "exit 0 with unicode state" || bad "exit $rc with unicode state"

echo "== str= / str!= (wasm emit; broke silently pre-2026-09-06) =="
cat > "$WORK/eq.lisp" <<'EOF'
(define (eq_yes) (if (str= "ab" "ab") (near/return_str "EQ") (near/return_str "NE")))
(define (eq_no)  (if (str= "abc" "abd") (near/return_str "EQ") (near/return_str "NE")))
(define (ne_yes) (if (str!= "a" "b") (near/return_str "DIFF") (near/return_str "SAME")))
(define (ne_no)  (if (str!= "a" "a") (near/return_str "DIFF") (near/return_str "SAME")))
(export "eq_yes" eq_yes)
(export "eq_no" eq_no)
(export "ne_yes" ne_yes)
(export "ne_no" ne_no)
EOF
"$NC" "$WORK/eq.lisp" "$WORK/eq.wasm" >/dev/null 2>&1
for c in "eq_yes:EQ" "eq_no:NE" "ne_yes:DIFF" "ne_no:SAME"; do
  m=${c%%:*}; want=${c##*:}
  out=$($NM "$WORK/eq.wasm" "$m" '{}' --state "$WORK/$m.bin" 2>/dev/null)
  check "wasm $m → $want" "📄 $want" "$out"
done

echo "== schnorr BIP-340 stitch (regression: call site vanished silently pre-e2aa618; lib re-sourced from schnorr/ 2026-09-08) =="
# compile the official vector-0 probe + a tampered variant (last sig nibble)
cp "$ROOT/schnorr/test_bip340.lisp" "$WORK/bip340.lisp"
sed 's/536C0/536C1/' "$ROOT/schnorr/test_bip340.lisp" > "$WORK/bip340_bad.lisp"
"$NC" "$WORK/bip340.lisp" "$WORK/bip340.wasm" >/dev/null 2>&1
"$NC" "$WORK/bip340_bad.lisp" "$WORK/bip340_bad.wasm" >/dev/null 2>&1
out=$($NM "$WORK/bip340.wasm" run --view --state "$WORK/bip.bin" 2>/dev/null)
check "BIP-340 vector 0 verifies" "📄 1" "$out"
out=$($NM "$WORK/bip340_bad.wasm" run --view --state "$WORK/bip2.bin" 2>/dev/null)
check "tampered signature rejected" "📄 0" "$out"
# the stitched contract must be self-contained: no residual env imports
imp=$($NM "$WORK/bip340.wasm" imports 2>/dev/null | strings 2>/dev/null | grep -c "schnorr_verify_bip340\|sha256_hash")
[ "${imp:-0}" = "0" ] && ok "no residual crypto imports (stitched in-emitter)" || bad "crypto imports left unresolved: $imp"

echo "== scenario: per-step as/predecessor + now/advance =="
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
"$NC" "$WORK/scen.lisp" "$WORK/contract.wasm" >/dev/null 2>&1
cat > "$WORK/scen.json" <<'EOF'
{"name": "verify: identity + time travel",
 "steps": [
   {"method": "whoami", "expect": "owner.test.near"},
   {"method": "whoami", "as": "alice.test.near", "expect": "alice.test.near"},
   {"method": "whoami", "as": "bob.test.near", "predecessor": "vault.test.near", "expect": "vault.test.near"},
   {"method": "clock", "now": 1700000000, "expect": "1700000000000000000"},
   {"method": "clock", "advance": 60, "expect": "1700000060000000000"},
   {"method": "clock", "expect": "1700000060000000000"},
   {"method": "gate", "as": "alice.test.near", "expect": "allowed"},
   {"method": "gate", "as": "eve.test.near", "expect": "trap"}
 ]}
EOF
out=$(cd "$WORK" && $NM scenario scen.json 2>&1); rc=$?
check "scenario runner banner" "scenario: verify: identity + time travel" "$out"
check "per-step as honored" "👤 as alice" "$out"
check "per-step predecessor honored" "predecessor vault" "$out"
check "step now" "1700000000000000000" "$out"
check "advance accumulates + persists" "1700000060000000000" "$out"
check "caller gate allowed (str= works)" "allowed" "$out"
check "caller gate traps for eve" "trap as expected" "$out"
[ $rc -eq 0 ] && ok "scenario exit 0 (9/9)" || bad "scenario exit $rc"
[ -f "$WORK/state.bin" ] && ok "scenario persisted state.bin" || bad "scenario state.bin missing"

# ---- --trace: host-call timeline + per-host gas attribution ----
tout=$(cd "$WORK" && NEAR_MOCK_TRACE=1 $NM scenario scen.json 2>&1)
check "trace timeline entries" "] predecessor_account_id" "$tout"
check "trace per-call gas" "gas=" "$tout"
check "trace per-step summary" "🔍 host trace —" "$tout"
check "trace records predecessor_account_id" "predecessor_account_id" "$tout"
single=$(cd "$WORK" && $NM contract.wasm gate --trace 2>&1)
check "single-call trace summary" "🔍 host trace —" "$single"
check "single-call trace shows storage_write cost" "storage_write" "$single"
jtrace=$(cd "$WORK" && $NM contract.wasm gate --trace --json 2>&1 | grep '^JSON ' | tail -1)
check "json host_trace field" '"host_trace"' "$jtrace"
check "json host_trace totals carry calls+gas" '"calls"' "$jtrace"

echo "== stub-kill surface (2026-09-08) =="
# PV155 crypto/validator gas pins in the default schedule
out=$($NM --gas-schedule-help)
check "ecrecover_base pin" "ecrecover_base" "$out"
check "p256_verify_base pin" "p256_verify_base" "$out"
check "alt_bn128_pairing_check_base pin" "alt_bn128_pairing_check_base" "$out"
check "validator_stake_base pin" "validator_stake_base" "$out"
# snapshot subcommand surface (hermetic: usage/exit codes only — live pull
# needs network; verified separately against pyth-oracle.near)
out=$($NM snapshot 2>&1); rc=$?
[ $rc -ne 0 ] && ok "snapshot without args exits nonzero" || bad "snapshot no-arg exit 0"
check "snapshot usage line" "usage: near-mock snapshot" "$out"
out=$($NM snapshot a b --rpc 2>&1); [ $? -ne 0 ] && ok "malformed snapshot exits nonzero" || bad "malformed snapshot exit 0"
out=$($NM --help 2>&1)
check "help lists snapshot" "near-mock snapshot" "$out"
out=$($NM state 2>&1)
check "state import usage" "state import" "$out"

echo "== 2026-09-08 sync: --version / storage namespace / trace error counts =="
out=$($NM --version 2>&1); rc=$?
check "--version prints version" "near-mock 0\." "$out"
[ $rc -eq 0 ] && ok "--version exit 0" || bad "--version exit $rc"
# single-call state must land under the DOCUMENTED default account, not the
# '' partition (crate e130226): dump attribution + prefix filter + interop
$NM "$WASM" sign '{"message":"ns probe"}' --state "$WORK/ns.bin" >/dev/null 2>&1
out=$($NM state dump "$WORK/ns.bin" 2>/dev/null)
check "single-call state under escrow.test.near" 'account": *"escrow.test.near"' "$out"
out=$($NM state dump "$WORK/ns.bin" escrow.test.near 2>/dev/null | jq -e 'length == 1' >/dev/null 2>&1 && echo FILTEROK)
check "dump prefix filter finds default account" "FILTEROK" "$out"
# trace error counts (crate 61aa006): a JSON-arg panic must show up as an
# erroring panic_utf8 in the summary AND in --json host_trace
out=$($NM "$WASM" sign '{}' --trace --state "$WORK/trap.bin" 2>&1)
check "trace summary counts host errors" "❌1err" "$out"
out=$($NM "$WASM" sign '{}' --trace --json --state "$WORK/trap2.bin" 2>/dev/null)
check "json host_trace carries errors" '"errors":1' "$out"

echo
echo "RESULT: $pass passed, $fail failed"
[ $fail -eq 0 ]
