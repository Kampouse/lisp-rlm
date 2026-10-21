# Launch v2 — Unified Pool ("bucket") spec

Status: DESIGN — not implemented. Supersedes the curve→graduation model
(`bond_curve_v1` + factory handoff) for the next launch generation.

## Core idea

One mode. No graduation event, no curve contract, no handoff. The launch
**is** the CLMM pool:

- **Ask side (cells above launch price P0):** the whole token supply `SUP`,
  placed at init by the creator.
- **Bid side (cells below P0):** nothing at init. Buyer QT **becomes** the
  bid stack — every buy leaves its QT behind as floor depth, and the price
  walks up. The raise and the floor are the same object.

The emission curve (the economic knob) = how many tokens sit per ask cell:
flat = linear price; denser at bottom = cheaper early entry; steeper top =
capped upside. This replaces the entire `V`/`G` curve-parameter debate with
one liquidity profile.

## What this kills

| v1 problem | v2 answer |
|---|---|
| Post-grad one-way market (no `ft_transfer_call`) | Pool is the custodian from block 0 — holders just swap |
| Graduation cliff / handoff price discontinuity | None — one book |
| Virtual reserve `V` display lie | No V. Depth is real cells |
| Deadline/refund debate | No fail state. Stuck launch = thin ladder; a "buyout" is anyone buying the range |
| Creator pulls the floor (rug) | Bids are custody, not a balance — never withdrawable |
| Redemption floor machinery | Free — the bid stack IS the redemption floor |

## Mechanics (all existing pool semantics, nothing new on the pool)

- **Measured polarity (2026-10-21, `launch_profile_matrix.py`)**: the
  pool's tick-P walks UP on toka-in. So the launched token is TOKB
  (asks at negative ticks), QT = TOKA; a QT-in buy walks P down into the
  asks (economic price up), and the QT it leaves becomes the bid stack.
- **Boot straddle is MANDATORY**: at init P sits in a liquidity gap and
  EVERY swap refunds — nothing to walk against. Init = two-sided seed
  straddle at P0 (creator QT = the initial floor) + ask segments above.
  The boot's QT side IS the launch's first floor — creator-funded, which
  collapses the "honest V" debate into a visible, real deposit.
- Buy: swap QT→token. QT fills the cells behind the walk → bid stack grows
  upward. Raise-to-date = Σ QT in bid cells (readable from custody escrow).
- Sell: swap token→QT out of the bid stack. Early sellers at P0 with no QT
  behind them hit the dead-zone clamp → refund until buys stack QT below.
  This is the honest "you're exit number N" behavior.
- Fees: no LP exists. Fee cells should compound into the bid stack
  (autocompound-to-floor — design item 7, free here). OPEN DECISION.

## Init flow (factory-orchestrated)

1. `init {tok, qt, pool, profile}` — one-shot, as today (`lf-init` shape).
2. Creator mints `SUP` to the factory (ft_transfer_call), factory holds it
   in escrow until placement completes.
3. Factory places SUP in ask cells by **profile** = list of
   `{lo, hi, tokens}` segments. Each segment = one pool mint leg — the
   existing `lf-seed`/`lf-onleg` two-leg forwarding machinery already does
   exactly this; v2 generalizes it from 1 mint to N segments (cap N at
   gas: ~2 legs per ~40T today, batched like `lf-seed` does).
   - Mint legs are single-sided (all cells above P0 = QT-side-only mint —
     the existing side-aware rule applies, range entirely above price).
4. Creator allocation: NOT in the asks. Creator share `c` of SUP is minted
   separately to the factory and vests in tranches (reuse the `lf-wd`
   tranche/lock machinery; payout = token transfer, not LP burn).
   OPEN DECISION: default `c` (0 = pure fair launch; pump.fun uses 0).
5. Pool storage top-up for N cells: sponsor pays ~1 NEAR/100KB (deploy law).

## Custody rules (real-money bar)

- Bid QT and ask token live in the pool's escrow keys — the factory holds
  NO money at rest between placement legs (same quarantine discipline as
  `6|ESCMX/Y`): unplaced SUP sits in the factory escrow only mid-placement,
  every leg confirmed by the existing `PH` phase machine.
- Creator can never withdraw the bid stack. Their only monetization is
  vested tranches + selling those into the market.
- View ops must expose: raise-to-date (Σ bids), floor depth at P, vested
  remaining — clients never scrape storage (standing rule).

## What happens to v1

- `bond_curve_v1` + the graduation path (`graduate`, `ong1/ong2`, freeze)
  stay deployed as-is for existing launches; no migration (frozen husks
  are inert and correct).
- Factory v2 = new verbs in `launch_factory_v1` (or a v2 lib) behind the
  same account pattern; `lf-init` gains `profile`; `lf-seed` generalized
  to N legs. The wd/claim/sweep plumbing is reusable as-is.

## Open decisions (Jean)

1. Fee destination: compound into bids (floor grows with usage) vs creator.
2. Creator allocation `c` default, and vesting schedule shape.
3. Profile encoding on-chain: explicit segment list vs a small family
   (flat/geo/step) with params — gas + storage per init.
4. Do we keep a minimum-raise gate at all (e.g. creator tranche unlock
   requires raise ≥ G_min)? v1 had R ≥ G; v2 has no natural gate.
