#!/usr/bin/env python3
"""Fuzz test JSON parsing against our Lisp contract vs expected behavior.
Generates edge cases that serde_json handles correctly.
"""
import subprocess
import json
import sys

BINARY = "./target/debug/near-compile"
TEST_FILE = "tests/test_json.lisp"
ACCOUNT = "kampy.testnet"

def call_testnet(method, args_dict):
    """Call contract on testnet, return (logs, return_value)"""
    args_json = json.dumps(args_dict)
    cmd = f'near contract call-function as-read-only {ACCOUNT} {method} json-args \'{args_json}\' network-config testnet now'
    result = subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=15)
    output = result.stdout + result.stderr
    
    logs = []
    return_val = ""
    in_logs = False
    for line in output.split('\n'):
        if 'Logs:' in line:
            in_logs = True
            continue
        if in_logs:
            stripped = line.strip()
            if stripped and not stripped.startswith('[') and 'return' not in stripped.lower():
                logs.append(stripped)
            if 'return' in line.lower():
                in_logs = False
        if 'Function execution return value' in line:
            # Next line has the value
            pass
        if 'return value (printed' in line:
            # Next line
            pass
    
    # Parse return value from output
    if '{"result":' in output:
        # Try to extract JSON result
        idx = output.index('{"result":')
        end = output.index('}', idx) + 1
        return_val = output[idx:end]
    
    return logs, return_val

# Test cases: (description, args, expected_log, expected_return_contains)
test_cases = [
    # Basic
    ("Basic int", {"amount": 42}, "42", None),
    ("Zero", {"amount": 0}, "0", None),
    ("Negative", {"amount": -5}, "-5", None),
    ("Large positive", {"amount": 9999999999}, "9999999999", None),
    ("Large negative", {"amount": -9999999999}, "-9999999999", None),
    
    # Quoted numbers
    ("Quoted int", {"amount": "42"}, "42", None),
    ("Quoted zero", {"amount": "0"}, "0", None),
    ("Quoted negative", {"amount": "-5"}, "-5", None),
    
    # Key edge cases
    ("Missing key", {"other": 1}, "0", None),
    ("Empty object", {}, "0", None),
    ("Multiple keys, target first", {"amount": 42, "other": 1}, "42", None),
    ("Multiple keys, target last", {"x": 1, "amount": 42}, "42", None),
    ("Multiple keys, target middle", {"x": 1, "amount": 42, "y": 3}, "42", None),
    
    # Substring traps
    ("Substring prefix", {"subamount": 99, "x": 42}, "0", None),
    ("Substring suffix", {"x": 42, "amountfoo": 99}, "0", None),
    ("Exact key after substring", {"subamount": 99, "amount": 7}, "7", None),
    ("Exact key before substring", {"amount": 7, "subamount": 99}, "7", None),
    
    # Whitespace variations
    ("No space after colon", None, None, None),  # Can't easily test via near CLI
    ("Extra spaces", {"amount":  42}, "42", None),
    
    # Number edge cases
    ("Max i64 safe", {"amount": 9007199254740991}, "9007199254740991", None),
    ("Single digit", {"amount": 1}, "1", None),
    ("Leading zeros", {"amount": "007"}, "7", None),
    
    # String key edge cases
    ("Empty string key", {"": 42}, "0", None),
    ("Key with special chars", {"a-mount": 42}, "0", None),
    ("Key with underscore", {"amount": 42}, "42", None),
    
    # Value edge cases  
    ("Float value", {"amount": 42.5}, "42", None),  # Should parse 42, stop at .
    ("Boolean true", {"amount": True}, "0", None),  # true is not a number
    ("Boolean false", {"amount": False}, "0", None),
    ("Null value", {"amount": None}, "0", None),
    ("Array value", {"amount": [1,2,3]}, "0", None),
    ("Object value", {"amount": {"nested": 1}}, "0", None),
    ("Empty string value", {"amount": ""}, "0", None),
    ("String value not number", {"amount": "hello"}, "0", None),
    
    # Unicode/weird
    ("Key with unicode", {"amöunt": 42}, "0", None),
    ("Amount in different case", {"Amount": 42}, "0", None),
    ("AMOUNT uppercase", {"AMOUNT": 42}, "0", None),
]

passed = 0
failed = 0
errors = []

print(f"Running {len(test_cases)} JSON fuzz tests against testnet...\n")

for desc, args, expected_log, expected_return in test_cases:
    if args is None:
        print(f"  SKIP  {desc}")
        continue
    
    try:
        logs, ret = call_testnet("add_amount", args)
        actual_log = logs[0] if logs else "(no log)"
        
        if expected_log is not None and expected_log in actual_log:
            print(f"  ✅ {desc}: LOG={actual_log}")
            passed += 1
        elif expected_log is None:
            print(f"  ❓ {desc}: LOG={actual_log} (no expected value)")
            passed += 1
        else:
            print(f"  ❌ {desc}: expected '{expected_log}' got '{actual_log}'")
            failed += 1
            errors.append((desc, args, expected_log, actual_log))
    except Exception as e:
        print(f"  💥 {desc}: {e}")
        failed += 1
        errors.append((desc, args, str(e), ""))

print(f"\n{'='*50}")
print(f"Results: {passed} passed, {failed} failed")
if errors:
    print("\nFailures:")
    for desc, args, expected, actual in errors:
        print(f"  {desc}: args={json.dumps(args)}")
        print(f"    expected: {expected}")
        print(f"    got:      {actual}")
