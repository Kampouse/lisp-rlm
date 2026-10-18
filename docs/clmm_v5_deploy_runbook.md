# CLMM v5 Deploy Runbook

Wasm: `lib/{limb_math,clmm_v5_kernel,clmm_pool_v5}.lisp` concatenated →
`near-compile` (single-file only — near-compile compiles the LAST arg;
concatenate via script, never multi-arg).

## Local build

    cat lib/limb_math.lisp lib/clmm_v5_kernel.lisp lib/clmm_pool_v5.lisp > /tmp/pool_concat.lisp
    ~/dev/lisp-rlm/target/release/near-compile /tmp/pool_concat.lisp /tmp/pool.wasm
    # expect ✅ in output, exit 0. Never mask a nonzero exit.

FT: `cat lib/limb_math.lisp lib/clmm_core.lisp lib/ft2.lisp` → same.

## Testnet deploy (current: pool3.clmm5-51fe5a.testnet)

2026-10-18 pool reset: fresh `pool3.clmm5-51fe5a.testnet` on the post-fix
wasm. Birth state all-zero; seeded 5e25 X/Y; minted d=1e26 on [-8,8]
(rx 4601 / ry 84997877655924663434914); smoke swap 1e21 X -> used
3906250000000002433, and 6|ESC == ft_balance_of(pool3) EXACTLY on both
sides after the partial refund (ledger == custody from birth).

Tombstones (do not reuse):
- pool.clmm5-51fe5a.testnet - drifted ledger (X +2.5e19, Y +1.97e21
  phantom), carries ~1e29 unclaimable tokens (owed 0). `near delete` is
  rejected by testnet (ActionError kind {account_id}); state archived at
  /tmp/old_pool_state.txt, phantom numbers in the note below.
- pool2.clmm5-51fe5a.testnet - pre-redeploy staging CLONE of the old pool:
  same 5e28 escrow values baked into storage. Deploying over it does NOT
  clear stale 6| keys - init leaves ESCX/ESCY polluted. ALWAYS check raw
  view_state (prefix 6|) before reusing an account for a fresh pool.

    # 1. build (above)
    # 2. deploy — near CLI asks confirmation; echo y | it
    echo y | near deploy pool.clmm5-51fe5a.testnet /tmp/pool.wasm --networkId testnet
    # 3. verify state survived (state persists across code swap; keys 6|*)
    curl -s -X POST https://rpc.testnet.near.org -H 'Content-Type: application/json' \
      -d '{"jsonrpc":"2.0","id":1,"method":"query","params":{"request_type":"call_function",
           "account_id":"pool.clmm5-51fe5a.testnet","method_name":"get_state",
           "args_base64":"'"$(printf '{}' | base64)"'","finality":"final"}}'

## ABI (2026-10-18 redesign — matches this wasm, not the currently deployed one)

15 exports: `exec` (SIMSWAP-gated sim door), `ft_on_transfer`, `pay`
(chain-imposed), `init` `burn` `collect` `register`
`position_requirements` (mutating verbs), `quote` `get_state`
`get_positions` `get_owed` `get_escrow` `get_fg` `grid` (read-only).

`ft_transfer_call` msgs are JSON nested inside the `"msg"` field (ft2
splices raw): `{"a":"seed"}`, `{"a":"swap"}`, `{"a":"mint","d":…,"lo":…,"hi":…}`,
`{"a":"cancel"}`, `{"a":"donate","i":…}`. Garbage => full refund.

Build: trio concat -> 181,588 bytes. Suites: 8/8 Rust (12 tests) +
oracle seeds 1/2/3/16/34 clean (2026-10-18).

## Init policy (Finding D)

`init` is first-caller-wins. After a FRESH deploy to a NEW account, run
`init` in the same transaction batch as deploy:

    near tx add ... deploy ... function-call init ...

Never leave a fresh account un-inited.

## Live-state note (2026-10-18, ledger-drift audit)

The pre-fix contract committed `esc -= c` while ft2's resolve debited the
pool's token balance by the refund `rm`: every partial-refund swap drifted
6|ESC upward by `rm - c`. The live pool carries that historical drift:
X +24,999,385,093,740,232,477,352 (0.00005% of esc), Y
+1,967,122,200,601,467,270,311,285,842 (3.75% of esc). Accepted (testnet;
the stock guard is lenient by exactly that much). Post-fix legs add zero
new drift - verified on-chain: leg ESCX delta == used exactly, drift
bit-identical before/after (b535cae4). The Isabelle machine's ESwap models
the full-keep case only; extending it with a refund step is follow-up.

ft2's ft_transfer_call attaches 250 Tgas to receiver.ft_on_transfer: 100T
was too tight for the ABI-era on_transfer walk (burned its whole budget,
receipt reverted). Compiler static gate: one call closure's gas literals
cap at 300 Tgas (250T attach + 30T resolve fits; 300T does not). Unused
attach self-refunds.

## Swap msg: min-out guard (2026-10-18, commit 38a8cfa5, tx 9G92swR5 redeploy)

Swap msgs take optional `"mo":"<amount>"`: after the walk, `out < mo`
⇒ ERR_MO — p5-fail aborts the whole receipt (escrow, books, P/S AND the
fee slice all revert; ft2 resolve sees empty result ⇒ full refund).
Absent/"0" mo = pre-mo behavior byte-identical. mo compared limb-exact
(li-cmp); canonical decimal expected. Verified on-chain pool3:
mo=exact+1 ⇒ state byte-identical after (tx GRV2M1C6…); mo=exact ⇒
commits (tx HeFNWBhj…); ESC==ft_balance_of holds after both.

Range note: pool3 X-side range is EXHAUSTED (live swap pushed P to
top, IMAX+1). Up-swaps (X-in) now refund-all by design — degenerate
revert branch. Trade the Y side or mint a position above the current
span to revive X-in.

## Prod checklist (pre-mainnet)

- INVARIANT: no code path may ever WRITE 6|SIMSWAP. The exec sim arms
  (mint/burn unescrowed) are dead on any live chain because only the
  near-mock `state import` seeds that key. Do not remove the arms
  (shipped wasm must equal the tested/suites/parity wasm); do not add
  a writer (would open an unescrowed token path on mainnet).

## Prod checklist (pre-mainnet)

- [ ] Finding A fixed: burn authority (+ kernel premise + negative tests)
- [x] Finding B resolved: donate = `ft_transfer_call` nested
      `msg={"a":"donate","i":…}` only; no exec donate arm (2026-10-18)
- [x] Finding C resolved: the whole `exec` dispatcher gated on `6|SIMSWAP`
      — never written by any live path; sim drivers arm it via near-mock
      `state import` (2026-10-18)
- [ ] deploy key → 2-of-3 multisig
- [ ] ft2 → production NEP-141 with storage stakes (1NEAR bindings)
- [ ] optional: 6|PAUSE switch checked in on-transfer + claim
- [ ] invariants + differential + gauntlet L5 all green on final wasm
- [ ] live replay spot-check vs oracle (FastNEAR RPC on MAINNET is fine —
      fastnear testnet RPC is stale, do not use it)

## RPC notes

- Testnet: rpc.testnet.near.org rate-limits hard (-429 for minutes after
  ~dozens of calls). near CLI shares the quota. Batch ops; snapshot views
  sparingly; back off exponentially.
- FastNEAR: mainnet only. `rpc.fastnear.com` testnet serves a stale tip
  (≈53M blocks behind, UNKNOWN_ACCOUNT on real accounts).

## TS frontend twin (byte-parity)

`fixtures/clmm_v5.ts` = BigInt twin of the pool's observable semantics
(grid G, dy/dx solvers, swarm arms, leg-decide escrow law, get_state
wire format incl. the LQ:<i>|<v> cell enumeration). Gate:
`python3 scripts/clmm_v5_ts_parity.py` — same op sequences through
near-mock (real wasm) and the twin under tsx; byte-diff of RET/STATE
lines. Green = 4 scenarios / 54 lines byte-exact (2026-10-18).
Harness seam notes: RET lines are ft2 post-resolve `used:<kept>`
(pool raw return = the refund amount); committed-ness in the swarm
arms is `rm != v0`, not exit branch; get_state enumerates only NONZERO
LQ cells in [IMIN..IMAX].

## Evidence artifacts (this repo)

- tests/test_pool_v5_fuzz.rs — differential vs Isabelle OCaml oracle
- tests/test_pool_v5_invariants.rs — ESC/owed/conservation per-op
- tests/test_pool_v5_views.rs, test_pool_v5_c2b.rs — views + two-leg mint
- afp-clmm/v5-kernel/ — Isabelle kernel, OCaml export, mapping doc
- docs/clmm_v5_auth_audit.md — authority findings A–G
