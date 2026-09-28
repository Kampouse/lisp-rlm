# BUG: `if` without `else` lowered to a numeric 0 hole (fixed 2026-09-17)

**Severity:** HIGH (legal TypeScript falsely rejected by the checker)
**Status:** FIXED + pinned (tests/test_if_noelse_regression.rs, 2/2 green)
**Found by:** cfg differential fuzz corpus — 20/60 generated files rejected
with `if: branch types disagree — type mismatch: str ≠ int`

## Symptom

```ts
// legal TS — compiles everywhere, rejected here:
if (acc % 4 == 0) { s = s + "b"; } else { if (acc > 31) { return s + "!"; } }
```

Checker error: `if: branch types disagree — type mismatch: str ≠ int`.
Minimal repro (both nestings failed):

```ts
if (n == 0) { s = s + "a"; } else { if (n > 5) { return s + "!"; } }  // str ≠ int
if (n == 0) { if (n > 5) { return s + "!"; } } else { s = s + "a"; }  // str ≠ int
```

## Root cause (two stacked defects)

1. **`lower_tail_stmt` / capture-arm None holes**: a statement-if with no
   `else` lowered its missing branch to `Num(0)` — an INT value. Combined
   with a str-returning branch: `(if c <str> 0)` → str ≠ int → false
   reject. Fix: the hole is `(quote nil)` — Nil is bottom (unify unifies
   it with anything; same idiom as the `__fn_res` init).

2. **Blanket `__fn_res` capture mis-fire** (EXPOSED by fix 1 — pre-fix
   these programs never type-checked, so the miscompile was unreachable):
   the mid-function capture wrapped the whole branch value even when the
   branch's return was CONDITIONAL — `{ if (c) { return v; } MORE }` set
   `__fn_done = 1` on the branch's fall-through path too, silently
   skipping `MORE` and every statement after the if. Fix: the commit now
   happens AT the return site (conditional-carrier lowering in
   `lower_tail_stmt`, guarded by `FN_FLAGS_BOUND`), and the blanket
   capture skips branches whose tail is already a commit form
   (`is_commit_form`).

## Verification

- 20/20 formerly-failing corpus files now compile.
- `tests/test_if_noelse_regression.rs`: early-return wins; fall-through
  produces the tail value and leaves `__fn_done` clear.
- Neighbor suites untouched: dynamic_json 18/18, money_safety 16/16,
  surface tours 10/10 + 11/11, compiler_bugs, json_sizes, onchain_verifier.
