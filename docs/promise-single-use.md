# Promise Single-Use — the double-await hazard and its detection tiers

*Design note JP ↔ Gork, 2026-10-08. Companion to `typed-surface-design.md` (T7 affine) and the async v2 arc (`ts_frontend.rs`).*

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
- **Nothing** detects awaiting the *same promise value* at two await points.
  `(await p) ... (await p)` typechecks today and compiles to two registered
  callbacks on one promise.

This is the single most expensive silent failure mode the language currently has.

## Why promises are the canonical case

A promise handle is **move-only by intent**: `promise_then`, `promise_return`,
`promise_and` are consuming positions — once you've chained or returned a
promise, using it again is a program bug, not a program choice. That makes the
promise family the perfect scoped target for affine discipline before any
general linear typing exists.

## Tier 1 — use-count lint on promise bindings (hours; do now)

Defn-local syntactic pass:

- Track every binding whose value is a promise (`near/promise_batch_create`,
  `near/promise_create`, `near/promise_and`, TS `near.call` results awaiting).
- Count uses in **consuming positions**: base arg of `promise_then`, argument
  of `promise_return`, operand of `promise_and`.
- Second consuming use → hard error naming both lines:
  `error: promise 'p' consumed twice (L41, L67)`.

No type system, no flow analysis beyond defn-local bindings — affine checking
collapsed to one op family. Kills the class for hand-written `.lisp` and for
TS-authored code at the emitted surface.

## Tier 2 — mock-enforced single-use at test time (hours; do now)

`near-mock` maintains a promise-id registry:

- Each id accepts at most **one** consumer (`promise_then` / `promise_return`).
- Second consumer → trap + report (op, state, promise id).
- The 10k-op invariant battery then catches double-consume paths the syntactic
  lint cannot see through: dynamically chosen promises, handles round-tripped
  through storage, callback re-entries.

Defense in depth: the lint guards the source you write; the mock guards the
code that runs.

## Tier 3 — T7 affine: `Promise` as a move-only value (the endgame)

With affine resource types (see `typed-surface-design.md` §T7):

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
