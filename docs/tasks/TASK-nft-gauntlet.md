# TASK: near-mock NFT gauntlet — find and fix the `@aaa…` key-garbage bug

## Goal
Make the STOCK near-sdk-rs NFT example run through near-mock's `cross` mode
the same way the stock FT example already does (it passes end-to-end).
Currently `new`/`new_default_meta` traps with:
  PANIC: The collection is an inconsistent state. Did previous smart contract execution terminate unexpectedly?

## Where things are
- Runner: `~/.openclaw/workspace/lisp-rlm/src/bin/near_mock.rs` (wasmtime-based NEAR VM mock; vendored `near-vm-runner` in `vendor/` is the semantics reference).
- Build: `cd ~/.openclaw/workspace/lisp-rlm && cargo build --release --bin near-mock` (~9s).
- Stock NFT wasm (already built): `/tmp/near-sdk-rs/examples/non-fungible-token/target/near/non_fungible_token/non_fungible_token.wasm`
- Stock FT wasm (PASSES — your regression oracle): `/tmp/near-sdk-rs/examples/fungible-token/target/near/fungible_token/fungible_token.wasm`
- SDK source for reading: `/tmp/near-sdk-rs/`
- Repro (fresh state):
  ```
  NM=~/.openclaw/workspace/lisp-rlm/target/release/near-mock
  W=/tmp/near-sdk-rs/examples/non-fungible-token/target/near/non_fungible_token/non_fungible_token.wasm
  $NM cross /tmp/dbg.bin "nft.test.near=$W" nft.test.near new \
    '{"owner_id":"owner.test.near","metadata":{"spec":"nft-1.0.0","name":"G","symbol":"G"},"royalties":{}}'
  ```
- Working FT comparison: same manifest pattern, `new_default_meta '{"owner_id":"owner.test.near","total_supply":"1000"}'`.

## Evidence so far (all reproduced)
1. Unfiltered trace shows the CONTRACT issuing garbage storage keys, e.g.:
   - `storage_read not found [v@aaaaaaaa…(64 'a's total)]`
   - `storage_write("n") = 86b`
   - `storage_write("v@aaaa…") = 68b`, `storage_write("@aaaa…") = 292b`
   - then `storage_read not found [<32B binary hash><@aaa…>]` → writes 8b
   - later reads of those same keys SUCCEED (round-trip consistent!) and THEN the collection-inconsistent panic fires during `nft_mint`/init serde.
2. '@'=0x40, 'a'=0x61. The 64-byte `@aaa…` pattern appears as key MATERIAL inside
   `store::`-style hashed keys (32-byte digest + inner key). It is NOT in the SDK source,
   not in the example source. Suspected: uninitialized-heap buffer being read as a key —
   i.e., some host function returns SUCCESS (or writes a register) when it should return
   ERR/leave register untouched, so near-sdk's `env::read_register` picks up heap slack.
3. Register ids seen: 0xFFFFFFFFFFFFFFFD (u64::MAX-2), 0xFFFFFFFFFFFFFFFE (MAX-1) — near-sdk's
   KEY/ATOMIC_OP registers. Check our `input`, `read_register`, `register_len`, and
   `write_reg_checked` handling of these ids and of register-id aliasing in the register map.
4. sha256/keccak hosts exist (lines ~2113-2232) and the lisp test suite passes (61+13+3), so
   basic crypto hosts work — but NFT init path may exercise a different register sequence.
5. ALL lisp-rlm tests + sandbox-tests erc20 pass on current HEAD (after today's promise-semantics
   fix: returned-promise results REPLACE the child's void outcome, see commit history 2026-09-07).

## Hypotheses (in order of promise — verify, don't assume)
H1. A host fn (input/storage_read/has_key/read_register) writes a register with heap-slack
    bytes when it should either fail or write nothing — near-sdk then uses that garbage as key.
    Look first at `read_register` when register is EMPTY: what does our host do vs
    `vendor/near-vm-runner` VMLogic (register map semantics: missing register →
    InvalidRegisterId error, memory NOT touched).
H2. `register_len` returning garbage/0 for missing registers instead of U64::MAX error code.
H3. wee_alloc heap slack is genuinely 0x61-filled in this wasm (its own pattern) and our
    host returns success on an op that on-chain FAILS, letting SDK read heap slack.
H4. Multi-register aliasing: MAX-1 vs MAX-2 map collisions.

## Method that works
- Diff host behavior against `vendor/near-vm-runner/src/logic/` (logic.rs = semantics oracle).
- Add temporary eprintln traces (pattern already used in file) rather than guessing.
- `RUST_LOG`/env not set up — just use eprintln + rebuild (9s).

## Definition of done
1. `new` + `nft_mint` (with royalties like
   `{"token_id":"t1","receiver_id":"alice.test.near","metadata":{"title":"x"},"royalties":{"bob.test.near":1000}}`)
   + `nft_token` view + `nft_transfer` (storage_deposit for receiver first, like FT) all succeed,
   `nft_tokens_for_owner` returns the token.
2. FT gauntlet still passes (run the sequence above).
3. `cargo test --release -p lisp-rlm-wasm` and `cd sandbox-tests && cargo test --release` all green.
4. COMMIT with a message explaining the root cause. ONE commit, focused diff.
5. Write findings summary to `~/.openclaw/workspace/memory/near-mock-nft-fix.md`.

## Constraints
- Do NOT touch anything outside lisp-rlm's near-mock surface (src/bin/near_mock.rs, maybe
  lib helpers it uses). No refactors, no drive-by fixes.
- Never `git add -A` blindly — add specific files.
- If a hypothesis requires changing promise semantics, STOP and report instead (that area
  just had a deliberate fix today).
