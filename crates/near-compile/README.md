# near-compile

**Lisp/TS/Solidity → NEAR smart contracts, from the command line.**

`near-compile` is the CLI face of the [lisp-rlm](https://github.com/Kampouse/lisp-rlm)
compiler: scaffold a project, compile `.lisp` (or `.ts`/`.sol`) sources to
NEAR-ready wasm, benchmark against [near-mock](https://crates.io/crates/near-mock),
and view/call deployed contracts on any network.

```bash
cargo install near-compile        # or: cargo install --git https://github.com/Kampouse/lisp-rlm --bin near-compile
```

## Usage

```bash
near-compile init my-contract          # scaffold near.json + main.lisp
near-compile build                     # main.lisp → target/near/my_contract.wasm
near-compile <in.lisp> <out.wasm>      # one-shot compile

near-compile bench <file.lisp>         # gas/cost benchmark via near-mock
near-compile view <contract> <method> [args]   # read-only RPC
near-compile call <contract> <method> [args]   # signed RPC (needs key in near.json)
```

The language speaks NEAR natively: `near/store-bytes`, `near/signer_account_id`,
`near/block_timestamp`, `near/panic`, NEP-297 `EVENT_JSON:` logs via
`near/log`, promises, borsh/collections — compiled to compact wasm
(a `whoami/clock/gate` demo contract is 2.1KB).

Compile determinism: same source → byte-identical wasm (verified against the
near-mock fixture battery).

## Verification

```bash
scripts/verify.sh    # compiles probe contracts and executes them under near-mock
```

## Relation to lisp-rlm

This crate is a **workspace member** of [lisp-rlm](https://github.com/Kampouse/lisp-rlm)
(the compiler core lives in the `lisp-rlm-wasm` root package); the driver
source here is the single source of truth — the workspace builds it, the
verify batteries exercise it, and `cargo publish -p near-compile` ships it
(the path dependency is rewritten to the registry version at publish time,
so the lib publishes first). All compiler and CLI issues belong in
[lisp-rlm](https://github.com/Kampouse/lisp-rlm).
