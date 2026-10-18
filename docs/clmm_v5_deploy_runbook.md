# CLMM v5 Deploy Runbook

Wasm: `lib/{limb_math,clmm_v5_kernel,clmm_pool_v5}.lisp` concatenated →
`near-compile` (single-file only — near-compile compiles the LAST arg;
concatenate via script, never multi-arg).

## Local build

    cat lib/limb_math.lisp lib/clmm_v5_kernel.lisp lib/clmm_pool_v5.lisp > /tmp/pool_concat.lisp
    ~/dev/lisp-rlm/target/release/near-compile /tmp/pool_concat.lisp /tmp/pool.wasm
    # expect ✅ in output, exit 0. Never mask a nonzero exit.

FT: `cat lib/limb_math.lisp lib/clmm_core.lisp lib/ft2.lisp` → same.

## Testnet deploy (current: pool.clmm5-51fe5a.testnet)

    # 1. build (above)
    # 2. deploy — near CLI asks confirmation; echo y | it
    echo y | near deploy pool.clmm5-51fe5a.testnet /tmp/pool.wasm --networkId testnet
    # 3. verify state survived (state persists across code swap; keys 6|*)
    curl -s -X POST https://rpc.testnet.near.org -H 'Content-Type: application/json' \
      -d '{"jsonrpc":"2.0","id":1,"method":"query","params":{"request_type":"call_function",
           "account_id":"pool.clmm5-51fe5a.testnet","method_name":"_run",
           "args_base64":"'"$(printf '{"op":"state"}' | base64)"'","finality":"final"}}'

## Init policy (Finding D)

`init` is first-caller-wins. After a FRESH deploy to a NEW account, run
`init` in the same transaction batch as deploy:

    near tx add ... deploy ... function-call init ...

Never leave a fresh account un-inited.

## Prod checklist (pre-mainnet)

- [ ] Finding A fixed: burn authority (+ kernel premise + negative tests)
- [ ] Finding B decided: donate funding rule
- [ ] Finding C decided: gross swap sim-only or wired to ft_on_transfer
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

## Evidence artifacts (this repo)

- tests/test_pool_v5_fuzz.rs — differential vs Isabelle OCaml oracle
- tests/test_pool_v5_invariants.rs — ESC/owed/conservation per-op
- tests/test_pool_v5_views.rs, test_pool_v5_c2b.rs — views + two-leg mint
- afp-clmm/v5-kernel/ — Isabelle kernel, OCaml export, mapping doc
- docs/clmm_v5_auth_audit.md — authority findings A–G
