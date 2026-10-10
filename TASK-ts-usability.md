# TASK: TS Frontend Usability — closures, compound assigns, `&&`/`||` value semantics

Repo: `/Users/asil/dev/lisp-rlm` (work on `main`, do NOT push — JP pushes)
Primary file: `src/ts_frontend.rs` (~5.5k lines, oxc-based, lowers TS-syntax → lisp s-expr source)
Tests: `tests/test_ts_surface_tour2.rs` (add new `#[test]` fns there, or a new `tests/test_ts_usability.rs`)

## Ground rules (non-negotiable)

1. **Hard-error policy**: any form you cannot lower must produce a descriptive error string naming the form — never a silent data-fallback, never a silent pass-through.
2. **No backend changes** unless a Step 0 probe proves one is required. If so, STOP and report instead of widening scope.
3. **No double evaluation.** Any lowered form that reuses a subexpression must bind it once via `let` (mangled name) and reuse the binding. (See the `near/return` double-eval history — this bit us before.)
4. **Commit discipline**: explicit paths only (`git add src/ts_frontend.rs tests/...`), NEVER `git add -A`. The tree contains JP's unrelated dirty state (`GAPS.md` modified, `rlm_runtime.lisp` deleted, untracked `ammloop.ts`, `examples/*`, `projects/tour/`, `{}`) — DO NOT TOUCH OR COMMIT ANY OF IT.
5. One commit per feature, message prefix `feat(ts):` / `docs(ts):`. Run `cargo test` before each commit.

## Step 0 — Backend probe (do this FIRST, report findings in your summary)

Before touching the frontend, verify the lisp backend actually supports what we're lowering onto:

- **Probe A (lambda-as-value)**: write a pure-lisp snippet `(define (apply2 f x) (f x))` + `(apply2 (lambda (y) (* y y)) 5)` → expect 25. Run it through BOTH (a) the bytecode interpreter bin (`src/bin/lisp_run.rs` → `lisp-run`) and (b) the wasm path (`near-compile` / compile bin + existing wasm test harness pattern from `tests/test_ts_bls_msig.rs` or `test_ts_amm.rs`).
- **Probe B (capture)**: same but the lambda references an enclosing local: `(define (mk n) (lambda (x) (+ x n)))` → call result with n=10, x=5 → expect 15. Both backends.
- The frontend header comment mentions a **"T4 closure-aliasing landmine"** (callbacks are currently inlined by `resolve_lambda_1/2` precisely so T4 never triggers). Find what T4 is (search tests/comments) and whether it affects emitted lambdas in argument position.
- If Probe A/B fail in wasm_emit → closures scope retreats to "non-capturing lambdas + existing builtin callback slots", with a hard error on capture. Report; do not fix wasm_emit.

## Feature 1 — Compound assignment (+ `**` if a power builtin exists)

`src/ts_frontend.rs:4055` currently rejects: "exponent/assign-ops in expressions not supported".

Lowering (all sugar, frontend-only):
- `x += e` → `(set! x (+ x e))`; same for `-=`, `*=`, `/=`, `%=`
- `x **= e` / `x ** e` → ONLY if the lisp already has an `expt`/`pow`-style builtin (grep builtins). If none exists, skip `**` entirely and extend the existing rejection message with a precise note. Do NOT add a backend builtin.
- `s += "str"` on an annotated/obviously-string LHS → lower via the existing strcat path (`lower_strcat_operand`, `(str ...)`). If shape isn't cleanly detectable, restrict `+=` to numbers with a clear error otherwise.
- `obj.x += e` stays unsupported (property assignment is out of scope) — keep/extend the existing descriptive error.

Tests: number ops, string `+=`, negative case (property compound-assign errors with a message naming the form), usage inside a loop body (`while`/`for` accumulate pattern).

## Feature 2 — `&&`/`||` JS value semantics

Current: lowered to boolean-valued 0/1 (documented in header "Truthiness" note). Target: JS value semantics.

- `a || b` → `(let __tsv_a a (if <truthy __tsv_a> __tsv_a b))` — single evaluation of `a`, value semantics.
- `a && b` → `(let __tsv_a a (if <truthy __tsv_a> b __tsv_a))`.
- Follow the SAME truthiness lowering the `if` statement uses (numeric by decree today; string truthiness is M2 — if operand annotations say string, emit the same behavior `if` would, i.e. existing semantics or existing error, do NOT invent string truthiness here).
- Use the existing mangling conventions for the temp binding (grep `__fn_done` / existing temp schemes; pick a collision-safe name).
- **Chains must work**: `a || b || c`, `x && y && z`, mixed with `??` (which already exists — verify interaction).
- **Audit existing tests**: some tests may assert the old 0/1 boolean results. Update any that encode the old semantics, and note each changed assertion in your report.

Tests: `const name = input || "anon"` value-select; `&&` value-select; single-eval proof (RHS with a side effect — e.g. `console.log` in operand — runs exactly once); chain; interaction with `??`.

## Feature 3 — General closures (M1)

`src/ts_frontend.rs:806` currently rejects anonymous functions outside the recognized callback slots.

Target (scoped by Step 0 results):
- Arrow function expressions in **argument position** and **variable initializer position**: `const f = (x: number): number => x * 2;` then `g(f, 5)`; `[(x) => ...].map(...)`-style nesting stays whatever it is today.
- Lower to real `(lambda (params) body)` forms, block bodies included (reuse `lower_block_tail`).
- **Capture**: if Probe B passed, support lexical capture of enclosing `const`/`let` locals. If it failed, hard-error on any free variable beyond params/globals with message "closure capture unsupported (backend T4)".
- Keep all existing map/filter/reduce inlining paths untouched — do not regress them (`resolve_lambda_1/2` stay).
- Returning a lambda from a function (`(x) => (y) => x + y` currying) — attempt only if Probe B passed; else hard error.

Tests (differential): every new TS test's expected output must ALSO be produced by a hand-written lisp equivalent run through the same harness (this is the repo's differential-proving convention). Include: non-capturing arg-position lambda, capturing lambda (or its hard-error negative test), lambda in initializer + later call, callback pipelines still passing (regression), negative test for whatever remains unsupported (e.g. property assignment error message unchanged).

## Feature 4 — Header doc truth-up

`src/ts_frontend.rs` lines 1–40: the ✓/✗ list is stale (says ✗ for function declarations/destructuring/early returns which now work; omits ternary, `??`, template-literal notes). Rewrite the list to match reality AFTER features 1–3 land, including new entries for compound assigns, value-semantics `&&`/`||`, closures status. Fold into the last feature commit or its own `docs(ts):` commit.

## Definition of done

- `cargo build` clean, `cargo test` fully green (old-semantics assertions updated deliberately, not deleted).
- 3–4 commits on `main`, explicit paths, no push.
- Report back: Step 0 probe results (A/B + T4 finding), what landed, any scope retreats + why, list of intentionally-changed old test assertions.
