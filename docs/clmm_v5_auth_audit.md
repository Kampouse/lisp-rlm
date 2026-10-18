# CLMM v5 Auth & Upgradability Audit — 2026-10-17

Scope: `lib/clmm_pool_v5.lisp` (HEAD), entry points `exec`, `ft_on_transfer`,
`pay` (chain-imposed) + 12 verb exports (below). ABI redesign 2026-10-18.
Method: manual read of every entry + predecessor/signer check. Line numbers at audit time.

## Entry points (post-ABI 2026-10-18)

| Export | Function | Caller gate |
|---|---|---|
| `exec` | `p5-exec` | anyone, but gated on `6|SIMSWAP=1` → returns `"sim-only"` otherwise (sim/harness door — never written by any live path) |
| `ft_on_transfer` | `p5-on-transfer` | predecessor MUST be 6|TOKX or 6|TOKY, else full refund ✓ |
| `pay` | `p5-pay` | predecessor MUST be self (promise callback) ✓ |
| `init` `burn` `collect` `register` `position_requirements` | verb forms of the mutating ops | same authority checks as below, no SIMSWAP gate |
| `quote` `get_state` `get_positions` `get_owed` `get_escrow` `get_fg` `grid` | read-only verbs | open ✓ |

## On-transfer msg ABI

`ft_transfer_call` nests the user msg: ft2 splices it RAW into the pool's
`ft_on_transfer` input as the `"msg"` field, so the pool parses
`(json-get-str <k> (json-get-str "msg" (near/input)))`. Actions
(nested inside `"msg"`): `{"a":"seed"}`, `{"a":"swap"}`,
`{"a":"mint","d":…,"lo":…,"hi":…}`, `{"a":"cancel"}`,
`{"a":"donate","i":…}`. Garbage ⇒ full refund, zero state change.

## `exec` op-by-op (sim door — all ops behind the `6|SIMSWAP` gate)

| op | authority check | verdict |
|---|---|---|
| `init` | only when `6|TOKX` unset ("already-initialized" otherwise) | ✓ one-shot; but ANYONE can take the empty slot if they call first after deploy — deploy runbook must init in the same tx burst as deploy |
| `reg` | open (self-reg; who fallback = predecessor) | ✓ registration is permissionless by design; no storage stake (testnet ft2 — prod ft needs 1NEAR storage binding, NOTE) |
| `mint` (via `exec`? NO — mints ride `ft_transfer_call` → `ft_on_transfer` → `p5-mint-leg`; `exec mint` = `p5-mint` bookkeeping leg used by C2b two-leg protocol) | who = predecessor; req flow pins amounts | ✓ |
| `burn` | **who-bounds: NO authority predicate — documented as spec-parity (thy burn_rel has no authority term); burn subtracts d from LQ cells [a,b] and shrinks support.** | ⚠️ FINDING A — see below |
| `burnc` | burns position POS key and credits owed; owed-total solvency guard (`esc + book` per side, `6|OWTX/6|OWTY`, `ERR_FLOAT` — 2026-10-18) | tied to `p5-burn` authority (Finding A) |
| `claim` | who = predecessor; owed ≥ 1; REG required | ✓ |
| `req` | open quote leg of the two-leg mint | ✓ (no funds move) |
| `donate` | **open: anyone can add v to LQ at any cell i** | ⚠️ FINDING B — see below |
| `swap` (gross, `exec`) | open; requires REG + caller-side funds? — `exec swap` debits NO token: it walks books + P/S and moves s only? → see Finding C | ⚠️ FINDING C |
| `quote`/`get`/`view`/`escrow`/`state`/`fg`/`owed`/`positions`/`g` | read-only | ✓ |
| `pay` via `exec`? | not in dispatch | ✓ |

## Findings

### A — `burn` had no authority predicate (HIGH — **FIXED 2026-10-17**)
`exec {"op":"burn","d":…,"a":…,"b":…}` subtracts `d` from every LQ cell in
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

### B — `donate` was open (LOW-MED — RESOLVED 2026-10-18)
Anyone can credit LQ at any cell — including cells outside the current
support (widens IMIN/IMAX). No funds required. Griefing vector: inflate LQ
at a dead cell to widen the walk span (gas cost for walkers) or dilute fee
share. Probably acceptable (donations are gifts), but should be a conscious
yes. Cheapest hardening: require `donate` funds to arrive via
`ft_transfer_call msg=donate:<i>` so LQ only grows against escrowed tokens
(burnc's owed-total solvency guard, 2026-10-18: owed can never stack past the pool's per-side holdings (esc float + input book — a swap out-leg moves X float→book), so a registered claim cannot trap underfunded; the old req-cap silently shorted LPs on exactly that case).

**Resolution (2026-10-18): the `exec` donate arm is DELETED.** Donations
ride `ft_on_transfer` with nested `msg={"a":"donate","i":…}` (`p5-donate-leg`): predecessor
must be a pool token, amount is the donation, garbage/unparseable/zero ⇒
full refund with zero state change — LQ only grows against escrowed
tokens. The dispatch answers `unknown-op` for `exec {"op":"donate"}`
(pinned by the Python edge battery).

### C — `exec swap` (gross) moved books without token escrow (MED — RESOLVED 2026-10-18)
The gross swap op debits no token contract — it trusts the caller sent
tokens in the SAME tx? It does not: there is no way to attach a NEP-141
transfer to `exec`. Audit of `p5-swap` (op path) shows it walks P/S and
updates 6|X/6|Y flow ledgers but touches no escrow — the book entries are
used by burns to distribute fees. If gross swap is meant as a *simulate*
op it must be view-only; if it is meant to settle trades it must be fed by
`ft_transfer_call msg=gross` like the custody path. Clarify intent before
prod; if settle-path, either wire it to ft_on_transfer or remove the `exec`
variant from prod (keep for the differential harness which uses it as the
oracle-vs-contract gross channel — but then PROD wasm should drop it).

**Resolution (2026-10-18): runtime-gated sim-only.** The gross arm fires
ONLY when storage key `6|SIMSWAP` = "1"; otherwise it returns
`ERR_SIM_ONLY` with zero state change. `init` never writes that key and
no op can write it, so on any live deploy the arm is permanently dead —
near-mock drivers (differential, edge battery) seed it via `state
import` and keep the oracle channel. Negative tests pin the gate: the
fuzz + invariants suites and the Python battery all assert `ERR_SIM_ONLY`
BEFORE arming.

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
2. RESOLVED Finding B (2026-10-18): donate = `ft_transfer_call
   nested msg `{"a":"donate","i":…}` only; `exec` arm deleted.
3. RESOLVED Finding C (2026-10-18): gross `exec swap` runtime-gated on
   6|SIMSWAP (never set in prod); sim harness arms via state import.
4. (runbook) D: init-in-same-batch; document.
5. (prod) E/F/G: no action.
6. (prod) pause switch: optional `6|PAUSE` checked in on-transfer/claim;
   recommend adding at next spec revision, not hot-patch.
