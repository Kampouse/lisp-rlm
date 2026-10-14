# Accrual mount — wiring the proven fee ledger into clmm_pool_v4

Status: GAS PASS M1a GREEN (make accrual-mount-v4: 29/29). The
PS accumulator (H1-H4) is wired into the REAL clmm_pool_v4 paths —
generator-concat (`contracts/accrual-mount/gen_accrual_v4.py`) reads the
lib fragments VERBATIM, applies anchor-checked transforms (one-shot
G/SHT/BAL/CK init in pool-init4; **M1a write-coalesce: the swap wave
never stores during the walk — pool-swap returns the wave tuple
(rem dy fee u0..u4), commits happen in pay_out success (F/G/AB/PB +
slots), pay_out fail restores pre-wave absolutes with slots untouched,
slippage/zero refunds are FREE; acm-grow is pure compute, G commits in
pay_out; settle-then-checkpoint in add-b-liq; settle-pre-burn +
owed-rides-A-leg in withdraw4; kc/bc reversal args restored by pay_b,
BAL zeroed on success**), and emits `v4-pool/src/main.built.lisp`.
lib files remain byte-identical (verified in git). The Python mirror is a
literal port of the emitted lisp — scenario numbers are single-sourced
from it and pinned bit-for-bit on-chain (G=11066195682399511 after two
waves; claims 1799/2399/1799 (+0 late-LP skim); withdraw paid 1,203,598 A incl.
claimed credit +
owed, and 4,816,941 B on real token legs; solvency 5997<=5999).
Reversal legs now pinned ON the real pool (scenario runs the fail legs
FIRST, against an empty pool token float, so the payout promises
actually trap): H1 fail-swap restores F/G/AB/PB to pre-wave absolutes
(slots were never stored — M1a) bit-exactly (post-reversal claim pays
0); pay_b fail leg restores SHT/SH/AB/PB + ck:=kc, bal:=bc. Late-LP
skim pinned (lp3 joins mid-stream, entry CK = current G, wave-2 claim
= 0 while early LPs claim 1799/2399); solvency
Σowed 5997 <= fees 5999 fail-closed. Two mount bugs found & fixed by
these pins: (1) acm-grow rode `near/store-bytes`' "" return into the
reversal args — pay_out couldn't un-grow G (phantom +wave rippled into
every later pin); grow now returns the increment explicitly. (2) pay_b
fail leg credited AB with tot_a (out_a+bc+owed) though withdraw only
debited out_a — "a" now carries out_a.

## Gas pass M1a — write-coalesce (measured, same 29-step scenario)

Pool-swap's walk ran in STORAGE (5 slot stores + F + DY/REM before the
promise). M1a moves the whole wave into registers: pool-swap returns
the tuple, ft-on-transfer4 stores NOTHING pre-promise, pay_out commits
on payout success and restores four absolutes on failure. Measured
with near-mock --trace, old wasm (65a0bcd5) vs new, identical scenario:

- swap calls: 3883→2942 G and 3863→2922 G (**−24%**, P3 verdict
  predicted ≈−25%)
- reversal swap (fail path): 5228→2117 G (**−60%**) — failed waves now
  touch nothing, which also SHRINKS the strand window by construction
- withdraws unchanged (M1b slot-packing = next lever); fixed-cost
  steps (claims/views/mints) unchanged, as expected
- scenario total: 42532→37534 G (**−11.8%**; swaps are 4 of 29 steps)

M1b candidates (unstarted): pack S0–S4 into one S key + str-slice
parse helpers (defers reads too), memoize G/F reads. residual
deposit-path stores untouched. Known residuals (stock-pool
design, inherited from v1, not mount bugs, flagged for Jean): share
pricing is donation-attackable (first-depositor inflation: donate
tokens directly to the pool, next depositor mints ~0 shares); the
final promise leg always strands on failure (topological — no
callback after the last transfer). Fixture fix
along the way: escrow-ft ft_transfer now reads NEP-141
`receiver_id` (was `to` only — pool payouts credited "" before); legacy
field kept as fallback, ft-probe/ft-refund still green. Harness leg
(make accrual-mount: 12/12) preserved.

## What exists on each side

Pool side (`lib/clmm_pool_v4.lisp`, assembled by generators):
- Swap A-side: `ft-on-transfer4` computes the fee delta
  `f = (fee-get) − f0` per successful swap (30 bps of used A), adds
  `(amt − rem)` revenue to `AB`. Slippage-refund path restores `F`
  exactly → the fee never existed.
- LP side: `SH:<a>` shares, `SHT` total; withdraw pays
  `out_A = sh·AB/SHT`, `out_B = sh·PB/SHT` with two-leg promise
  reversal (`pay_out`, `pay_b`).

Accrual side (proven, `Aggregate_Accrual.thy` + twins):
- Growth-based fold: `owed(g,c) = (g−c)·Lf(g) div 2^64`, checkpoint
  monotone (`ERR_ORDER` on `g < c`), theorem `accrue_within_charged_book`
  (owed ≤ charged, fail-closed), per-op slack ≤ 1, dust-farming
  exactness. Fixtures: `vectors-gen/accrual-vectors.json`, both twins
  assert them (lisp 16/16, ts 16/16).

## Design: growth events = swap fee deltas

The pool already mints the only thing the ledger is allowed to trust:
the per-swap fee delta `f`. So:

    growth g_new = g_old + f          (aggregate checkpoint, u128)

`Lf` is the cap law. Two candidates:

1. `Lf = sh/SHT` per-LP claim on the aggregate (proportional split at
   READ time; ledger stores one aggregate owed) — cheapest, matches
   v4's `out_A = sh·AB/SHT` payout shape.
2. Per-LP checkpoints (`a:ck:<lp>`) as in the smoke twin — exact
   while-in-capital accrual (LP entering late doesn't skim earlier
   fees). More writes.

CHOSEN: (2) for correctness — the theorems' while-in-capital discipline
is the anti-sandwich story; (1) re-opens the skim the ladder closes.
Cost: one extra muldiv + 2 writes per LP per settle, only on
deposit/withdraw/claim (NOT per swap — swaps move the aggregate only).

## Mount points (four hooks, all in ft-on-transfer4/add-b-liq/withdraw4)

H1 · swap success (`ft-on-transfer4`, after `AB` update, before the
     promise): `(accrue-grow f)` — writes
     `G` (aggregate growth checkpoint) += f and bumps the per-cell
     charged book. Reversal-safe: the slippage path restores `F`; H1
     must ride the SAME branch (only the non-refund path grows G).
     ~3 writes/swap total (G, cell-charge, none else).

H2 · LP deposit (`add-b-liq`, after `SH` mint): write
     `a:ck:<lp>` = G (fresh LP starts at current growth; zero owed
     until new fees land). +1 write/deposit.

H3 · settle (new view+mutate `accrue-claim`): owed =
     `(G − a:ck:<lp>)·sh/SHT` via the twin's div-floor64 (muldiv by
     2^64 not needed at pool scale — the twin's fixture table still
     pins the arithmetic); credit `a:bal:<lp>` += owed, checkpoint
     := G. Called by `withdraw4` BEFORE share burn (owed computed on
     pre-burn shares) and by an explicit claim method.

H4 · withdrawal (`withdraw4`, payout legs): add `a:bal:<lp>` to
     `out_a` (fee side rides the same A-payout promise); delete
     `a:bal:`/`a:ck:` keys when shares hit zero (storage rent).
     Reversal paths (`pay_out`/`pay_b`): owed credits MUST be in the
     reversal delta set — a reverted withdraw un-credits the balance.

## Scenario plan (fixture-wired like the smoke)

1. deposit → deposit → swap: LP1 owes nothing pre-swap; post-swap
   owed split = fee·sh_i/SHT per LP, asserted against hand-run
   fixture rows appended to accrual-vectors.json (regenerated by the
   same runner — no hand numbers).
2. late LP skim attack: LP2 deposits after the fee, claims → owed
   covers only post-entry growth (the skim pays 0 — THE theorem).
3. sandwich replay: swap in/out around static book → G grows 2f,
   attacker's pool P&L ≤ 0 − 2f; LPs' owed +2f. Ledger net-zero for
   the attacker, positive for LPs.
4. ERR_ORDER guard: replaying an old g traps (checkpoint monotone).

## Non-goals / sequencing

- No u128-native chain work here (separate TASK-u128-native-chain.md).
- Gas: H1 adds ~3 writes/swap — fold into the P3 write-coalesce pass,
  not before.
- The pool fragment is assembled by generators; H1–H4 go into the
  generator template + a new lib fragment `clmm_accrual_mount.lisp`,
  never hand-edited into v4.lisp.
