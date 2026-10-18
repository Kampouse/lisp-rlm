# CLMM v5 Auth & Upgradability Audit — 2026-10-17

Scope: `lib/clmm_pool_v5.lisp` (HEAD), entry points `_run`, `ft_on_transfer`, `pay`.
Method: manual read of every entry + predecessor/signer check. Line numbers at audit time.

## Entry points

| Export | Function | Caller gate |
|---|---|---|
| `_run` | `p5-run` | anyone (dispatch below) |
| `ft_on_transfer` | `p5-on-transfer` | predecessor MUST be 6|TOKX or 6|TOKY, else full refund ✓ |
| `pay` | `p5-pay` | predecessor MUST be self (promise callback) ✓ |

## `_run` op-by-op

| op | authority check | verdict |
|---|---|---|
| `init` | only when `6|TOKX` unset ("already-initialized" otherwise) | ✓ one-shot; but ANYONE can take the empty slot if they call first after deploy — deploy runbook must init in the same tx burst as deploy |
| `reg` | open (self-reg; who fallback = predecessor) | ✓ registration is permissionless by design; no storage stake (testnet ft2 — prod ft needs 1NEAR storage binding, NOTE) |
| `mint` (via `_run`? NO — mints ride `ft_transfer_call` → `ft_on_transfer` → `p5-mint-leg`; `_run mint` = `p5-mint` bookkeeping leg used by C2b two-leg protocol) | who = predecessor; req flow pins amounts | ✓ |
| `burn` | **who-bounds: NO authority predicate — documented as spec-parity (thy burn_rel has no authority term); burn subtracts d from LQ cells [a,b] and shrinks support.** | ⚠️ FINDING A — see below |
| `burnc` | burns position POS key and credits owed, capped at escrow float | tied to `p5-burn` authority (Finding A) |
| `claim` | who = predecessor; owed ≥ 1; REG required | ✓ |
| `req` | open quote leg of the two-leg mint | ✓ (no funds move) |
| `donate` | **open: anyone can add v to LQ at any cell i** | ⚠️ FINDING B — see below |
| `swap` (gross, `_run`) | open; requires REG + caller-side funds? — `_run swap` debits NO token: it walks books + P/S and moves s only? → see Finding C | ⚠️ FINDING C |
| `quote`/`get`/`view`/`escrow`/`state`/`fg`/`owed`/`positions`/`g` | read-only | ✓ |
| `pay` via `_run`? | not in dispatch | ✓ |

## Findings

### A — `burn` had no authority predicate (HIGH — **FIXED 2026-10-17**)
`_run {"op":"burn","d":…,"a":…,"b":…}` subtracts `d` from every LQ cell in
[a,b] with only the per-cell `LQ_i ≥ d` guard. Anyone can burn *someone
else's* liquidity (per-cell guard only needs the GLOBAL cell balance to
cover d, not the caller's own position). `burnc` then converts a position
key to owed — but burn itself already corrupts the curve: a griefer can
zero the pool's liquidity and walkers lose fees/L; LPs keep POS records but
their collateral LQ is gone. The Isabelle relation `burn_rel` intentionally
has no authority term — it is real-valued pool math (lq :: int ⇒ real, no
participants); access control is execution-layer, not venue-layer, so the
premise belongs ABOVE the relation, not inside it.

**Fix applied (this audit):** `p5-burn` now requires `POS:<caller>:<a>:<b> ≥ d`
(same check as `p5-burnc`) AND decrements the caller's POS key by `d`
(previously POS never shrank on burn, so repeat-burn and stale-position
bookkeeping were also wrong). Contract comment cites this audit. Kernel
(quote-only) untouched — no re-export needed; `burn_rel` itself is
unchanged, matching the layering. Negative tests:
`refund_reject_paths`-style probe + fuzz harness burn path now runs as the
position owner.

### B — `donate` is open (LOW-MED, design decision)
Anyone can credit LQ at any cell — including cells outside the current
support (widens IMIN/IMAX). No funds required. Griefing vector: inflate LQ
at a dead cell to widen the walk span (gas cost for walkers) or dilute fee
share. Probably acceptable (donations are gifts), but should be a conscious
yes. Cheapest hardening: require `donate` funds to arrive via
`ft_transfer_call msg=donate:<i>` so LQ only grows against escrowed tokens
(burnc's float cap already assumes this discipline).

### C — `_run swap` (gross) moves books without token escrow (MED)
The gross swap op debits no token contract — it trusts the caller sent
tokens in the SAME tx? It does not: there is no way to attach a NEP-141
transfer to `_run`. Audit of `p5-swap` (op path) shows it walks P/S and
updates 6|X/6|Y flow ledgers but touches no escrow — the book entries are
used by burns to distribute fees. If gross swap is meant as a *simulate*
op it must be view-only; if it is meant to settle trades it must be fed by
`ft_transfer_call msg=gross` like the custody path. Clarify intent before
prod; if settle-path, either wire it to ft_on_transfer or remove the `_run`
variant from prod (keep for the differential harness which uses it as the
oracle-vs-contract gross channel — but then PROD wasm should drop it).

### D — `init` squatting (LOW, runbook item)
`init` is first-caller-wins. Deploy runbook: call `init` immediately in the
same batch as `near deploy`. On mainnet use a lockup or hardcode tokens at
build time for the production wasm.

### E — fee shift mutability (INFO, verified GOOD)
`6|SHIFT` is written only by `init` (range-guarded < 41, empty/0 → 8). No
setter exists. Fee tier is immutable post-init ✓ (matches the
"shift is a parameter, never change silently" rule).

### F — `pay` gate (verified GOOD)
`p5-pay` requires predecessor == self; all state mutation is behind the
promise-result dispatch. An outside `pay` call is a no-op fail. ✓

### G — token-side checks (verified GOOD)
`ft_on_transfer` refunds unless predecessor is exactly 6|TOKX/6|TOKY ✓;
`p5-num-ok?` bounds amt to ≤40 ASCII digits (u128 + parse safety) ✓;
`p5-claim-one` requires `6|REG:<who>` before paying ✓; owed zeroed BEFORE
the payout promise (reentrancy-safe single-threaded receipts) ✓.

## Upgradability

- Contract has NO migration hook and NO pause switch. Redeploy replaces
  code, state keys persist (verified in the live redeploy 2026-10-17: state
  intact across code swap). A key-format change would orphan state — any
  storage migration needs a versioned prefix (none exists; `6|` is v6-era).
- Deploy key = whatever account holds the pool (testnet: clmm5-51fe5a).
  Prod: move to a multisig (SWEAT-style 2-of-3) before mainnet.

## Actions

1. (prod-blocker) Finding A: add authority to burn + kernel premise;
   negative test in fuzz + differential.
2. (decision) Finding B: donate funding rule.
3. (decision) Finding C: gross swap = sim-only (make it view) or wire to
   ft_on_transfer; keep `_run` variant only in the harness build.
4. (runbook) D: init-in-same-batch; document.
5. (prod) E/F/G: no action.
6. (prod) pause switch: optional `6|PAUSE` checked in on-transfer/claim;
   recommend adding at next spec revision, not hot-patch.
