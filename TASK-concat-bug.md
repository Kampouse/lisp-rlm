# TASK: Engine str-concat expression path zeroes prefix operands

## Summary
A single-expression multi-operand string concat `a + b + c + d` (and in some
shapes even `x + y` of let-bound long strings) returns CORRUPTED output:
the PREFIX operands' bytes come back as NULs; only the last operand survives.
The statement-wise growing path `buf = buf + x` in a loop is CORRECT.

Proven on NEAR testnet 2026-09-02 (lisp7 debug series), not hypothetical.

## Observed manifestations (real, on-chain)
1. `return sigma + gg + pt + apk;` where sigma = host `p1_sum` result
   (192-hex), gg/pt = storage strings (384/192-hex), apk = host `g2_multiexp`
   result (384-hex): returned 768 NUL chars followed by apk's 384 chars,
   correct total length 1152.
2. `let half1 = sigma + gg; let half2 = pt + apk; return half1 + half2;`
   returned an 8-byte garbage value (wrong length entirely — looks like a
   tagged pointer leaked as the string).
3. Loop accumulation `sigBlob = sigBlob + s;` up to 582 hex chars and
   `mxBlob = mxBlob + pkc;` up to 1348 hex chars: CORRECT (values verified
   byte-identical against py_ecc oracle).
4. Short/literal concats (`"bls:sig:" + id + ":" + i` shape, via template
   literals in fixtures; error message building) work fine — battery green.

So the failure needs LONG operands and/or HOST-RESULT strings (p1_sum,
storage_get results — heap-resident runtime strings).

## Your job
1. Write a FAILING test in the mock harness (near_mock) reproducing the bug
   without testnet: stub a host (e.g. BLS p1_sum stub or storage) to return
   long strings, concat ≥3 of them in one expression, assert equality with
   the expected concatenation. Also repro shape (2) if it falls out.
2. Root-cause in `src/wasm_emit/` str-cat lowering. Hypothesis: the
   expression path allocates the dst ONCE at total size, then copies each
   operand — and something (scratch aliasing / heap_bump sync / copy offset
   arithmetic) zeroes or skips the earlier regions. The `x = x + y` statement
   path grows incrementally through a different (correct) code path. Diff them.
3. Fix the expression path. Do NOT regress the loop path.
4. Optional finisher (only if the fix is clean): revert the `+=` workaround
   in `fixtures/bls_msig.ts` back to the single-expression form and prove the
   mock test still passes. Keep as separate commit.
5. `cargo test --release` battery must stay green (1576/0 as of bf02ea8).

## Constraints
- Scope: `src/wasm_emit/**` + exactly one new test file (or extend an
  existing ts-string test file if more natural). NOTHING else.
- Never `git add -A`. Explicit paths only.
- Commit message references this file. Push NOT needed — I verify and push.
- If root cause turns out to be in alloc_data/heap_bump interplay rather
  than str-cat lowering itself, that's fine — just explain it in the commit.

## Verification oracle
/tmp/bls_client.py + /tmp/bls_txs.json hold the py_ecc-truth values; the
mock's BLS stubs return deterministic strings — the test's expected values
can be computed in-test by plain string concat in Rust.
