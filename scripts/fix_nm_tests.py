#!/usr/bin/env python3
"""Fix two obsolete near_mock tier tests to match current (correct) semantics."""

PATH = "/Users/asil/dev/lisp-rlm/tests/test_regression.rs"
R = []

# T1: 2^60 is outside the tagged range [-2^60, 2^60) — compiler now rejects
# it at compile time. Use the largest representable product instead.
R.append((
'''        // 2^30 * 2^30 = 2^60 which overflows i32 but fits i64 — should succeed
        let out = run_near_mock("(define (main) (* 1073741824 1073741824))", "_run", "{}", None);
        let ret = extract_return(&out).expect("should have return value");
        // 2^60 = 1152921504606846976
        assert!(ret.contains("1152921504606846976"), "expected 2^60, got: {}", ret);
    }''',
'''        // 2^30 * 2^30 = 2^60 is OUTSIDE the tagged payload range [-2^60, 2^60):
        // the compiler now rejects that literal at compile time (silent-corruption
        // guard). The largest exact product must stay strictly below 2^60.
        let out = run_near_mock(
            "(define (main) (* 1073741824 1073741823))",
            "_run", "{}", None,
        );
        let ret = extract_return(&out).expect("should have return value");
        // 2^30 * (2^30 - 1) = 1152921503533105152
        assert!(
            ret.contains("1152921503533105152"),
            "expected 2^60 - 2^30, got: {}",
            ret
        );
    }'''))

# T2: (attached-deposit) is the low-64-bit form — 2e18 exceeds the 2^60 tagged
# payload range, so the low read is garbage by design. The u128 host form
# returns the EXACT decimal string (verified 2026-09-05).
R.append((
'''        let out = run_near_mock(r#"
(define (main)
  (let* ((bal (attached-deposit)))
    (to-string bal)))
"#, "_run", "{}", Some("2000000000000000000"));
        let ret = extract_return(&out).expect("should have return value");
        // Should log or return something with the deposit value
        assert!(out.contains("2000000000") || out.contains("2e18") || out.contains("2000000000000000000"),
            "expected deposit value in output, got: {}", out);
    }''',
'''        // near/attached_deposit_u128 renders the EXACT decimal u128 (the low-64
        // (attached-deposit) form corrupts values >= 2^60 by design — payload
        // range). Round-trip verified 2026-09-05.
        let out = run_near_mock(
            r#"(define (main) (near/attached_deposit_u128))"#,
            "_run", "{}", Some("2000000000000000000"),
        );
        let ret = extract_return(&out).expect("should have return value");
        assert!(
            ret.contains("2000000000000000000"),
            "expected exact deposit 2000000000000000000, got: {}",
            ret
        );
    }'''))

with open(PATH) as f:
    c = f.read()
for i, (old, new) in enumerate(R, 1):
    n = c.count(old)
    assert n == 1, f"T{i} anchor count {n}"
    c = c.replace(old, new, 1)
    print(f"T{i}: applied")
with open(PATH, "w") as f:
    f.write(c)
print("done")
