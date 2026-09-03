# TASK-wasm-u128 — u128 string-family → WASM lowering

## Status (2026-08-25 22:25): PARTIAL, stopped at to_str runtime bug

Committed: a27b0b2 (helpers+ops+dispatch), 41aee4b (alignment + println fixes).

## What works (machine-verified via near-compile + near-mock)
- All 11 `__h_*` helpers (parse/to_str/add/sub/mul/divmod/i64_to_str) compile,
  validate, and are exported; KEEP-print shows expected instruction counts.
- All u128 ops lower to wasm and pass `wasmtime validate`.
- `println` of string literals: correct quoted rendering (`LOG: "hello"`), incl.
  the closing quote — fixed off-by-one (log len was `l+1`, must be `l+2`).
- Runtime bump allocators (str-cat dst, println dst) now round up to 8:
  tagged string ptrs need 8-aligned addresses because low 3 bits carry the tag.
  This was the root cause of the mid-copy traps (misaligned `i64.load`).

## Where it stopped (the ladder rung)
`(println (u128/add "1" "2"))` returns raw 9223089462349659902 — tag bits = 6
(array/pair), payload junk → to_str's digit buffer is garbage.

### Suspected root cause
The aligned copy-down loop I added at the end of `__h_u128_to_str` reuses
local 8 as the copy index ("bit local, dead now") — it is NOT dead: it still
holds live divmod remainder/quotient state feeding the loop above, or is used
by the caller. Result: digits never land where the tagged ptr points.
Same pattern in `__h_i64_to_str` (reuses local 2 as index).

### Fix sketch (next session, ~15 min)
In `call_u128_str.rs`: allocate a fresh scratch local (`next_local += 1`) for
the copy index in both to_str and i64_to_str instead of reusing 8/2. Then
re-run:
  printf '(println (u128/add "1" "2"))' | near-compile → near-mock → expect `LOG: "3"`.

## Other known issues found (not regressions)
1. **3-arg str-cat drops the last arg**: `(str-cat "a" "b" "c")` → `"ab"`.
   Same family as the documented ARITY-PIN (GAPS.md "user-fn arity").
   Also emits a phantom empty-string LOG before it (two log_utf8 calls for one
   println when arg is a computed expr — needs its own look).
2. probe2.lisp full battery still traps after line 3 — retest after the
   to_str fix; several of those traps were the alignment issue now fixed.
3. `wasm-objdump` on macOS awk: `strtonum` unavailable (mawk vs gawk) — use
   sed/grep ranges, not awk numeric compares.

## Infra notes
- Disk filled mid-run (errno 28); `rm -rf target/debug/incremental` freed 2.8G.
  Build OK since. If it recurs: prune target/debug/deps (7.7G).
- A concurrent `git stash` (interpreter truthiness work, stash@{0}) swept the
  wasm WIP — recovered via `git stash pop`; both strands now committed cleanly
  (truthiness fix + t20 pin flip are in a27b0b2 too).
