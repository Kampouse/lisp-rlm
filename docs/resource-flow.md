# Compile-Time Resource Flow — gas, arity, money domain, deposit

Landed 2026-10-08 as `dd1081e6`. All gates live in
`src/typing/checker.rs::check_resource_flow`, wired into BOTH compile
pipelines (`compile_near` source path and `compile_near_from_exprs`).
House invariants hold: checker-only, rejection via `Err` (never panic),
no new host functions, and the soundness rule — **hard reject only what is
provably broken; everything probabilistic degrades to a warning.**

## Hard errors (provable violations)

| Class | Trigger | Rationale |
|---|---|---|
| Arity | promise/transfer op called with wrong argument count (slot maps transcribed from the emitter: `promise_then`=6 (gas@5, amt@4), `promise_create`=5 (gas@4, amt@3), `batch_action_function_call`=5 (gas@4, amt@3), `batch_action_transfer`=2 (amt@1), `transfer`/`transfer_u128`=2 (amt@1), `call` sugar=5 (gas@3, amt@4), `batch_create`/`batch_then`=per emitter) | runtime trap, provable from the source |
| Gas cap | single attached gas > 300 Tgas, or the sum of gas in one defn closure > 300 Tgas. A symbol at a gas slot **resolves** when it names a top-level immutable numeric define (`(define TGAS 300000000000000)`) that no binder (param/lambda/let) shadows anywhere in the program — sound because the const cannot change and cannot be rebound; every other shape stays opaque and the sum remains a sound lower bound. Regression (2026-10-09): `collect_bodies` skipped zero-parameter defines — the `(define (main) …)` contract entrypoint shape — so every closure check on them silently summed 0, and a contract attaching 6×300 Tgas via one hoisted const shipped to testnet ("Exceeded the prepaid gas" on all six receipts). | mainnet per-tx limit; consts are immutable so resolution is exact, unresolved terms only shrink the lower-bound sum |
| Raw money arithmetic | `+ - * / %` on a u128-domain value | i64 wrapping silently corrupts amounts > 2^63 (the exact class the TS surface rejects) |
| Truncation | `u128/to-i64` result used at a money slot | drops the high 64 bits of a 128-bit amount |
| Bad literal | non-decimal string literal at a money slot (`promise` amount, `batch_action_transfer`, `transfer_u128`) | runtime parse trap |
| Double attach | one `attached_deposit_u128` call-site value flowed (via let*/params) to 2+ promise-op amount slots | the second attach spends funds the transaction does not have; identity tracks through call paths with join=sum |

## Warnings (risk, not proof)

- **Same-program callback existence**: `promise_then p (near/current_account_id) "cb"` where `cb` is not a define in the file. Cannot be a hard error — the account argument may be any cross-contract string, so the string is only *suggestively* local. The warning names the dead-receipt class (receipt fails, parent commits — downstream silently dead).
- **Callback gas floor**: for callbacks that ARE defined in-file, a static lower bound (straight-line instruction cost + conservative minimum per host op + attached receipt gas) is compared against the attached gas. Lower bounds are sound (loops only weaken upper bounds, never floors); upper bounds are NOT claimed. Fires only when floor > attached.
- **Gas ≤ 0**: a zero/negative literal attachment produces a receipt that can never run (dead-receipt class, parent still commits).

## Compile-time gas fill (`(gas default)`)

At any promise-op gas slot, the sentinel `(gas default)` expands to a
static floor literal BEFORE the gate, the type checker, and emission
(`pipeline_from_exprs` → `src/typing/gas_fill.rs`):

    fill = (CB_FLOOR_BASE + CB_FLOOR_PER_HOST × hosts_closure(callback)) × 2

The model is the same one validated against real-VM replay (Tier B
below); ×2 headroom covers the measured <1.5× replay margin. Because the
expansion is a plain literal before every later stage, the fill
participates in the 300 Tgas closure sum like any other attachment.

- **Same-program callbacks only**: locality is proven when the target
  argument is `(near/current_account_id)` AND the method names a define
  in this file. Any other callee is code we do not own — its floor is
  unknowable, so `default` there is a HARD ERROR, never a silent guess.
- **Undefined local callback**: also a hard error (no closure → no floor).
- **Explicit numeric gas unchanged**: a literal at the slot is the
  override and behaves exactly as before the sentinel existed.

## Storage money stamps

`storage_write`/`storage_set` of a u128-domain value stamps the key
literal. `storage_read`/`storage_get` of a stamped key seeds the read as
u128-domain — so the write/read round-trip of an amount keeps its domain
and any raw arithmetic on the read-back value is rejected. Stamps are
key-literal-scoped (dynamic keys degrade to unknown; the runtime mock is
the backstop).

## What this does NOT claim

- No exact gas prediction. Real burnt gas depends on runtime state (key/value sizes, iteration counts, cross-contract callee costs). Tier B numbers below are a fitted envelope, not a guarantee.
- Cross-contract callback existence is invisible by design (we do not own the callee).
- Dynamic storage keys and dynamically chosen handles degrade to unknown and fall through to the near-mock runtime guard.

## Tier B: fitted floor vs real-VM measurement

Static floor model (1 Tgas base + ~10 Ggas per host op + attached receipt
gas), validated against `examples/ab_runner.rs` real-VM replay:

| Fixture | static floor | real VM (ab_runner) |
|---|---|---|
| g1_good (promise chain + defined callback) | ~1.2 Tgas + 10 Tgas attached | **14.43 Tgas** |
| x11 single-use dance | ~1.1 Tgas + 10 Tgas attached | **11.98 Tgas** |

The model behaves as a floor (measurement > floor, gap = runtime state
cost). The useful output is the asymptotic flag — a method whose cost is
quadratic in storage size compiles fine with 3 keys and dies at 3M; the
host-op census per method makes that visible before deploy. Fitting better
constants is a loop (measure → adjust → re-verify), not a project.

## Tests

`tests/test_resource_flow.rs` — 8 pins: arity reject, single-attach cap,
closure-sum cap, money arithmetic, truncation, bad literal, deposit
double-attach, legal green (plus the warning-path probes under
`/tmp/seal_probe/g2*`). Corpus-wide pipeline byte-identity is enforced in
`test_regression.rs`, `test_json_set.rs`, `test_u128_memory_bounds.rs`,
`test_u128_safe_arithmetic.rs` (fixtures that fail compilation are
skipped — parity is only meaningful when compilation succeeds).

Gate at landing: **180 suites / 1,983 passed / 0 failed.**
