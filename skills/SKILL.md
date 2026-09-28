---
name: near-mock
description: >-
    Run, test, debug and REPLAY NEAR smart contracts locally — real wasm,
    real host surface, mainnet-parity gas, and FORKS of real mainnet state.
    Use for any task involving NEAR contracts: local execution, debugging
    failed transactions, forking live state, gas analysis, scenario tests,
    or forensic analysis of on-chain behavior.
---

# near-mock — local NEAR contract runner (no node, no signing, zero blast radius)

Real wasm, ~140 host functions, **mainnet-parity gas** (finite-wasm
instrumentation, PV155 costs — the same meter mainnet runs), deterministic
clock/entropy, receipt-atomic state. In-process, milliseconds.

```bash
cargo install near-mock    # 0.3.0+
```

**Safety**: near-mock never signs or broadcasts anything. Agents can point
it at mainnet freely — writes land in local state files only.

## The six modes

```bash
# 1. single call (local wasm, fresh state)
near-mock out.wasm method '{"json":"args"}' [--view] [--json]

# 2. cross: multi-contract world in one state file
near-mock cross state.bin acct=/path/a.wasm,acct2=/path/b.wasm acct2 method '{}'

# 3. scenario: scripted multi-step flows with expectations
near-mock scenario flow.json

# 4. fork: call a REAL mainnet contract against its REAL state — zero setup
near-mock fork intents.near is_account_locked '{"account_id":"<hex>"}'
near-mock fork wrap.near ft_balance_of '{"account_id":"alice.near"}' --block 215138898
#    --block H (pinned) | --block final (always-fresh, may drift) | omitted (latest, pinned)
#    --view for read-only · --json for machine output
#    code (view_code) + storage (paginated view_state) page in lazily —
#    works on the biggest contracts (intents.near 11.7GB) via pagination

# 5. replay: re-execute a REAL mainnet transaction locally and diff it
near-mock replay <tx-hash> [--json] [--trace]
#    forks state at the block before execution, replays the entry receipt
#    with the real predecessor/args/deposit, diffs status/logs/gas vs
#    mainnet's recorded outcome. THE "why did my tx fail?" command.
#    --trace adds the full host-call timeline (every storage key, gas/op).

# 6. snapshot: pull a contract + state for offline forensics (legacy;
#    fork-mode usually better — snapshot fails on huge contracts)
near-mock snapshot some-contract.testnet state.bin --rpc <archival-url>
```

## Gas numbers are real

Instruction gas uses the same finite-wasm instrumentation + PV155 cost
model as mainnet (regular_op_cost/op, control flow free, bulk ops =
base + unit × length). Host costs, action fees, register/memory
composites are protocol-86 values. Known calibration gap: trie-node
charges use a flat walk model — ~0.6x on 10+ GB contracts (intents,
wrap), ~0.9x+ on small ones. `replay` prints the measured ratio.

## Determinism controls

NEAR_MOCK_SEED (entropy) · NEAR_MOCK_NOW / --advance (clock) ·
--fail-receipt N (injected receipt failure) · --state FILE (persistence).
Fork-mode: pin --block for reproducibility.

## JSON everywhere

`--json` on fork/replay/cross/call: machine-readable outcome (return
value, error class, gas, logs; replay adds the mainnet diff + host
timeline). Prefer it when scripting or driving from an agent.

## Library

```rust
use near_mock::chain::MockChain;
let chain = MockChain::builder()
    .contract("x.test.near", "out.wasm")?      // or .fork(rpc, Some(block))
    .signer("alice.test.near")
    .build()?;
let out = chain.call("x.test.near", "mint").args("{}").fire()?;
// out.ok / out.logs / out.panic / out.gas_burned / out.receipt_failures
```

## Debugging recipe (agents)

1. "What does mainnet state say?" → `near-mock fork <acct> <view> '{...}' --json`
2. "Why did tx T fail?" → `near-mock replay T --json --trace`
3. Divergence mock-vs-mainnet → compare panic classes + host trace; gas
   ratio ≈ 0.6 on whale contracts is expected calibration, not a bug.
4. Multi-step flows → `near-mock scenario` (see example-scenario.json)
5. Live differential testing at scale → examples/stream_replay.rs in the
   repo (multi-contract replayer with per-method scoreboards).
