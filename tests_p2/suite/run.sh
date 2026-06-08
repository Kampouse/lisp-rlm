#!/bin/bash
COMPILER="${COMPILER:-$HOME/.openclaw/workspace/lisp-rlm/target/release/near-compile}"
RUNNER="${RUNNER:-$HOME/.inlayer/bin/inlayer}"
DIR="$(cd "$(dirname "$0")" && pwd)"
RPC_MAINNET="--rpc https://rpc.mainnet.fastnear.com"
PASS=0
FAIL=0

run_test() {
    local name="$1"
    local expect="${2:-}"
    local rpc_flag="${3:-}"
    local src="$DIR/$name.lisp"
    local wasm="/tmp/suite_${name}.wasm"

    # Compile
    local compile_out
    compile_out=$($COMPILER "$src" --target=outlayer-p2 -o "$wasm" 2>&1)
    if [ $? -ne 0 ]; then
        echo "❌ $name — COMPILE FAIL"
        FAIL=$((FAIL+1)); return
    fi

    # Run
    local output
    output=$($RUNNER run "$wasm" '{}' $rpc_flag 2>&1) || true

    # Extract payload
    local payload
    payload=$(echo "$output" | grep '📤 Output:' | sed 's/^.*📤 Output: //' || true)
    local payload_len=${#payload}

    # Check success
    if echo "$output" | grep -q '✅ Success: true'; then
        if [ -n "$expect" ]; then
            if echo "$payload" | grep -q "$expect"; then
                echo "✅ $name — ${payload_len}B (match: $expect)"
            else
                echo "❌ $name — expected '$expect' in output"
                echo "   got: ${payload:0:200}"
                FAIL=$((FAIL+1)); return
            fi
        else
            echo "✅ $name — ${payload_len}B"
        fi
        PASS=$((PASS+1))
    else
        echo "❌ $name — RUNTIME FAIL"
        echo "$output" | grep -iE 'fault|error|OOB|out of bounds' | head -3 || true
        FAIL=$((FAIL+1))
    fi
}

echo "═══ lisp-rlm P2 regression suite ═══"

echo ""
echo "── 1. HTTP GET ──"
run_test "01_http_get_raw" '"price"'
run_test "02_rpc_status" '"version"'
run_test "03_rpc_post" '"code"'

echo ""
echo "── 2. JSON extraction ──"
run_test "04_json_single_key" "615"
run_test "05_json_multi_key" "381"
run_test "06_json_nested" '"nbtc"'

echo ""
echo "── 3. String ops ──"
run_test "07_strcat" "hello world"
run_test "08_strcat_json" '"nbtc"'

echo ""
echo "── 4. Control flow ──"
run_test "09_let_star" "42"
run_test "10_cond" "yes"
run_test "11_arithmetic" "52"

echo ""
echo "── 5. OutLayer RPC view ──"
run_test "12_burrow_positions" "kampouse.near" "$RPC_MAINNET"
run_test "13_burrow_decode" "has_supplied" "$RPC_MAINNET"
run_test "14_rpc_view" "usdt_price" "$RPC_MAINNET"

echo ""
echo "═══ Results: $PASS pass, $FAIL fail ═══"
[ "$FAIL" -eq 0 ]
