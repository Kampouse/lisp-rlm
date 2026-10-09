# Promise Single-Use — the double-await hazard and its detection tiers

*Design note JP ↔ Gork, 2026-10-08. Companion to `typed-surface-design.md` (whose T5/T6 ladder affine types
would extend) and the async v2 arc (`ts_frontend.rs`).*

---

## The hazard (verified in-tree, 2026-10-08)

`near/promise_then` **does not consume** its base promise at the host level.
Registering two callbacks on the same promise is *legal* — and **both fire**.
No trap, no error: the continuation executes twice. For a payout path that
means double-pay, double state mutation, fully silent.

Current state of the merged tree:

- `ts_frontend.rs` async v2 has real discipline: the **T4 frame rule**
  (locals die at the await boundary — a read of a `let` after an await is a
  compile error naming the variable) and a lowering that guarantees **exactly
  ONE `promise_then` per await node** in generated code.
- **The TS surface is already safe by construction:** v2 accepts only
  `await near.call(...)` directly (verified 2026-10-08 — `const p =
  near.call(...); await p` is rejected: "await must wrap near.call"). A
  promise handle can never be held in a TS binding, so `(await p) ...
  (await p)` is unrepresentable there.
- **The exposure is the `.lisp` surface.** Nothing detects two consuming
  uses of one promise binding. Verified in-tree 2026-10-08: a defn that
  runs `near/promise_then` on the same `p` twice compiles clean (exit 0,
  no warning) — two registered callbacks, both fire.

This is the single most expensive silent failure mode the language currently has.

## Why promises are the canonical case

A promise handle is **move-only by intent**: `promise_then`, `promise_return`,
`promise_and` are consuming positions — once you've chained or returned a
promise, using it again is a program bug, not a program choice. That makes the
promise family the perfect scoped target for affine discipline before any
general linear typing exists.

## Tier 1 — use-count lint on promise bindings (hours; do now)

Defn-local syntactic pass:

- Track every defn-local binding whose value is a promise
  (`near/promise_batch_create`, `near/promise_create`, `near/promise_and`,
  `near/promise_then`). The TS side needs no lint — its await-shape rule
  already forbids holding promises in bindings.
- Count uses in **consuming positions**: base arg of `promise_then`, argument
  of `promise_return`, operand of `promise_and`.
- `promise_batch_action_*` is NOT a consuming position — batch actions
  legally accumulate on one handle (create → action… → then → return is the
  canonical ft2.lisp idiom). Only the three callbacks above consume.
- Second consuming use → hard error naming both lines (actual format,
  shipped 2026-10-08):
  `error: promise 'p' consumed 2 times (promise_then (line 4), promise_then (line 5)) — a promise handle is single-use; both callbacks would fire on chain`.
- **Scope widened same day (SHIPPED, cross-defn flow)**: keys are per
  `(defn, binding)` — the pre-flow name-keyed lint would have falsely
  flagged two sibling defns each consuming their own `p`; fixed. Each defn
  contributes a consumption summary (direct uses + params marked
  consuming); call sites contribute flow edges for bare-symbol promise
  arguments; a fixpoint over the call graph flags three buckets:
  1. let-bound promise consumed twice in one defn (message above);
  2. let-bound promise whose **total across call edges** reaches 2 — the
     x10 class —
     `error: promise 'p' consumed 2 times across calls (promise_then in use1 (line 5), promise_then in use2 (line 7)) — …`;
  3. a defn param consumed twice in total (every caller passing a promise
     loses both callbacks).
  Two-hop chains are caught; self-recursive forwarders terminate via a
  cycle guard (per-iteration re-consume stays the runtime trap's job).
  Composite carriers SHIPPED same day (v3 value model): producers mint
  round-stable origin ids; call edges carry elementwise tuples
  (Handle | Tuple | Opaque) that a fixpoint joins into callee params;
  `car`/`first` and `cdr`/`rest` peel tuples positionally. The x16
  class — a `(list p ...)` tuple passed to two callees that each
  consume `(car t)` — and the ground-tuple x17 class (`(car (cdr t))`)
  are now compile errors. Counts are call paths, not static sites (a
  handle reaching one leaf site by two routes reports `consumed 2
  times` with that single site named). Ambiguity still degrades to
  Opaque: dynamically chosen promises, storage round-trips and
  callback re-entries remain the Tier 2 trap's job.

No type system, no affine discipline — flow analysis collapsed to one op
family (per-defn summaries + call-graph edges). Kills the class for
hand-written `.lisp` and for TS-authored code at the emitted surface.

## Tier 2 — mock-enforced single-use at test time (SHIPPED 2026-10-08)

`near-mock` maintains a promise-id registry (`CONSUMED_PROMISES`), shipped in
the vendored near-mock (`src/bin/near_mock/mod.rs`):

- Each id accepts at most **one** consumer. Consuming hosts:
  `promise_then`, `promise_batch_then`, `promise_return`, and every element
  of `promise_and`. `promise_batch_action_*` marks nothing (non-consuming).
- Second consumer → trap:
  `promise 0 already consumed: promise_then on an already-consumed handle (promises are move-only, single-use)`.
- The invariant batteries then catch double-consume paths the syntactic
  lint cannot see through: dynamically chosen promises, handles round-tripped
  through storage, callback re-entries.
- Verified in-tree 2026-10-08: single-use chains run green; a cross-defn
  double-`promise_then` is now a **compile error** (cross-defn flow, same
  day — see Tier 1). The trap remains the backstop for what static flow
  cannot see: dynamically chosen promises, handles round-tripped through
  storage, callback re-entries, non-symbol argument carriers (probes
  `x11_single_use.lisp` / `x10_cross_defn.lisp`).

Defense in depth: the lint guards the source you write; the mock guards the
code that runs.

## Tier 3 — T7 affine: `Promise` as a move-only value (the endgame)

With affine resource types (beyond the typed doc's current ladder —
`typed-surface-design.md` tops out at T5 effects / T6 phantoms; affine is the
agreed next rung, not yet written up there):

```
(await p)        ;; moves p
...
(await p)        ;; error: use of moved value 'p' (L41 → L67)
```

- Compile-time, structural, forever — no lint to bypass, no mock needed.
- Same machinery extends to `Deposit` and `Payout` for free (double-spend
  becomes unrepresentable — Move's conservation thesis).
- Until T7 lands, tiers 1+2 are the same idea scoped to where money flows.

## Gates

- Lint: fixtures with legal single-use compile clean; a double-await fixture
  fails with both line numbers (new test pair, suite green).
- Mock: a double-consume driver test traps with the report; all existing
  promise fixtures (portfolio joins, two-leg payouts) stay green.
- Corpus byte-identity: lint is tooling; emit untouched — determinism sweep
  must stay byte-identical.

## See also

- `docs/typed-surface-design.md` — five inference rules, effects, T5–T8 ladder.
- `TASK-TYPED.md` — T1–T4 implementation spec (queued behind M3).
- Near-term safety floor (same session): pre-commit gate chain
  (paren-balance → lisp-diff → byte-compare), deployed-byte verification,
  storage-key registry lint, promise-pairing lint (this doc's tier 1 is its
  consuming-position half), 100k-case oracle sweeps, testnet soak of the
  invariant battery.
