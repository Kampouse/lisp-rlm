# SPEC — CLMM v5 (real concentrated liquidity, DCL-v2-parity)

_Status: spec, 2026-10-07 ~00:00 ET. Owner: JP + agent. Predecessor: v4.x
ladder pool (retired as a design; harness + mock + THREAT.md carry over)._

## Why v5 exists

v4 was named CLMM but is a fixed-price ladder — 5 slots, constant price
per slot. v5 is the real thing: **Uniswap-v3-style concentrated
liquidity on NEAR, implemented in lisp-rlm**, validated by differential
testing against **DCL v2** (`dclv2.ref-labs.near`), the production
audited CLMM on NEAR mainnet.

## The model (extracted from ref-sdk + DCL views — not vibes)

- **Point** = Uniswap v3 tick, verbatim: `price(p) = 1.0001^p`
  (raw y per raw x). Range clamped to ±800000.
- **Fee tiers** (bps) → point grid delta:
  `100→1, 400→8, 2000→40, 10000→200`. Liquidity may only be *placed*
  on grid multiples; a swap *crosses* grid points as it consumes depth.
- **Liquidity net**: each initialized point stores `L_net` = Σ(liquidity
  added by ranges starting here − ranges ending here). Swap at point p
  walks: consume current segment with current L, on crossing add L_net
  of the crossed point.
- **Segment math** (the Uni-v3 core, integer form): with L and
  sqrt-price S (Q64.64 fixed point, u128), moving S → S′:
  - out-y: `dy = L·(S′ − S) >> 64`
  - in-x:  `dx = L·(S·S′⁻¹ difference form) = L·(S′ − S)/(S·S′) >> 64`
  - max input to reach S′: derived same pair of formulas.
  All division floors in the pool's favor; fees round up against the
  trader (input-side fee: `fee_amt = ceil(amount·fee_bps/10000)`).
- **Positions**: LP supplies [pa, pb] (grid-aligned) + L (or a token
  amount pair converted to max L via the segment formulas at both
  endpoints). Mint/burn/collect with O(1) fee accounting:
  `fee_growth_inside(position) = global − outside_below(pa) − above(pb)`
  stored per position; tokens owed = unclaimed growth × L + cached.

## Integer kernel (lisp surface)

- `isqrt_u128` — Newton iterations, exact floor sqrt of u128 (exists in
  python ref first, proven against 10⁶ random cases + edge powers).
- `S(p) = floor(sqrt(floor(1.0001^p · 2^64)))` — point → Q64.64
  sqrt-price. Table-free: computed by fixed-point pow via repeated
  squaring in Q64.64 (error ≤ 1 ulp; **must match the python ref
  bit-for-bit**, not just approximately — the differential dies else).
  Inverse: `p(S)` only used for views/debug.
- All arithmetic on the existing u128 limb-math stack (li-mul, li-div,
  li-add, li-sub — proven by the v4 audit arc).
- Caps: L, amounts ≤ 1e18·10^(dec) sanity-bounded like v4's numOk.

## Storage layout (pool account keys)

```
TOKX, TOKY        token pair, set at init, immutable
FEE               bps tier (one pool per fee), immutable
PD                point delta (derived, cached)
CUR_POINT         i32 current point
CUR_S             u128 Q64.64 sqrt price (S of CUR_POINT floor)
L                 u128 active liquidity
FGX, FGY          global fee growth (Q64.64 per-L accumulators)
P:<point>         net liquidity at point (i128, created on first touch;
                  deleted at zero — v4's storage-grief lesson)
POS:<owner>:<pa>:<pb>  L, feeGrowthInside snapshots, cached owed x/y
```

## Ops (surface = v4's promise-chained pattern, reused)

- `init(tokia, tokib, fee)` — one-shot.
- `add_liquidity(pa, pb, L | max_x, max_y)` via ft_transfer_call
  (both tokens in — v4 was B-only; v5 needs two-sided ranges).
- `remove_liquidity(pa, pb, L)` — pays out via promise chain,
  delta-reversal rollback exactly as v4 (the whole pattern transfers).
- `swap_x_in(amount, min_out)` / `swap_y_in(...)` — the point walk.
- `collect(pa, pb)` — fee claim.
- Views: `quote(amount, dir)` — and this must equal DCL's.

## Differential testing (the whole point)

**Comparator: DCL v2 mainnet, black-box.** No source is public
(ref-labs org empty); the audited view API is the spec:

1. **Single-segment parity (primary):** `list_pools` exposes
   `current_point`, `liquidity`, fee → state needed for any swap that
   doesn't cross a point is fully readable. Read pool → run DCL
   `quote(input=X)` vs v5 `quote` on identical state → **outputs must
   match bit-for-bit after fee accounting**. Repeat across many pools
   (400+ live) × sizes (dust → 0.9×segment capacity).
2. **Cross-point shape (secondary):** no per-point views exist
   (probed: list_point_data/list_points/point_data → MethodNotFound),
   so crossing swaps are validated by curve-shape probing: quote at
   increasing sizes reveals the exact kink points where liquidity
   changes; v5's predicted kinks/depths must match observed.
3. **Python reference (`scripts/clmm_v5_ref.py`) is the bit-exact
   oracle** the lisp must equal first (10⁶-case isqrt/pow parity,
   then full-op parity in near-mock).
4. The v4 harness carries over: I1–I7 invariants (I7 wealth
   conservation now covers both tokens), fault injection on both legs,
   rollback bit-identity, lisp↔TS parity, edge battery (empty pool,
   single-sided range, L=0 at point, position at clamp edges ±800000).

## Milestones

- **M1 (done when ref passes):** python kernel — isqrt, S(p) table-free
  pow, segment math, point-walk swap, fee growth; self-tests + float
  cross-check within 1 ulp; conservation property tests.
- **M2:** lisp kernel equals python bit-for-bit (pure functions, no
  storage) — pow/isqrt parity suite first.
- **M3:** pool storage + ops in near-mock; harness I1–I7 green.
- **M4:** live differential vs DCL mainnet (single-segment parity
  across ≥20 pools; shape probing on ≥3).
- **M5:** TS port + parity; THREAT.md update; testnet E2E deploy.

## Non-goals (unchanged from THREAT.md)

No admin, no pause, no upgrade. Tokens are trusted arg sources
(NEP-141 inherent). Not an oracle. Fee revenue to LPs only.
