# TASK-ts-usability — progress notes (subagent ts-usability-2)

Repo: /Users/asil/dev/lisp-rlm, branch main. Do NOT push. Never `git add -A`.
JP's dirty tree (GAPS.md, rlm_runtime.lisp del, ammloop.ts, examples/*, projects/tour/, {}, fixtures/amm.wasm.map) — never add.
src/typing/types.rs has JP's pre-existing H3 "error" builtin change — DO NOT TOUCH that file.
tests/probe_closures_tmp.rs is TEMPORARY (Step 0 harness) — delete before final commit.

## Step 0 COMPLETE (2026-10-04) — probe verdicts

Harness: tests/probe_closures_tmp.rs (near-mock cross runs, `to-string` returns, assert `📄 N`).
GOTCHA: asserting bare digits false-passes on temp wasm path (`probe_25535.wasm` contains "25") — always assert `📄 N`.
GOTCHA: `(str n)` is not a lisp callable — use `(to-string n)`.

### Interpreter (target/release/lisp-run) — ALL PASS
- A lambda-as-value: 25 ✓; B capture (param): 15 ✓; B2 two-invocation T4 independence: 302 ✓.

### WASM (near-mock) — 3 landmines
1. ORDER (loud): `(define (apply2 f x) (f x))` emitted before ANY lambda → compile err "in apply2: unknown function 'f'". lambda_info populates in emit order; named fns get pre-pass, lambdas don't (compile.rs pre-scan only registers defines).
2. DISPATCH FREEZE (SILENT, probe D): a `(f x)` site's if/else chain enumerates lambda_info AT EMIT TIME → lambdas emitted later hit -1 fallback. Probe D: 10 + (-1) = 📄 9, correct 110.
3. T4 CAPTURE ALIASING (SILENT, probe B2): capture cells at compile-time constant heap addr (lambda.rs heap_bump at emit) → 2nd invocation of closure-creating fn overwrites 1st. B2: 📄 402, correct 302. Single-invocation capture works (B: 📄 15, B3 local-capture: 📄 10) but frontend can't prove single invocation.

### Safe shapes proven both backends
- C1: non-capturing lambda bound to local, direct-name call: 📄 42 ✓ wasm + interp.
- C2: same inside while loop (4 calls): 📄 12 ✓ wasm.

### T4 definition
t4-closures.lisp (compiler-torture): closures over mutable state must have per-invocation independent cells. Bytecode fixed (2026-08-26 round-3). wasm NOT fixed. Frontend header line 21: callbacks are inlined by resolve_lambda_1/2 precisely so T4 never triggers.

## Feature-3 scope decision (per task retreat rule)
- SUPPORT: non-capturing arrow fn in variable-initializer position, called DIRECTLY by name in same function → `(let ((f (lambda ...))) ...)`. Block bodies via lower_block_tail.
- HARD ERROR (descriptive): any free variable in lambda beyond params/globals → "closure capture unsupported (backend T4)".
- HARD ERROR: lambda expr in argument position to user fns; passing/returning a lambda-valued var (track via side-set like STRING_LOCALS) → dispatch-freeze landmine.
- Keep map/filter/reduce resolve_lambda_1/2 inlining untouched.

## Session 3 (2026-10-04, subagent ts-usability-3) — resuming

- Reviewed dead agent's Feature-1 partial work in src/ts_frontend.rs (uncommitted): *= /= %= expansion,
  **/**= hard errors, extended element-write/property messages, tests/test_ts_compound_assign.rs complete.
- VERIFIED myself: NO expt/pow in NEAR builtin set (src/typing/types.rs near-builtins have neither; expt
  only in interpreter helpers.rs + bytecode dispatch_arithmetic). ** skip decision CONFIRMED correct.
- cargo build (debug+release bins) OK. All 10 compound-assign tests PASS.
- INFRA GOTCHA #3: disk hit 100% ("No space left on device") → bogus linker failures on 4 test bins
  (test_borsh_gaps, test_compiler_v3, test_continue_keyword, test_trace_overflow) that PASS standalone.
  Fixed by `rm -rf target/debug` (freed 14G; release profile kept — 82 test refs use ./target/release/*).
  Full debug rebuild + baseline `cargo test` running.
- INFRA GOTCHA #3b: full debug rebuild CONSUMED the freed 14G again (177 test binaries with debuginfo
  ≈ 2.7G+; disk is globally tight, 431G/460G used by other stuff — not ours). REMEDY (final):
  `rm -rf target/debug/incremental; rm -f target/debug/deps/test_*` then run
  `CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test` — test bins link without debuginfo
  (~10x smaller). KEEP THIS PROFILE FOR ALL `cargo test` RUNS THIS TASK or the disk fills again.
- NOTE: pipeline `cargo test | grep` masks exit codes (tail exit 0) — always redirect to file.

### F2 plan (studied, ready to implement after F1 commit)
- Current &&/|| in lower_expr (~L4139): boolean 0/1 via to_bool both arms. Coalesce arm untouched.
- New: a||b → (let ((__tsv a)) (if __tsv __tsv b)); a&&b → (let ((__tsv a)) (if __tsv b __tsv)).
  Fixed name __tsv + let-shadowing is the existing convention (__not_tmp does the same).
- checker infer_if allows ANY cond type ("allow any for truthy"), unifies branch types — mixed-type
  a||b gets a natural checker error; bool-typed operands keep 0/1 values (old tests stay valid unless
  they assert number-y operands coerced to 1).
- while/for false-arm typing uses statically_bool (still true for LogicalExpression) — safe for
  well-typed TS (while-cond operands are bool).
- emit_cond_branch: tag-aware (falsy = Bool false/Nil/Num 0) — same truthiness as if-statement. Requirement met.

### F3 reconnaissance
- lower_expr ALREADY lowers ArrowFunctionExpression → (lambda (params) body) via arrow_parts
  (expression body, single-return block, or lower_block_tail for general blocks).
- Local `const f = arrow` hits the PURE binding path (expr_has_call(arrow)=false) → (let ((f (lambda..))) ..)
  — exactly the target shape. Suspect basic case may already work; needs live probe + capture-detection
  hard-error + lambda-as-argument hard-error + side-set tracking for lambda-valued vars.
- F3 impl plan: (a) GLOBAL_BINDINGS pre-pass (top-level fns + consts) so capture classification is
  order-independent; free-var walk of arrow bodies in INITIALIZER position only (map/filter/reduce
  callback inlining untouched per scope decision); free = used −(arrow params ∪ body-internal bindings);
  free∩globals OK, else hard error "closure capture unsupported (backend T4)".
  (b) LAMBDA_LOCALS cell (like STRING_LOCALS, cleared in lower_function): registered on initializer arrows;
  Identifier-arm hard error when a lambda local is used in VALUE position (g(f), return f, f+1, o.f...);
  direct-name callee calls stay allowed. (c) top-level non-exported `const f = arrow` → value define =
  stub landmine → hard error w/ suggestion (export const f = arrow / function decl).
  (d) capture-check also fires for self-recursion (fact → error) — correct, let has no self-ref.
  (e) PROBE then decide: .map(x => f(x)) calling local lambda from callback; nested callback capturing
  initializer-lambda's params. If backend chokes → hard error; if works → keep + test.

## TODO
- [x] Feature 1: compound assign — DONE, committed d47156b. *= /= single-op, %= truncated-mod w/ bind-once impure rhs, ** hard-error (no expt builtin).
- [x] Feature 2: &&/|| value semantics — DONE, committed 431e1f3 + 9ed3829 (near-mock 8-byte printer fix, 3 print sites).
- [x] Feature 3: scoped closures — DONE, committed d2fd048. Whole-function static analysis (check_fn_closure_safety) — inline checks were ORDER-BLIND (hoisting pre-pass reorders lowering; lesson banked). Supported: local const-arrow + direct call + IMMUTABLE capture (probed both backends). Hard errors: T4 mutable capture, dispatch-freeze (lambda-local called in pipeline callback — probed: emits INVALID wasm), arrow-as-call-argument. 7/7 tests.
- [x] Feature 4: header doc truth-up — DONE (2026-10-09 after F1–F3; extended 2026-10-10 with string truthiness + the two latent backend bugs its landing exposed — see ts_frontend.rs header).
- [x] 2026-10-10 follow-up batch (main agent): t1 parse-cache loop-back staleness (the splt3 freeze — backend root cause, force-invalidate set!-targets + HOF param rebinds; tests/test_parse_cache_loop.rs 4/4), t2 pipeline verification (chained map/filter/sum = 10:2:9 exact — see below), t3 u128 closed-namespace did-you-mean (tests/test_ts_u128_namespace.rs), t4 stale-wasm src-stamp guard (tests/test_stale_wasm_guard.rs), t5 string truthiness (tests/test_str_truthiness.rs 7/7), t6 for-sugar pin (tests/test_for_loop_sugar.rs 6/6).

## Findings (F3)
- ~~`.map((x) => x*3)` over json array param TRAPS at runtime~~ **SUPERSEDED 2026-10-10**: typed-param `.map` verified working (📄 6); the REAL defect was HOF param-rebind parse-cache staleness (silent zero-count filters — same family as t1), FIXED in call_list.rs; chained map/filter/reduce verified end-to-end `📄 10:2:9` = oracle (tests/test_parse_cache_loop.rs::hof_pipeline). "Pipeline chaining needs re-verification" → done, green.
- Mock temp-file collisions in parallel tests: use unique filenames (atomic counter) — bit test_ts_closures.
- Baseline failing suites (pre-existing, NOT from this task): surface_parity (schnorr/vrf interp parity), test_continue_keyword (2), test_pool_v3 (2), test_ts_pool_internal (1). Verified by stash A/B. 156 suites green otherwise.
- [x] probe_closures_tmp.rs deleted (expt probe failures resolved: no builtin)

## Pre-existing test failures (NOT from this task — verified by stash A/B):
**RESOLVED by 2026-10-10**: full-suite gates (`/tmp/fullsuite{3,4}.log`) report 0 FAILED across
all 188 suites — surface_parity, test_continue_keyword, test_pool_v3, and test_ts_pool_internal
all pass now. The morning batch (JP's 09:16 fmt+checker/ops_spec work + the t1/t5 emitter fixes)
touched both layers; exact fix attribution per suite was not bisected. Keep the list above as
historical record only — do not cite these suites as failing anymore.
