# TASK-pool-v3 — GC26 launchpool v3: gate + flip tax + mcap tax + seniority pot + sell-gap fix

## Context
JP's launchpad (factory `gc26fund.testnet` + shared pool `gcpool26.testnet`) was live-tested today;
source is NOT on this machine. v3 is a fresh build of the POOL only (factory unchanged) from the
observed ABI below. Written in the repo's Lisp dialect, compiled via `near-compile`.

## Hard fences
- New files ONLY: `examples/pool_v3.lisp`, `pool-v3/` scaffold, `tests/test_pool_v3.rs`.
- READ-ONLY: `data/**`, `scripts/rlm-tasks/**`, everything else pre-existing.
- No git commits. No deploys (owner live-tests himself).

## Observed ABI (from today's live txs — match exactly)
- Single shared pool, multi-token: pools keyed by token account id (string).
- `buy` (payable): args `{"token": "<acct>", "min_out": "<u128 string>"}`; attach = NEAR in;
  constant-product x*y=k, ZERO swap fee, `ERR_ZERO` on 0/near-0 amounts, `ERR_NO_POOL` on unknown
  token, `ERR_SLIP` (or similar panic) when out < min_out. Tokens out via `ft_transfer` to
  `predecessor-account-id`.
- `quote_buy` (view): same args, returns JSON `{near_in, tokens_out}` or `null` when no pool.
- Sell: NEP-141 `ft_on_transfer` (pool registered as receiver; user does `ft_transfer_call`):
  tokens in → NEAR out to sender (promise transfer), unused tokens returned via the callback's
  return string (amount to keep as u128 string; "0" = keep none).
- `get_pool` (view): `{"token": ...}` → `{"near": "<u128>", "tokens": "<u128>"}` or null.
- Launch seeding: `pool_init`/`launch` payable registers a new pool (attached NEAR = initial
  reserve; initial tokens arrive via an `ft_transfer_call` from the launcher with memo `seed` —
  if tokens arrive before init, hold them in escrow key until init, then credit).

## v3 new mechanics (the point of this task)
All constants defined at top of file. Integer math ONLY (u128; see `examples/clmm_u128.lisp` for
the u128 builtin surface). Rounding always toward pool safety.

1. **Launch gate (buy timeout).** `gate_until_ts` set at init = `block_ts + GATE_MS` (default
   300_000 ms). `buy` reverts `ERR_EARLY` while `block_ts < gate_until_ts`. Sells NOT gated.
2. **Flip tax (time-decaying, sells only).** Pool records `last_buy_ts[account]` (per token pool)
   on every buy. On sell: `t = now - last_buy_ts` (no record → treat t=0, i.e. MAX bracket —
   inverted default). `flip_tax = FLIP_MAX_BP * max(0, 1 - t/FLIP_T)` with FLIP_MAX_BP=5000 (50%),
   FLIP_T=600_000 ms. Tax taken from NEAR proceeds.
3. **Mcap tax (ratio tiers, sells only).** Pool records `reserve_at_buy[account]` on every buy.
   On sell: `ratio = reserve_now / reserve_at_buy` (no record → treat as 10x+ bracket).
   Tiers: ratio<2x → 0; 2–5x → 1000 bp; 5–10x → 2500 bp; ≥10x → 4000 bp.
   **Effective sell tax = max(flip_tax, mcap_tax)** (bp of proceeds).
4. **Tax split.** `DEPTH_SPLIT_BP`=5000 → 50% of tax stays in the NEAR reserve (just don't pay it
   out — accounting honest), 50% → `seniority_pot[token]` BUT only when the pool was initialized
   with `seniority: true` (launch arg, default false → 100% to depth).
5. **Seniority pot claim.** `claim_seniority` (per token): pot share = `pot × w[me] / Σw` where
   `w[acct] = k / reserve_at_buy[acct]` (u128-friendly: store weight numerator at buy time as
   `1e24 / reserve_at_buy`); claimants = accounts with recorded buys (lazy — only they can claim,
   one-shot per payout round; after a claim zero their weight, decrement Σw). Payout via promise
   transfer. Views: `get_taxes(token)` → `{flip_bp, mcap_bp, pot, weights_sum}`.
6. **Sell-gap fix (GAPS.md #1).** On any sell where computed NEAR proceeds would leave a token
   surplus in the pool (dust, partial-keep, min-out revert), return the surplus tokens to the
   seller in the SAME callback (keep-string / ft_transfer promise), never strand them.

## Tests (machine-verified before reporting)
`tests/test_pool_v3.rs` — interp + wasm (mock hosts) both:
- [ ] constant-product buy/sell exactness incl. rounding-toward-pool on both sides
- [ ] gate: buy before/after boundary; sell allowed during gate
- [ ] flip tax: t=0 → 50%; t=300s → 25%; t≥600s → 0; NO buy record → 50% (inverted default)
- [ ] mcap tax: ratio 1.9x→0, 2x→10%, 4.9x→10%, 5x→25%, 9.9x→25%, 10x→40%; no record → 40%
- [ ] max() composition: t=60s & ratio=12x → max(45%, 40%) = 45%
- [ ] tax split accounting: proceeds + depth + pot + refunds == inputs (Σ-invariant, to the yocto)
- [ ] seniority: two buyers (early low-reserve, late high-reserve) → weight ratio ≈ reserve
      ratio; claim pays pot×w/Σw; second claim pays 0
- [ ] sell-gap: partial-keep returns surplus tokens; min-out revert returns full amount
- [ ] NEP-141 receiver: return string is exact u128 keep-amount in all paths
- [ ] full suite green + `near-compile build pool-v3` clean, wasm < 40KB

## Landmines (from GAPS.md — respect ALL)
No closures; `recur` tail-position only; numeric 0 TRUTHY / Float(0.0) falsy; no lambdas (T4);
loop accumulators in loop bindings only; let-bind before `near/return*` (double-eval bug); use
`near/storage_write` family (str→str), never bare storage-remove.

## Report
Per-file summary, test tails (all green), wasm size, any ABI guess you had to make (flag clearly —
owner tested the real v2 live and will diff), and the exact init args JSON for live-testing.
