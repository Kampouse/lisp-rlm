# CLMM v5 M3 — The Pool: Design Notes

Status: design as-built for `lib/clmm_pool_v5.lisp` + `scripts/clmm_v5_pool_test.py`.
Oracle hierarchy: **formulas** = `scripts/clmm_v5_ref.py` + M2 kernel (parity proven,
commit e720b148). The ref's `Pool` quote *class* is NOT a settle oracle — see
"Crossing convention" below for the one place it is provably wrong.

## Model

Concentrated liquidity, DCL-v2 / Uni-v3 semantics on the integer grid:

- `price(p) = 1.0001^p`, `S(p) = floor(sqrt(floor(1.0001^p · 2^128)))` — Q64.64.
  Computed through the M2 kernel (`v5-op-spow`, string in/out). `|p| ≤ 800000`
  hard-gated: points outside trap (`panic`), mirroring the ref's PMAX.
- Point spacing `d ∈ {1,8,40,200}` (fixed at init alongside fee tier; 100→1,
  400→8, 2000→40, 10000→200). All range bounds are multiples of `d`.
- Liquidity `L` (u128) active on segment `[g, g+d)`; sqrt price `S` may sit
  strictly between `S(g)` and `S(g+d)`.
- Fee on input, ceil: `v5-op-fee(amt, bps)`; fee tier `bps` at init.
- Segment math (all through M2 kernel, string forms):
  - up (x in, y out): `dy = (L·(S2−S))>>64` [`v5-op-dy`], `dx = L·(S2−S)//(S·S2)` [`v5-op-dx`]
  - down (y in, x out): `dx = L·(S−S2)//(S·S2)` [`v5-op-dxo`], `dy = (L·(S−S2))>>64` [`v5-op-dyi`]
  - max-input solvers: `v5-op-sfdx / sfdxo / sfdy / sfdyi` (exact ref forms)

## Storage layout (prefix `5|` — no v4 key collisions; v4 used bare keys)

All values decimal strings; missing key = `0` for numeric reads (`v5-bz`).

| key | meaning |
|---|---|
| `5\|INIT` | init marker |
| `5\|TOKX` / `5\|TOKY` | token accounts (X = "A" side, Y = "B" side) |
| `5\|FEE`, `5\|PD` | fee bps, point delta d |
| `5\|P` | current grid point (segment start) |
| `5\|S` | current sqrt price, Q64.64, `S(P) ≤ S < S(P+d)` |
| `5\|L` | active liquidity on current segment |
| `5\|NET:<p>` | signed net liquidity delta at grid point p |
| `5\|POS:<who>:<pl>:<pr>` | position `<L>\|<ox>\|<oy>\|<gx>\|<gy>`: liquidity, owed fees x/y, fee-growth snapshots |
| `5\|FGX`, `5\|FGY` | global fee growth per unit L (Q128-scaled: `fg += fee·2^128 // L_attribution`) |
| `5\|X`, `5\|Y` | book of pool-held X / Y (mirror of token balances) |
| `5\|PEND:<who>` | in-flight two-leg add: `<pl>\|<pr>\|<L>\|<x_pull>\|<y_used>` |

Notes:
- `NET:<p>` may be negative (upper bound of a range) — I4 is restated, not
  violated (see invariants).
- `FGX/FGY` can exceed 2^128 (they are `Σ fee·2^128//L`, unbounded above);
  all FG arithmetic uses arbitrary-precision string helpers (`v5-li-add/-sub`),
  NOT the kernel's fixed-width buffers. The `2^128` constant as decimal string
  lives in the pool file.
- No `storage_iter` on NEAR or the mock (deprecated + removed). Next-point
  search is a d-grid walk reading `5|NET:<p+d>`, `5|NET:<p+2d>`, … capped at
  MAXWALK = 4096 probe steps per direction per swap; exceeding the cap ends
  the walk (partial fill + refund of the remainder), never a mid-mutation trap.
  Real deployments let gas be the cap; the driver generates ranges within
  ±512·d of the active window so MAXWALK is a safety net, not a steering wheel.

## Liquidity / share accounting

Positions are the shares. A position is `(who, pl, pr) → L` plus fee state.
There is no global SHT. Adding to an existing (who, pl, pr) first **collects**:
`ox += L·(FGX−gx)//2^128`, `oy += …`, then `gx←FGX, gy←FGY`, then `L += L_new`
(exact integer arithmetic throughout; the only rounding is the //2^128 in
collection — dust strictly favors the pool).

`L(g) = Σ_{q ≤ g} NET[q]` — active liquidity on segment `[g, g+d)`.

## Fee growth policy (simplification, documented)

Fees are charged in full up front (`fee = ceil(amt·bps/10000)` on the input)
and attributed as one growth step to the L active at swap start:
`FGX += fee·2^128 // L0` (or FGY for y-side input). Per-segment attribution
(Uni-v3 style) is deferred to M4 if the DCL parity work demands it.
Consequence: on partial fills the fee on the refunded remainder is kept by
the pool (trader loses it). Conservative for the pool, exact for books.
`L = 0` swaps are a full no-op refund instead (see below), so no
division-by-zero growth ever runs.

## Crossing convention (deliberate divergence from the ref Pool class)

- Cross UP into boundary point `b`: `L += NET[b]`, `P = b`.
- Cross DOWN, leaving segment `[g, g+d)`: `L -= NET[g]`, `P = g−d`.
  (Down-crossing subtracts the NET at the segment start being left.)

The ref's `quote_y_in` does `L += net[next_below]`, which double-counts the
lower bound of the segment being exited; its self-tests never re-cross a bound
after a down-cross, so it never shows. The v5 pool walk implements the
self-consistent convention above; M4's real-contract parity work will validate
it against `dclv2.ref-labs.near` quotes.

Loop safety: `L > 0 ⟹ ∃ NET point above P and at/below P` (every active
position contributes `NET[pr] < 0` above and `NET[pl] > 0` at/below), so a
walk with `L > 0` always has a boundary and terminates. `L = 0` never walks.

## Swap settle policy (vs ref quote semantics)

- `L = 0` at swap entry → **full refund**, zero state writes (no-op rollback,
  I5 path). The ref quote consumes the fee and returns 0 out; a settle
  contract that keeps the fee while price cannot move would be a theft bug.
- Partial fill (walk exhausted: no further NET boundary, or MAXWALK hit):
  refund the unconsumed post-fee remainder `rem`; fee is *not* prorated
  (documented above).
- min-out (`msg "swap:<minout>"`): if `out < minout`, restore the full
  pre-swap state (P, S, L, FGX, FGY, X, Y — all linear string fields) and
  refund everything (v4's guard discipline).
- Payout leg: `ft_transfer` of the out-token to the trader via promise +
  `pay_out`-style callback; on transfer failure reverse this swap's deltas
  and refund the input (v4 pattern, exact).
- `ft_on_transfer` return = unused input (refund to the token).

## Entry API (v4 conventions: msg dispatch inside ft_on_transfer)

- `init` (view-style method, signer = deployer): one-shot; sets TOKX/TOKY,
  FEE, PD, P0, S = S(P0), L = X = Y = FGX = FGY = 0. Second init rejected.
- `ft_on_transfer` (predecessor = TOKX):
  - `swap:<minout>` — sell X for Y (walk up)
  - `add:<pl>:<pr>` — add liquidity paying X only: range entirely above
    (`pl > P`) is single-leg; straddle (`pl ≤ P < pr`) is two-leg: this X
    legs first, the pool pulls the Y side via `TOKY.ft_transfer_call`
    (msg `pull:<who>`), commit happens in the pull receipt.
- `ft_on_transfer` (predecessor = TOKY): mirror (`swap:` walk down;
  `add:` pure-Y single-leg if `pr ≤ P`, else two-leg pulling X).
- `remove_liq` (direct method, signer = LP): burns position liquidity,
  collects owed fees, pays out x and/or y (two-leg v4-withdraw pattern with
  delta reversal on leg failure).
- Internal callbacks (self-call guarded): `add_cb` (pull resolution),
  `pay_out` (swap payout resolution), `pay_rm` (remove payout resolution).

Liquidity amounts from token amounts (all exact via u128-muldiv chains):

- range above (`pl > P`), pay x: `L = x·S(pl)·S(pr) // (S(pr)−S(pl))`
- range below (`pr ≤ P`), pay y: `L = y·2^64 // (S(pr)−S(pl))`
- straddle at current S, pay y first: `L = y·2^64 // (S−S(pl))`,
  `x_pull = L·(S(pr)−S) // (S·S(pr))`; pay x first is the mirror.
- Position value accounting on remove mirrors these forms at the then-current
  S (three cases: below / straddle / above).

Dust handling: if the computed `L` floors to 0 → full refund, no writes
(no-op, I5). Two-leg adds where the pulled side floors to 0 commit
single-leg. PEND is keyed by sender; concurrent adds by one sender are
sequential-receipt in the mock (driver uses 3 LPs, one in-flight add each) —
a real deployment needs per-receipt nonces (documented limitation).

## Books

`5|X` / `5|Y` track exactly what the pool custody-account holds: += consumed
input (fee included), −= payouts. After every op (all promise legs settle
inside one near-mock `cross` invocation — v4 harness relies on the same),
`book == token balance` must hold (I1/I3 adapted).

## Invariants, adapted (I1–I7 → concentrated model)

- **I1 (liability book == held)**: `bal_TOKX(pool) == X_book` and
  `bal_TOKY(pool) == Y_book` after every op.
- **I2 (share conservation)**: for every grid point q:
  `Σ L of positions with pl ≤ q < pr == Σ_{g ≤ q} NET[g]` — i.e. the
  position set and the NET map describe the same liquidity. (Global SHT
  doesn't exist; this is its concentrated replacement.)
- **I3 (fee accrual)**: books move only by consumed input (fee included) and
  payouts; `X+Y` book deltas per swap equal `consumed_in − paid_out`; fee
  growth `FG` deltas equal `fee·2^128 // L0` exactly (attribution policy).
- **I4 (non-negativity, restated)**: `S, L, X, Y, FGX, FGY ≥ 0`; position
  `L, ox, oy ≥ 0`; NET entries are signed but bounded: active `L ≥ 0` after
  every op, and `Σ_{g ≤ q} NET[g] ≥ 0` for all q (no negative liquidity
  anywhere on the grid).
- **I5 (no-op rollback)**: any rejected/refunded/fault-reversed op leaves
  pool storage BIT-IDENTICAL to before (driver: full storage snapshot diff).
- **I6 (liquidity bound, restated from "ladder-liquidity ≤ B")**: when
  `L > 0`: `Y_book ≥ (L·S)>>64` and, with `S_t` = S at the next NET boundary
  above P (exists by the loop-safety argument), `X_book ≥ L·(S_t−S)//(S·S_t)`.
  That is: holdings cover the virtual reserves implied by active liquidity —
  solvency at the current price. (The classic bound `X ≥ L/S` is the
  S_t→∞ relaxation; we check the tighter per-boundary form.)
- **I7 (wealth conservation)**: per token, `Σ user balances + X_book/Y_book
  (+ in-flight refunds)` changes only via owner mints. Nothing burns, nothing
  mints inside the system.

Any invariant needing a tolerance to pass = design conversation, not a tweak.

## Driver (`scripts/clmm_v5_pool_test.py`)

- Build: concat `lib/limb_math.lisp` + `lib/clmm_v5_kernel.lisp` +
  `lib/clmm_pool_v5.lisp` (pool file carries its own string↔buffer adapters
  and dispatch; the M2 dispatch tail is the parity harness, not the pool).
- near-mock `cross` with ft2 tokens (fault-toggleable), pool in manifest.
- Property battery: deterministic PRNG (seed printed), random
  init/add/remove/swap interleavings over 3 LPs; **all seven invariants
  checked after every op**; first failure dumps op, full storage state, and
  the violated invariant. ≥10,000 ops, wall < 15 min, gas per swap reported.
- Edge battery (`--edge`): empty-pool swap, single-point range
  ([pl, pl+d)), full-range removal back to zero state, fee=0 tier path,
  extreme-point init (P0 near ±800000, max-width range), dust add (zero-L
  no-op), MAXWALK partial fill, min-out rollback, payout-fault rollback.
- Regression gates: `python3 scripts/clmm_v5_parity.py` (full) PASS=1764;
  `cargo test` green.

## Open questions deferred to M4+

- Per-segment fee attribution (currently one growth step at entry L0).
- Concurrent PEND per sender (nonce keying) for real deployment.
- Fee-on-refunded-remainder on partial fills (kept by pool today).
- Ref `Pool` class down-crossing bug — fix upstream or fork the oracle.
- Real DCL-v2 parity quotes (`scripts/dcl-price.py`) on readable states.
