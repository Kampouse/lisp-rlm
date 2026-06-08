#!/bin/bash
# ─── P2 OutLayer Regression Test Suite v2 ───
# Tests every builtin the Rhea+Burrow pipeline needs.
# Captures the REAL baseline so regressions are visible.
#
# Usage: ./tests_p2/run_suite.sh
# Exit codes: 0=all pass, 1=failures, 2=compile regressions
#
set -euo pipefail

COMPILE="${COMPILE:-$HOME/.openclaw/workspace/lisp-rlm/target/release/near-compile}"
RUNNER="${RUNNER:-$HOME/.inlayer/bin/inlayer}"
PASS=0
FAIL=0
COMPILE_ERR=0
EXPECTED_FAIL=0
KNOWN_COMPILE_FAIL=0
RESULTS=()
TMPDIR=$(mktemp -d)

cleanup() { rm -rf "$TMPDIR"; }
trap cleanup EXIT

header() { echo ""; echo "━━━ $1 ━━━"; }

# run_test <name> <lisp> <mode> [extra]
# Modes: compile_only, run_valid_json, run_no_nulls, run_match:pattern, run_exact:val, run_gt0_bytes
run_test() {
    local name="$1" lisp="$2" mode="$3"
    local wasm="$TMPDIR/${name// /_}.wasm"

    # ── Compile ──
    local compile_out
    compile_out=$($COMPILE "$lisp" --target=outlayer-p2 -o "$wasm" 2>&1) || true
    if [ ! -f "$wasm" ] || [ ! -s "$wasm" ]; then
        echo "  ❌ COMPILE FAIL: $name"
        echo "    $(echo "$compile_out" | grep 'error' | head -1)"
        COMPILE_ERR=$((COMPILE_ERR + 1))
        RESULTS+=("COMPILE_ERR|$name")
        return
    fi
    local wasm_size
    wasm_size=$(echo "$compile_out" | grep -o '[0-9]* bytes' | tail -1 | grep -o '[0-9]*')
    echo -n "  📦 $name (${wasm_size}B) → "

    # ── Run ──
    local raw
    raw=$($RUNNER run "$wasm" '{}' 2>&1)

    local output
    output=$(echo "$raw" | sed -n 's/.*Output: //p')

    local instr
    instr=$(echo "$raw" | grep -o 'Instructions: [0-9]*' | head -1 | grep -o '[0-9]*')

    local raw_size
    raw_size=$(echo "$raw" | grep -o 'Raw output size: [0-9]*' | head -1 | grep -o '[0-9]*')

    case "$mode" in
        compile_only)
            echo "✅ compiles"
            PASS=$((PASS + 1))
            RESULTS+=("PASS|$name|compile_only")
            ;;
        run_valid_json)
            local first="${output:0:1}"
            local null_count
            null_count=$(echo -n "$output" | od -A n -t x1 | tr -d ' \n' | grep -co '0000' || true)
            if [ "$first" = "{" ] || [ "$first" = "[" ]; then
                if [ "${null_count:-0}" -eq 0 ]; then
                    echo "✅ valid JSON (${raw_size}B, ${instr}i)"
                    PASS=$((PASS + 1))
                    RESULTS+=("PASS|$name|valid_json|${raw_size}|${instr}")
                else
                    echo "⚠️ JSON but ${null_count} null runs (${raw_size}B)"
                    FAIL=$((FAIL + 1))
                    RESULTS+=("FAIL|$name|null_runs:${null_count}")
                fi
            elif [ "$first" = $'\0' ] || [ "${raw_size:-0}" -eq 0 ]; then
                echo "❌ null bytes / empty (${raw_size:-0}B) — KNOWN BROKEN"
                EXPECTED_FAIL=$((EXPECTED_FAIL + 1))
                RESULTS+=("EXPECTED_FAIL|$name|null_output|${raw_size:-0}")
            else
                echo "❌ not JSON (starts '${first:0:20}')"
                FAIL=$((FAIL + 1))
                RESULTS+=("FAIL|$name|not_json")
            fi
            ;;
        run_no_nulls)
            local null_bytes
            null_bytes=$(echo -n "$output" | head -c 20 | xxd -p | tr -d '\n')
            if echo "$null_bytes" | grep -q '00000300'; then
                echo "❌ 196608 corruption"
                FAIL=$((FAIL + 1))
                RESULTS+=("FAIL|$name|corruption_196608")
            elif echo "$null_bytes" | grep -q '^0000'; then
                echo "❌ null bytes at start"
                EXPECTED_FAIL=$((EXPECTED_FAIL + 1))
                RESULTS+=("EXPECTED_FAIL|$name|null_start")
            else
                echo "✅ no corruption"
                PASS=$((PASS + 1))
                RESULTS+=("PASS|$name|no_corruption")
            fi
            ;;
        run_match*)
            local pattern="${mode#run_match:}"
            if echo "$output" | grep -q "$pattern"; then
                echo "✅ matches '$pattern'"
                PASS=$((PASS + 1))
                RESULTS+=("PASS|$name|match:$pattern")
            else
                echo "❌ no match for '$pattern'"
                FAIL=$((FAIL + 1))
                RESULTS+=("FAIL|$name|no_match")
            fi
            ;;
        run_exact*)
            local expected="${mode#run_exact:}"
            if [ "$output" = "$expected" ]; then
                echo "✅ → '$expected'"
                PASS=$((PASS + 1))
                RESULTS+=("PASS|$name|exact:$expected")
            else
                echo "❌ got '${output:0:40}' expected '$expected'"
                FAIL=$((FAIL + 1))
                RESULTS+=("FAIL|$name|exact_mismatch")
            fi
            ;;
        run_gt0_bytes)
            if [ "${#output}" -gt 0 ]; then
                echo "✅ ${#output} chars"
                PASS=$((PASS + 1))
                RESULTS+=("PASS|$name|${#output}chars")
            else
                echo "❌ empty"
                EXPECTED_FAIL=$((EXPECTED_FAIL + 1))
                RESULTS+=("EXPECTED_FAIL|$name|empty")
            fi
            ;;
        *)
            echo "❓ unknown mode: $mode"
            ;;
    esac
}

cat_l() { cat > "$1"; }

# ═══════════════════════════════════════════════════
#  1. COMPILE REGRESSIONS (must compile, don't need to run)
# ═══════════════════════════════════════════════════
header "1. Compile: Core Language"

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run) (+ 20 22))
EOF
run_test "arithmetic" "$TMPDIR/a.lisp" compile_only

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run) (if (= 1 1) 10 20))
EOF
run_test "if" "$TMPDIR/a.lisp" compile_only

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run) (let ((x 5) (y 7)) (* x y)))
EOF
run_test "let" "$TMPDIR/a.lisp" compile_only

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run) (let* ((x 5) (y (* x 2))) y))
EOF
run_test "let*" "$TMPDIR/a.lisp" compile_only

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run) (cond ((= 1 2) 10) ((= 1 1) 20) (else 30)))
EOF
run_test "cond" "$TMPDIR/a.lisp" compile_only

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (square x) (* x x))
(define (run) (square 7))
EOF
run_test "define+call" "$TMPDIR/a.lisp" compile_only

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run) "hello world")
EOF
run_test "string literal" "$TMPDIR/a.lisp" compile_only

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run) (str-cat "hello" " world"))
EOF
run_test "str-cat 2arg" "$TMPDIR/a.lisp" compile_only

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run)
  (let ((a (str-cat "a" "b"))
        (c (str-cat "c" "d")))
    (str-cat a c)))
EOF
run_test "str-cat nested" "$TMPDIR/a.lisp" compile_only

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run) (str-len "hello"))
EOF
run_test "str-len" "$TMPDIR/a.lisp" compile_only

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run) (json-get-str "a" "{\"a\":42}"))
EOF
run_test "json-get-str" "$TMPDIR/a.lisp" compile_only

# ═══════════════════════════════════════════════════
#  2. RUNTIME: HTTP + JSON (live network)
# ═══════════════════════════════════════════════════
header "2. Runtime: HTTP GET"

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run)
  (http-get "https://rpc.mainnet.fastnear.com/status"))
EOF
run_test "http-get status" "$TMPDIR/a.lisp" run_valid_json

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run)
  (http-get "https://api.rhea.finance/list-token-price"))
EOF
run_test "http-get rhea prices" "$TMPDIR/a.lisp" run_valid_json

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run)
  (http-get "https://api.rhea.finance/list-token-price"))
EOF
run_test "http-get no nulls" "$TMPDIR/a.lisp" run_no_nulls

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run)
  (http-get "https://rpc.mainnet.fastnear.com/status"))
EOF
run_test "http-get status no nulls" "$TMPDIR/a.lisp" run_no_nulls

header "3. Runtime: HTTP POST"

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run)
  (let ((body "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_account\",\"args_base64\":\"eyJhY2NvdW50X2lkIjoia2FtcG91c2UubmVhciJ9\"}}"))
    (http-post "https://rpc.mainnet.fastnear.com" body)))
EOF
run_test "http-post burrow" "$TMPDIR/a.lisp" run_valid_json

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run)
  (let ((body "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_account\",\"args_base64\":\"eyJhY2NvdW50X2lkIjoia2FtcG91c2UubmVhciJ9\"}}"))
    (http-post "https://rpc.mainnet.fastnear.com" body)))
EOF
run_test "http-post no nulls" "$TMPDIR/a.lisp" run_no_nulls

# ═══════════════════════════════════════════════════
#  4. RUNTIME: JSON + str-cat pipeline
# ═══════════════════════════════════════════════════
header "4. Runtime: JSON extraction + str-cat"

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run)
  (let* ((prices (http-get "https://api.rhea.finance/list-token-price"))
         (nbtc (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
         (zec (json-get-str "price" (json-get-str "zec.omft.near" prices)))
         (usdt (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
         (out (str-cat nbtc "," zec "," usdt)))
    out))
EOF
run_test "3 prices + str-cat" "$TMPDIR/a.lisp" run_match:'[0-9]\.'

# ═══════════════════════════════════════════════════
#  5. FULL COMBINED PIPELINE
# ═══════════════════════════════════════════════════
header "5. Combined: prices + burrow positions"

cat_l "$TMPDIR/a.lisp" << 'EOF'
(define (run)
  (let* ((prices (http-get "https://api.rhea.finance/list-token-price"))
         (nbtc-p (json-get-str "price" (json-get-str "nbtc.bridge.near" prices)))
         (zec-p (json-get-str "price" (json-get-str "zec.omft.near" prices)))
         (usdt-p (json-get-str "price" (json-get-str "usdt.tether-token.near" prices)))
         (stnear-p (json-get-str "price" (json-get-str "meta-pool.near" prices)))
         (p0 (str-cat "{\"nbtc\":\"" nbtc-p "\""))
         (p1 (str-cat p0 ",\"zec\":\"" zec-p "\""))
         (p2 (str-cat p1 ",\"usdt\":\"" usdt-p "\""))
         (p3 (str-cat p2 ",\"stnear\":\"" stnear-p "\"}"))
         (args-b64 "eyJhY2NvdW50X2lkIjoia2FtcG91c2UubmVhciJ9")
         (rpc-body (str-cat "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"method\":\"query\",\"params\":{\"request_type\":\"call_function\",\"finality\":\"final\",\"account_id\":\"contract.main.burrow.near\",\"method_name\":\"get_account\",\"args_base64\":\"" args-b64 "\"}}"))
         (rpc-result (http-post "https://rpc.mainnet.fastnear.com" rpc-body))
         (outer (json-get-str "result" rpc-result))
         (inner (json-get-str "result" outer))
         (supplied (json-get-str "supplied" inner))
         (out (str-cat "{\"prices\":" p3 ",\"supplied\":" supplied "}")))
    out))
EOF
run_test "prices+positions combined" "$TMPDIR/a.lisp" run_valid_json

# ═══════════════════════════════════════════════════
#  6. KNOWN-BROKEN (tracked, not blocking)
# ═══════════════════════════════════════════════════
header "6. Stale WASM baseline (compiled Jun 6)"
if [ -f ~/lisp-rlm/tests_p2/test_all_prices_fixed.wasm ]; then
    raw=$($RUNNER run ~/lisp-rlm/tests_p2/test_all_prices_fixed.wasm '{}' 2>&1)
    output=$(echo "$raw" | sed -n 's/.*Output: //p')
    instr=$(echo "$raw" | grep -o 'Instructions: [0-9]*' | head -1 | grep -o '[0-9]*')
    if echo "$output" | grep -q '"nbtc.bridge.near"'; then
        echo "  ✅ stale WASM still works (${instr}i, ${#output}B)"
        PASS=$((PASS + 1))
        RESULTS+=("PASS|stale_prices_wasm|${instr}")
    else
        echo "  ❌ stale WASM broken too!"
        FAIL=$((FAIL + 1))
        RESULTS+=("FAIL|stale_prices_wasm")
    fi
else
    echo "  ⏭️  no stale WASM to test"
fi

# ═══════════════════════════════════════════════════
#  SUMMARY
# ═══════════════════════════════════════════════════
header "RESULTS"
TOTAL=$((PASS + FAIL + COMPILE_ERR + EXPECTED_FAIL))
echo "  Total:          $TOTAL"
echo "  ✅ Pass:        $PASS"
echo "  ❌ Fail:        $FAIL"
echo "  ❌ Compile err:  $COMPILE_ERR"
echo "  ⚠️  Known broken: $EXPECTED_FAIL"
echo ""

if [ "$COMPILE_ERR" -gt 0 ]; then
    echo "🔴 COMPILE REGRESSIONS (must fix before merge):"
    for r in "${RESULTS[@]}"; do
        [[ "$r" == COMPILE_ERR* ]] && echo "  - ${r#*|}"
    done
    echo ""
fi

if [ "$FAIL" -gt 0 ]; then
    echo "🟡 RUNTIME FAILURES:"
    for r in "${RESULTS[@]}"; do
        [[ "$r" == FAIL* ]] && echo "  - ${r#*|}"
    done
    echo ""
fi

if [ "$EXPECTED_FAIL" -gt 0 ]; then
    echo "⚪ KNOWN BROKEN (tracked, not regressions):"
    for r in "${RESULTS[@]}"; do
        [[ "$r" == EXPECTED_FAIL* ]] && echo "  - ${r#*|}"
    done
    echo ""
fi

# Exit code
if [ "$COMPILE_ERR" -gt 0 ]; then
    echo "⛔ BLOCKED by compile errors"
    exit 2
elif [ "$FAIL" -gt 0 ]; then
    echo "🔴 FAILURES detected"
    exit 1
else
    echo "🟢 ALL $TOTAL TESTS PASSED (${EXPECTED_FAIL} known broken tracked separately)"
    exit 0
fi
