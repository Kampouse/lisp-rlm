# TASK: Correctness sweep — interp surface drift + recur hard-error + (memory N) hoist

> **Status: DONE** — merged (Aug-31 blitz, hard-error policy in main). Full history in `git log`.

**Repo you work in:** `/Users/asil/dev/lisp-rlm` (NOT the workspace clone — another agent is working there; do not touch it).
**Branch:** `fix/interp-surface-drift` off `main` (e48d64c). NEVER commit on main, NEVER push.
**Timebox:** ~90 min. Commit early/often with explicit paths.

## A. Interp↔emitter surface inventory + port (the T6 class, GAPS.md ~line 217)

1. Build an inventory: enumerate builtin names dispatched by the wasm emitter (src/wasm_emit/, incl. call_string.rs, call_near_storage.rs etc.) vs builtins handled by the interpreter's `eval_builtin` (src/bytecode.rs / interp path). A unit test `surface_parity` that FAILS with a printed diff list is the ideal artifact — the class must not silently regrow.
2. Port the missing ones into the interpreter. Known: `near/has_key`, `near/kv-get`, possibly more from round-2 notes. Semantics must match the wasm path EXACTLY — including argument-count and type error behavior (repo policy: hard errors, never silent fallback; see how `str-cat` was resolved for precedent — strings-only semantics matching call_string.rs).
3. If a builtin is genuinely unimplementable in interp (needs wasm memory layout), document it in the inventory test with a reason instead of porting.

## B. `recur` out of direct tail → compile-time hard error

Found the hard way (bignum run died ~depth 700): `recur` inside `(begin ... (recur ...))` or non-tail position silently compiles to real recursion instead of the frame-free loop. Fix: reject at compile time with a clear error (e.g. `recur must be in direct tail position within its loop`), in the wasm emit path AND the interp compile path if they share the form check.
**Caution:** the bignum corpus programs (/tmp copies may be gone; tests + projects/nostr-gov-lisp corpus in-repo) legitimately use `(begin (buf-set! ...) 0)` tails and direct-tail recur — those must keep compiling. Run the full corpus/battery; if a legit program relies on non-tail recur, STOP that part and report instead of forcing the break.

## C. Hoist `(memory N)` declarations

Current bug: memory-page guards bake `memory_pages` at emit time, so a `(memory N)` declared after defines leaves early functions with the 64-page default. Fix: hoist `(memory N)` to the top during emit prep (same treatment consts get). Test: program with `(memory N)` after defines must compile identically (same wasm bytes or same semantics via trace test) to decl-first ordering.

## Verify (all before reporting done)

- `cargo test --workspace --no-fail-fast` — must be fully green (baseline ~1426+ tests).
- Corpus / differential harness if present in-repo (projects/, tests/) — values unchanged.
- Any new wasm behavior gets checked through near-vm-run (stricter than wasmtime).
- Machine-verify constants you embed in tests (python3) — hand-math has burned this repo 6× in one day.

## Rules

- NEVER `git add -A` — stage explicit paths only.
- Do not touch: `stash@{0}` (labeled WIP), any untracked files, main.
- Report: commit list, surface-diff table (before/after), corpus results, anything you stopped on.
