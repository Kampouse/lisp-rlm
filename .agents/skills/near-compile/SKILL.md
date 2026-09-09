---
name: near-compile
description: >-
    Compile and test NEAR smart contracts written in Lisp or the
    lisp-rlm TypeScript dialect (TS subset) to WASM, and run them locally
    under near-mock with full NEAR host semantics. Use when a task involves
    the lisp-rlm toolchain: writing, compiling, deploying, or testing
    contracts in this repo or projects using it (e.g. nostr-gov).
---

# near-compile — Lisp/TS → NEAR contract toolchain

Two binaries do everything:

- `near-compile` — compile `.lisp` / `.ts` (dialect) / `.sol` sources to
  NEAR wasm; scaffold, deploy, bench, view/call contracts on testnet.
- `near-mock` — run compiled (or snapshot-fetched) contracts locally with
  the real NEAR host surface: deterministic clock, state files, storage
  staking, promises, stitched BIP-340 schnorr. No node, milliseconds.

Both install from crates.io (`cargo install near-compile` / `cargo install
near-mock`) or build from this repo (`cargo build -p near-compile` /
`--bin near-mock`; the crypto artifact is committed — one-step build).

## Compile

```bash
near-compile src/main.ts target/out.wasm          # TS dialect source
near-compile build                                # project mode: near.json in cwd
near-compile init my-contract                     # scaffold (near.json + src/main.ts + tests/)
```

Project `near.json`: `{ name, src, account, network, output, tests }`.
`build` compiles, validates (wasmparser), and with `wasm-opt` installed
shrinks (`--enable-bulk-memory-opt -g -Oz`, name section preserved for
trap symbolication). When a `public/` dir exists beside the project,
build.sh conventions sync the optimized wasm there (no UI-binary drift).

## TS dialect rules (violations fail the typechecker or compile)

The frontend accepts a STRICT M1/M2 subset. Verified-safe idioms (all used
by nostr-gov's 764-line contract):

- `near.jsonGetStr("k") ?? ""`, `near.storageGet(k) ?? ""`,
  `near.storageSet(k, v)`, `strLength`, `strSlice`, `strIndexOf`,
  `strCat`, `strSplit` (returns LispArr: `.length`, `[i]`, `.push`),
  `strToNum`, `toStr`, template literals, `u128Add/Sub/...`
- Declare `const` at function top level; inside loops use `let` +
  `while (i < n)` idioms (no `const` in loop bodies, no `for...of`
  guarantee) — copy the loops in `contract-ts/src/main.ts` of nostr-gov.
- NO: `Math.*`, `String.fromCharCode`, `.split()` on strings (use
  `strSplit`), `charCodeAt`, `throw`, `as` casts, string relational
  comparisons (`c >= "a"` — use charset `strIndexOf` membership instead),
  `strJoin` (use `.join(",")` on arrays or `strCat`).

### Return conventions (a live-deployment lesson)

- View methods: `return "value";` — the frontend lowers export-level
  `return <expr>` to a `value_return`. Do NOT mix `near.jsonReturnStr(x)`
  with `return 0` — both emit value_return and the LAST one wins
  (nearcore semantics; near-mock ≥0.1.5 matches).
- Mutating methods may `return 0;`.
- `depositGte(lo, hi)` is ONE u128 threshold split (lo64, hi64) — NOT
  (base, per-byte). 0.01 Ⓝ ≈ `depositGte(0, 542)`. Its result is
  boolean-tagged: `if (!near.depositGte(...))` — never `!== 1`.

## Test locally with near-mock

```bash
near-mock out.wasm method '{"json":"args"}' --view     # single call
NEAR_MOCK_SIGNER=alice.test.near NEAR_MOCK_NOW=1787000000 \
NEAR_MOCK_ATTACH=20000000000000000000000 \
  near-mock out.wasm method '{...}'                    # signed call + deposit
near-mock cross state.bin acct=path.wasm acct method '{}'   # multi-contract
near-mock snapshot live.acct.testnet state.bin --rpc https://rpc.testnet.near.org
near-mock state dump|import|reset ...                       # inspect/mutate state
```

- Determinism: `NEAR_MOCK_NOW` (unix secs) pins the clock,
  `NEAR_MOCK_SEED` pins randomness, `--advance <secs>` time-travels.
- `--view` enforces read-only; traps roll back atomically (single tx).
- near-sdk 4/5 AND near-contract-standard binaries instantiate (protocol
  69/72 hosts bound as of near-mock 0.1.4+).
- Signatures: `schnorrVerify`/`sha256Hash` work in-contract via the
  stitched crypto (verify with BIP-340 vectors from a reference impl).

## Deployment (testnet)

```bash
near-compile deploy --account <acct>          # builds + deploys from near.json
near-compile call <contract> <method> '<json>' --account <acct> --deposit 0.02
near-compile view <contract> <method> '<json>'
near-compile create <name> <funder>           # sub-account creation (non-interactive)
```

`--deposit` is NEAR-decimal (`0.02`), NOT yocto. Credentials come from
`~/.near-credentials/<network>/<account>.json` (`private_key` or
`secret_key`, base58 `ed25519:`). Top-level account creation without a
funder: POST `{"newAccountId", "newAccountPublicKey"}` to
`https://helper.testnet.near.org/account` (see nostr-gov
registry/scripts/deploy-registry.sh for a scripted openssl+faucet path —
near-cli-rs demands an interactive TTY confirmation that scripts can't
provide).

## Reference projects

- `nostr-gov` (github.com/Kampouse/nostr-gov): 764-line TS-dialect
  multisig treasury with schnorr governance; `contract-ts/tests/e2e-mock.py`
  is the canonical offline e2e pattern (signed events, deposits, full
  lifecycle). Its `registry/` is a minimal public-registry example.
- This repo's `scripts/verify_near_mock.sh` (61 checks) and
  `crates/near-compile/scripts/verify.sh` (10 checks) are the regression
  batteries — run them after any emitter/host change.
