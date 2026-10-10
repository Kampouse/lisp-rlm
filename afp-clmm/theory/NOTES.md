# AFP entry: Concentrated Liquidity Market Making Operations

URL: https://isa-afp.org/entries/Concentrated_Liquidity_Market_Making_Operations.html
Local full sources: `theory/` (this dir) — CLMM_Misc.thy, Grid_Information.thy,
CLMM_Description.thy (4325L), CLMM_Transformation.thy (4259L). Isabelle/HOL,
author Mnacho Echenim (Grenoble INP - UGA, Kaiko), ANR BlockFI. Imports
HOL-Analysis. ~12.5K lines total. Entry date: 2025 (check entry page).

## What it is

Machine-checked model of Uniswap-v3-style CLMMs as a function `P` over an
infinite fine grid (not per-tick storage):

- `grd P i` — grid of sqrt-prices, `strict_mono`, dense both ways (clmm_dsc)
- `lq P i` — liquidity per interval `[grd i, grd (i+1)]`
- `fee P i` — per-interval fee in [0,1)
- `quote_gross P p`, `base_net P p` — cumulative token counts reachable
  at sqrt-price `p` (the swap curve, built by summation over grid intervals)
- `quote_reach P q` — inverse: sqrt-price reached after adding q quote
- `quote_swap P pi y = base_net P pi - base_net P (quote_reach P (y + quote_gross P pi))`
  (quote in → base out); `base_swap` symmetric

## Transformations (CLMM_Transformation.thy)

- `refine P1 sqp` — sub-pool holding P1's liquidity strictly above sqp
- `slice_pool P2 sqp` — cut of P2's active interval at sqp
- `pool_join` — pointwise liquidity add on a common grid, fees joined
  (`pool_fee_join`)
- `pool_comb P1 P2 sqp = pool_join (refine P1 sqp) (slice_pool P2 sqp)`

## Headline results

- **Combination theorem**: for equal-fee pools sharing a grid, swapping `y`
  in `pool_comb P1 P2 sqp` == swapping `y1` in P1 and `y−y1` in P2, and the
  exact split is computed from the joint curve (`quote_swap_opt_above`,
  `quote_swap_opt_below` in locale `combined_pools_cst_fee`).
- **Optimality**: that split maximizes base out for given quote in
  (sub-additivity inequality proven by case analysis on sqrt-price positions).
- Finer-pool slippage INVARIANCE (corrected 2026-10-10 — previously misstated as "strictly reduces"): finer_quote_slippage / finer_base_slippage prove slippage P1 = P2 — refinement changes NOTHING about the curve. Pinned by the `slip` scenario (pb vs curve-preserving pb-ref @2e9: identical price path, outputs equal to the yocto).

## lisp-rlm porting relevance

- The math is float-free in structure but assumes REAL arithmetic (division,
  sqrt). On-chain port needs: `u128/muldiv` (256-bit intermediate) for
  `amt*L` terms, Q64.64 sqrt via `tick_to_sqrtPrice64` — see skill SKILL.md
  toolchain matrix. `quote_reach` is a binary search over the grid — fine as
  a bounded loop under PV155 gas metering.
- The optimality theorem is a ROUTING result (off-chain): it licenses a
  router/splitter contract that, given two same-fee pools, computes y1/y2
  on-chain. Near-mock differential tests can pin combination == two swaps
  exactly, mirroring the theorem statement.
- Grid-as-function model maps to lisp-rlm storage as the tick-key layout
  already used in `examples/clmm_u128.lisp` (`((tick + 50000) << 4) | field`).
- Rounding parity (round-up/down by direction) remains the hard 20% — the
  AFP entry is exact over reals; every `=` in it becomes ≤/≥ with
  ceil/floor on-chain.

## PORTED (2026-10-09): pool_combination — refine/slice on DIFFERENT grids + cst_fee
- Grids now GENUINELY different: pa@[1,2,4]e9 (liq [1e23,6e22]), pb@[1,3,4]e9
  (liq [3e22,9e22]); pd = pool_join(refine pa, refine pb) on [1,2,3,4]e9 —
  B's [1,3] cell CUT at 2e9 (refine's proportional split, exact: constants
  divide); per-cell liq = density-summed slices; per-cell gross books
  Q=ceil(L*DEN/(DEN-NUM)), phi=3e15/1e18. splt = optimal splitter.
- 7/7 scenarios GREEN through compiled WASM (first run, again):
  exact (1313e27): comb=1294327475435, split=1296010435018 (slack 1.68e9
  = per-leg floor slack), A-only=1292145922951
  offgrid (1.3e30): comb=1281655100775, split=1283305317269
  cross (3e32): comb=75000000000000 (refined walk saturates last cell),
  split BEATS it by 12474957470938; dust refunds exact at left:splt
  inside (sqp=2.5e9 in all grids): refine identity pinned — pd from 2.5e9
  = 6757173048552; pa/pb interior outs pin original pools from same point.
- OPTIMAL SPLIT ON DIFFERENT GRIDS = EQUALIZED ENDING PRICE: p* solves
  qA(p*)+qB(p*)=y piecewise over MERGED breakpoints (2e9=A tick, 3e9=B
  tick); y1=qA(p*). The shared-grid shortcut y1=qA(sqpC) is SUBOPTIMAL
  here — the opt-sweep CAUGHT it first (92 violations, beating alt ended
  both legs at the SAME price); oracle+template redesigned to zone-solve.
  After fix: 0 violations x 3001 alts/tag; sweep max-alt = y1 exactly;
  2 on-chain alt splits per tag <= splitter sum.
- Lessons: (1) template+oracle drift = paren crash — cstep template was
  hand-rewritten instead of derived from STEP, missing one close; the
  scanner desynced at a QUOTE and pointed 20 lines away; per-line depth
  trace found the real line. gen.py now verifies BALANCE BEFORE WRITING
  (bad template writes nothing). (2) magnitudes must be emitted as
  STRINGS — bare 1e23+ literals parse as variable names ("undefined
  variable '100000000000000000000000'"). (3) cb str-cat: build inside
  the f-string template with {{}} escapes, never post-hoc .replace (the
  rendered braces never match the raw-template pattern). (4) run_scen
  imports gen directly + reads pins.json — zero re-typed constants.
- LEG 3 (2026-10-09): heterogeneous fees DONE — pc (fee 5e15/1e18, grid=A's,
  liq [4e22,9e22]) + pj = pool_fee_join(pa, pc) on the shared grid.
  KEY THEOREM SHAPE: join_gross_fct makes the joint GROSS book EXACT even
  for f1!=f2 (QBK{i}=QA{i}+QC{i}, integer add) — the union fee only needs
  to reproduce that book, it is NOT in the walker. Decomposition on-chain:
  gross additive exact; net legs >= joined (union 0.357% > A 0.3%, equality
  iff equal fees). fee_union chain at FEE SCALE 1e15 (DEN-scale overflows
  u128 at l*DEN^2~1e59): Fhat within 746e-18/278e-18 of exact Fraction
  value (fee-scale floor dominates; nested floors partially cancel).
  get-fee export reproduces Fhat bit-for-bit through compiled WASM.
  width answer: __h_u128_muldiv = FULL 256-bit product (8-limb schoolbook +
  restoring division, traps d==0 | q>=2^128) — l*FS~1.3e38 intermediates
  safe; only RESULTS must fit u128.
  TRAPS HIT: (1) fee_union weights CROSS the pools — l1 pairs with (1-f2),
  l2 with (1-f1); own-complement pairing passed every structural check and
  was caught ONLY by the Fraction-vs-Fhat oracle check (err 1.1e13 e-18).
  (2) generator comprehension used stale loop var i -> fee1/QBK1 defined
  twice, fee0 never (balancer passed it: balanced != correct; compiler
  caught). Use enumerate for indexed emissions. (3) walk_swap needed an
  lpat hook alongside qpat (LK{i} vs L{i}).
- 9/9 scenarios GREEN (join: joined+legs+get-fee+alt-split; inside-join:
  interior entry pins pj/pa/pc from 2.5e9; pa's interior pin agrees across
  two independent scenario files).
- LEG 4 (2026-10-09): arbitrary N-pool composition DONE (N=3, the paper is
  pairwise-only — this leg is OUR generalization, pin+sweep verified).
  THEORY: net is CONCAVE in y (marginal rate (1-phi)/p^2 falls with price)
  => equal-fee optimum = EQUALIZE ENDING PRICE p* across pools (leg-2
  theorem, N-pool form): p* solves sum_k qgross_k(p*) = y. Plain fee-greedy
  is WRONG (first design; marginal rates cross after each pool's cell 0).
  splt3: flat unrolled 48-iter binary search (storage round-trips, no
  nesting), allocation y_k = min(cap_k, qgross_k(p*)) with SEQUENTIAL
  remaining-y; muldiv-free qg (mdiv by "1"); withheld excess when y >
  total cap (scenario 'over', refund trace via get-split).
  Pools: pa (imported liq gen.LA — NOT re-typed) + n1 [3e22,8e22] + n2
  [5e22,3e22], all grid [1,2,4]e9, fee 0.3%.
  Pins: star y=4.66e32 net=128759999998519 vs all-in-n2 32500000000000
  (composition wins 3.96x); sweep full 33^3 grid worst gap 0 (VIOLWIN 6 =
  2N floor-dust units) + 20k random splits clean.
  TRAPS: (1) plain-string alloc literal needed DOUBLE backslashes but
  SINGLE braces (f-string vs plain string escape desync — splt used {{ in
  f-string; gen3 alloc plain '...' kept }} literal). (2) chain qg true-
  branch emitted FULL cell-1 cap + partial (overcounts Q_k1*(G2-p)) —
  search collapsed to G1+1, n2 attach 0, ERR_ZERO; must mirror model's
  min(p, G2). (3) re-typed pa liquidity from memory ([4e22,9e22] vs actual
  gen.LA=[1e23,6e22]) — probe swap proved L0=1e23 exactly (dp = y/(2.5Q)).
  "never re-type constants" means IMPORT (list(gen.LA)); regen+recompile
  everything that embeds them. (4) paid: keys are PER-CONTRACT partitions —
  opt-n n1 pin must be n1_net alone, NOT cumulative across contracts
  (cumulative only valid within one contract, cf. leg-2 opt pins). (5)
  scen/<name>/state.bin persists between manual near-mock runs (loads if
  present, mod.rs:1745) — polluted my debug replays with doubled paid:
  keys; rm state.bin or use fresh dir. (6) div not a lisp builtin -> mdiv
  x "1" "2". (7) dropped (sput "sp:who" who) in first splt3 cut: callbacks
  wrote "paid:" empty-who — always carry who through storage before
  receipts. (8) n1/n2/splt3 emit target/<name>.wasm (not <dir>/<name>.wasm
  like gen.py projects) — manifest must point at the real file.
- 14/14 scenarios GREEN across 4 legs (base 9 + npool-under/two/star/over
  + opt-n). Remaining gap vs the entry: the Isabelle proofs themselves
  (pin + sweep, never prove) — PORT SURFACE COMPLETE. 

- REFINE CORRECTION (2026-10-10, from reading CLMM_Transformation.refine_lq +
  wedge in CLMM_Misc): AFP refine CARRIES L across the cut (wedge inserts the
  same lq on both sides) — the port's width-proportional L-split was a
  different object. Marginal out/y = (1-phi)/p^2 is L-independent, so
  small-trade pins passed, but spanning-cell capacity was halved: the "cross"
  differential showed comb behind the split legs by 1.25e10 (14%) — previously
  misattributed to "refined walk saturates last cell". With LJ corrected to
  [1.3e23, 9e22, 1.5e23] (refine_carry + per-cell add): exact slack 0 (was
  1.68e9), offgrid -1 (one floor unit), cross 0 (was -1.25e10). The
  combination theorem now holds to the yocto +/- floor dust. Lesson: a pin
  passing "with slack" is a hypothesis, not a proof — chase every slack.
