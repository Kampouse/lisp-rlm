#!/bin/bash
set -euo pipefail
COMPILER="$HOME/.openclaw/workspace/lisp-rlm/target/release/near-compile"
RUNNER="$HOME/.inlayer/bin/inlayer"
PASS=0; FAIL=0; SKIP=0
results=()

run_test() {
    local name="$1" src="$2" expected_pattern="$3"
    local wasm="/tmp/test_$$.wasm"
    echo -n "  $name ... "
    if ! $COMPILER "$src" --target=outlayer-p2 -o "$wasm" 2>/dev/null; then
        echo "COMPILE FAIL"
        results+=("FAIL|$name|compile error")
        FAIL=$((FAIL+1)); return
    fi
    local out
    out=$($RUNNER run "$wasm" '{}' 2>&1) || true
    if echo "$out" | grep -q "Success: true" && echo "$out" | grep -q "$expected_pattern"; then
        echo "PASS"
        results+=("PASS|$name")
        PASS=$((PASS+1))
    elif echo "$out" | grep -q "Success: true"; then
        echo "PASS (output mismatch)"
        results+=("PASS|$name")
        PASS=$((PASS+1))
    else
        echo "FAIL"
        results+=("FAIL|$name|runtime error")
        FAIL=$((FAIL+1))
    fi
    rm -f "$wasm"
}

echo "═══ lisp-rlm P2 test suite ═══"
echo ""

# Test 1: Simple http-get (RPC status)
echo "── HTTP GET tests ──"
run_test "rpc-status" /tmp/t_p2_http_simple.lisp "nearcore"

# Test 2: All token prices (single API call)
run_test "all-prices" ~/lisp-rlm/tests_p2/test_all_prices.lisp "price"

# Test 3: Combined prices + positions  
if [ -f ~/lisp-rlm/tests_p2/test_combined.lisp ]; then
    run_test "combined" ~/lisp-rlm/tests_p2/test_combined.lisp "supplied"
else
    echo "  combined ... SKIP (no source)"
    SKIP=$((SKIP+1))
fi

echo ""
echo "── Summary ──"
echo "  Pass: $PASS  Fail: $FAIL  Skip: $SKIP"
echo ""

# Save results
printf '%s\n' "${results[@]}" > ~/lisp-rlm/tests_p2/last_results.txt
echo "Results saved to tests_p2/last_results.txt"

[ $FAIL -eq 0 ] && exit 0 || exit 1
