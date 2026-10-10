#!/bin/bash
cd ~/.openclaw/workspace/lisp-rlm

PASSED=0
FAILED=0

run_fn() {
    local method="$1"
    local args_hex="$2"
    local expected="$3"
    local label="$4"
    
    if [ -z "$args_hex" ]; then
        ARGS=""
    else
        ARGS=$(echo "$args_hex" | xxd -r -p | base64)
    fi
    
    raw=$(expect -c "
        set timeout 25
        log_user 1
        spawn near contract call-function as-transaction lending.kampy.testnet $method base64-args {$ARGS} prepaid-gas 300Tgas attached-deposit 0N sign-as kampy.testnet network-config testnet sign-with-keychain send
        sleep 2
        send \"\r\"
        expect {
            \"succeeded\" { expect eof }
            \"unsafe\" { expect eof }
            \"Error\" { expect eof }
            \"TIMEOUT\" { puts \"TIMEOUT\" }
        }
    " 2>&1)
    
    # Extract return value (number printed to stdout) or "unsafe" or error
    ret=$(echo "$raw" | grep -oE 'printed to stdout.*' | grep -oE '[0-9]+' | head -1 || true)
    unsafe=$(echo "$raw" | grep -c 'unsafe' || true)
    error=$(echo "$raw" | grep -c 'Error' || true)
    
    if [ -n "$expected" ]; then
        if [ "$expected" = "unsafe" ] && [ "$unsafe" -gt 0 ]; then
            echo "  PASS: $label"
            PASSED=$((PASSED+1))
        elif echo "$ret" | grep -q "$expected"; then
            echo "  PASS: $label -> $ret"
            PASSED=$((PASSED+1))
        else
            echo "  FAIL: $label -> got '$ret' (expected '$expected')"
            FAILED=$((FAILED+1))
        fi
    else
        echo "  OK:   $label -> $ret"
        PASSED=$((PASSED+1))
    fi
}

# Little-endian u32: 5000000 = 0x004C4B40 -> 404B4C00
# Little-endian u32: 10000000 = 0x00989680 -> 80969800
# Little-endian u32: 3000000 = 0x002DC6C0 -> C0C62D00
# Little-endian u32: 1000000 = 0x000F4240 -> 40420F00
# Little-endian u32: 2000000 = 0x001E8480 -> 80841E00
# Little-endian u32: 50000000 = 0x02FAF080 -> 80F0FA02

# register_asset args: 7 u32 LE: vol=80, cf=80, tu=80, r0=100, r1=500, r2=3000, rc=25
# 80=50, 500=0x01F4, 3000=0x0BB8, 25=0x19

echo "=== 1. Init ==="
run_fn init "" "" "init"

echo "=== 2. Register asset ==="
run_fn register_asset "50000000500000005000000064000000f4010000b80b000019000000" "" "register_asset"

echo "=== 3. Deposit 5M ==="
run_fn deposit "804f0100" "" "deposit 5M"

echo "=== 4. Pool state ==="
run_fn get_pool_s "" "" "pool/s"
run_fn get_pool_a "" "" "pool/a"
run_fn get_borrow_index "" "" "borrow_idx"
run_fn get_supply_index "" "" "supply_idx"

echo "=== 5. Collateral 10M ==="
run_fn inc_collat "80969800" "" "collateral 10M"

echo "=== 6. Borrow 3M ==="
run_fn borrow "c0c62d00" "" "borrow 3M"

echo "=== 7. Health & rates ==="
run_fn get_health "" "" "health"
run_fn get_borrow_rate "" "" "borrow_rate"
run_fn get_supply_rate "" "" "supply_rate"
run_fn get_reserve "" "" "reserve"

echo "=== 8. Repay 1M ==="
run_fn repay "40420f00" "" "repay 1M"

echo "=== 9. Deposit 5M more ==="
run_fn deposit "804f0100" "" "deposit 5M more"

echo "=== 10. Withdraw 2M ==="
run_fn withdraw "80841e00" "" "withdraw 2M"

echo "=== 11. Unsafe borrow 50M ==="
run_fn borrow "80f0fa02" "unsafe" "reject unsafe"

echo "=== 12. Dec collateral 3M ==="
run_fn dec_collat "c0c62d00" "" "dec_collat 3M"

echo "=== 13. Repay remaining 2M ==="
run_fn repay "80841e00" "" "repay remaining"

echo "=== 14. Final health ==="
run_fn get_health "" "" "final health"

echo ""
echo "=========================================="
echo "Results: $PASSED passed, $FAILED failed"
