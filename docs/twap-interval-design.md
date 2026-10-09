# TWAP execution scheduling on lisp-rlm — interval semantics design sketch

Status: DESIGN ONLY, no code. Scope decision input for the launchpad lane.
Date: 2026-10-08. Landscape as scanned 2026-10: nearly.trade, justhoot.fun,
nearfi.trade are all spot-only — scheduled execution is the open lane.

## 1. Two products often conflated

- **TWAP price reference** (Uniswap V2-style cumulative-price accumulator):
  answers "what was the average price over the last N minutes". Useful for
  oracles. Nobody in the local pad set ships this either, but it is not
  where the scheduling gap is.
- **TWAP execution** (order scheduler): answers "fill 10k NEAR into token X
  over 2 hours without one block-sized splash". This is the missing
  primitive for launchpad-scale exits and for $CROSS pool seeding.

This sketch designs **execution**. A price accumulator can ride the same
observation store later; it is out of scope here.

## 2. Architecture: lazy, permissionless `tick` — not keepers, not promise chains

NEAR has no chain-native scheduler. Three shapes exist:

| shape | trust | failure mode |
|---|---|---|
| trusted keeper cron pushes slices | keeper liveness = product liveness | keeper down → order stalls silently |
| self-scheduling promise chain (each slice's callback creates the next promise) | none | one failed/receipt-dead promise kills the whole chain; gas per hop; hard to inspect mid-flight |
| **lazy contract cursor + permissionless `tick(order_id)`** | none — whoever calls earns the fee | nobody calls → slices slip their window but order stays claimable/refundable |

The lazy shape is the only one that is permissionless-first and multi-user
by default: execution liveness is outsourced to an incentive, not to a
server. It also matches what the compiler gives us today — storage maps,
u128 chunked arithmetic, cross-contract promises, and (since `3f6b529a`)
a compile-time single-use gate that statically rejects double-consumed
promise handles across defn boundaries, which is exactly the bug class a
hand-rolled self-scheduling promise chain dies of.

**Why not the promise chain (positive statement):** NEAR promises are
move-only and single-use; a scheduler that re-arms itself through
`promise_then` needs careful per-iteration handle discipline, and a
receipt failure mid-chain orphans the tail — the user's funds sit in a
state no top-level call can reach without deep debugging. A storage
cursor survives all of that: any caller, any time, reads the cursor,
executes due slices, advances it. Worst case is slippage of schedule,
never loss of custody.

## 3. Interval semantics (the core)

### 3.1 Clock: block height, not timestamps

`interval` is a **block-height delta**. NEAR block timestamps are
validator-controlled within bounds; height is monotonic and cannot be
nudged to make a slice "due" early. At ~1s blocks, a `interval_h` of
60 ≈ one minute; users express minutes, the contract stores heights.
`start_by_h` (latest height at which the first slice must have fired)
uses the same unit, so all deadline arithmetic is integer and
overflow-free under u64 for any realistic order life.

### 3.2 Order state

```
Order {
  seller, token_in, token_out,
  total_in: u128            -- escrowed at order creation
  n_slices: u32,
  interval_h: u64,
  start_h: u64,             -- first slice due at start_h
  grace_h: u64,             -- slice k expires unfilled after
                            -- start_h + k*interval_h + grace_h
  min_out_per_slice: u128,  -- per-slice slippage guard
  min_avg_out: u128,        -- order-level guard (checked at finalize)
  fee_bps_cap: u16,         -- max executor fee the seller tolerates
  next_slice: u32,          -- cursor: next slice to execute
  filled_out: u128,         -- cumulative output received
}
```

Escrow model: `total_in` is transferred INTO the order at creation
(deposit attached to the create call). The contract owns the funds
mid-order; slices draw `slice_in = total_in / n_slices` (remainder
added to the last slice), swap, credit `token_out` to `recipient`.
This keeps every slice self-contained — no per-slice deposits from
callers (which also sidesteps the NEP-611 gas-key no-attachment limit
for fee-claiming executors).

### 3.3 Due / grace / expiry

- Slice `k` is **due** when `height >= start_h + k * interval_h`.
- It stays executable until `height > start_h + k*interval_h + grace_h`,
  then is **expired**: marked unfilled, its `slice_in` share becomes
  refundable to the seller at finalize. Schedule slippage converts to
  partial fill, never to stuck funds.
- **Catch-up policy: bounded.** One `tick` executes up to `K` due slices
  (K = 4 in v1), oldest first. Unbounded catch-up in one block defeats
  the time-weighting purpose and risks gas exhaustion; K bounds both.
  If executors keep up (the incentive makes this the common case), each
  tick fires exactly one slice and the schedule is exact.
- **Finalize** is callable once `next_slice == n_slices` or all
  remaining slices are expired: refunds expired `slice_in` sums +
  fails `min_avg_out` check → returns everything unswapped.

### 3.4 Non-goals / edge calls

- No partial-slice execution (a slice is atomic; if the pool cannot
  absorb `slice_in` under `min_out_per_slice`, the slice fails and stays
  due for a retry until grace expires — executor loses nothing but their
  time; repeated-fail is visible on-chain).
- No price accumulator in v1 (see §1).
- No order mutation after creation (cancel = finalize-equivalent that
  refunds undue slices; callable by seller only, before next due slice).

## 4. Fee composition

Three parties, one flow per slice: `slice_in → swap → slice_out`.

1. **Executor fee**: `fee_bps` of `slice_out`, taken at slice execution,
   bounded by the seller's `fee_bps_cap` (order creation rejects orders
   whose market-level default exceeds the cap). This is the incentive
   that replaces the keeper. Executor also pays the tx gas; at sane
   fee floors (≥ 5 bps on slices of meaningful size) gas (~12 Tgas per
   cross-contract swap per ab-runner smoke) is rounding error.
2. **Protocol cut**: a protocol bps of `slice_out` routed to burn, per
   the $CROSS thesis (fixed supply, 100% burn — usage must raise
   value). Burn-on-output (not burn-on-input) means the burn scales
   with realized performance, not with intent.
3. **LP / pool fees**: whatever the underlying pool charges is inherent
   in the swap output; the scheduler adds nothing on top.

Default split proposal: executor 10 bps, protocol 5 bps, both
governable per deployment; seller's `fee_bps_cap` is the safety valve.

## 5. Why lisp-rlm is the right compiler for this

- **Zero new host fns.** Everything above is expressible with storage
  maps + u128 chunked arithmetic + existing promise ops
  (house invariant: checks and design stay checker/frontend-side).
- The single-use gate (Tier 1 + mock Tier 2) statically rejects the
  classic scheduler bug — consuming one promise handle twice across
  helper defns — at compile time; tuple-carrier flow covers
  config-passed-as-list patterns.
- Money-taint + return-contract checks mean a u128 mixing bug in the
  fee math is a compile error, not a silent on-chain loss.

## 6. Open questions (for JP before any implementation)

1. `interval_h` granularity: is height-based (integer minutes ≈ 60
   heights) acceptable UX, or do orders want wall-clock deadlines for
   display? (Display-only conversion is fine; semantics stay height.)
2. Grace default: 3× interval? Order-level `grace_h` vs per-deployment
   constant?
3. `K` (max slices per tick): 4 fixed, or `min(4, due)` with gas-meter
   excuse?
4. Failed-slice retry: should a slice that fails `min_out_per_slice`
   auto-narrow (re-quote smaller) or strictly retry full size until
   grace? (v1 says strict; auto-narrow leaks MEV timing.)
5. Cancel semantics: seller cancel allowed only while no slice is due,
   or always with a cancel fee covering the executor's lost fee stream?
6. Which pool(s) may a slice route through — pinned pool at creation,
   or best-of-allowlist at execution? (Pinning is deterministic and
   MEV-auditable; routing raises output but adds trust in the router.)
