# near-compile

**Lisp/TS/Solidity → NEAR smart contracts, from the command line.**

`near-compile` is the CLI face of the [lisp-rlm](https://github.com/Kampouse/lisp-rlm)
compiler: scaffold a project, compile `.lisp` (or `.ts`/`.sol`) sources to
NEAR-ready wasm, benchmark, deploy to testnet, and view/call contracts —
with [near-mock](https://crates.io/crates/near-mock) for instant local
testing without a node.

```bash
cargo install near-compile        # compiler + project tooling
cargo install near-mock           # local contract runner (no node)
```

## Quickstart

```bash
near-compile init my-contract          # scaffold: near.json, src/main.lisp,
                                       #   tests/, AND .agents/skills/near-compile/
cd my-contract
near-compile build                     # → target/my-contract.wasm (validated + shrunk)

# test locally — milliseconds, no network, deterministic
near-mock target/my-contract.wasm hello --view
NEAR_MOCK_SIGNER=alice.test.near NEAR_MOCK_ATTACH=1 \
  near-mock target/my-contract.wasm some_method '{}'

# deploy + drive on testnet
near-compile deploy --account <your-account>.testnet
near-compile view  <contract> <method> '{}'
near-compile call  <contract> <method> '{"arg": 1}' --account <acct> --deposit 0.1
```

## All commands

```bash
near-compile init <name>               Scaffold a project (skill included)
near-compile build [dir]               Compile from near.json; --target=outlayer[-p2]
near-compile <in.lisp|in.ts|in.sol> <out.wasm>   One-shot compile (frontend auto-selected)
near-compile test [dir]                Build and run the project's tests
near-compile bench <file|wasm>         Fuel-metered benchmark (works for TS too)
near-compile deploy [dir]              Build + deploy (--account/--network/--key-path/--seed-phrase)
near-compile create <acct> [funder]    Create a (sub-)account; --fund = testnet faucet
near-compile call <contract> <method> [args]     Signed call (--deposit is NEAR, e.g. 0.02)
near-compile view <contract> <method> [args]     Read-only RPC
near-compile skill [--stdout|--force]  Install the AI-agent skill into this project
near-compile --repl                    Interactive REPL
```

## The AI-agent skill

Every `near-compile init` project ships with
`.agents/skills/near-compile/SKILL.md` — coding agents (Zed, etc.) working
in the project automatically learn the toolchain: the TS-dialect subset
rules, return conventions, `depositGte` semantics, near-mock testing
patterns, and the deployment paths. Retrofit an existing project:

```bash
near-compile skill              # installs .agents/skills/near-compile/ here
near-compile skill --stdout     # just print it (docs, piping)
```

## Language

Three frontends: **Lisp** (`(define (hello) (near/return_str "hi"))`),
the **TypeScript dialect** (a strict subset — see the skill for the safe
idioms and the forbidden list), and **Solidity** translation. NEAR is
native: storage, context, promises, NEP-297 events, borsh, and
in-contract BIP-340 schnorr verification (stitched at compile time — a
764-line TS multisig compiles to ~150KB with working signature checks).

Compile determinism: same source → byte-identical wasm.

## Verification

```bash
scripts/verify.sh    # 10 checks: compile probes, execute under near-mock,
                     # TS + stitched-schnorr vector
```

## Relation to lisp-rlm

This crate is a **workspace member** of [lisp-rlm](https://github.com/Kampouse/lisp-rlm)
(the compiler core lives in the `lisp-rlm-wasm` root package); the driver
source here is the single source of truth — the workspace builds it, the
verify batteries exercise it, and `cargo publish -p near-compile` ships it
(the path dependency is rewritten to the registry version at publish time,
so the lib publishes first). All compiler and CLI issues belong in
[lisp-rlm](https://github.com/Kampouse/lisp-rlm).
