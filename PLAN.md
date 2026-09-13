# zk-NEAR Stack — Session Continuity Plan

> Living document. Update at end of each session. Read at start of each session.
> Last updated: 2026-09-12

---

## Where We Are (one paragraph)

We have a working zero-knowledge stack on NEAR: a TS→wasm compiler (lisp-rlm, published 0.1.6), a calibrated local runner (near-mock, published 0.7.1, gas within 0.2% of mainnet), a circomlib-exact Poseidon hash on-chain (146 Tgas), and a live Groth16 verifier contract on testnet verifying real snarkjs proofs at 33.6 Tgas. The x>y private-data milestone (user proves statement about private data locally, chain verifies) is DONE end-to-end. The compiler survived a major shakedown: 8 silent-corruption bugs found and fixed. Next: decide between Merkle tree (mixer path), zkVM receipt verifier (general-purpose path), or the Honk verifier port (Noir path — now plausible thanks to the stitched-wasm Grumpkin insight).

---

## What We Have (verified, with receipts)

### On-chain contracts (testnet, all live)

| contract | what | gas | account |
|---|---|---|---|
| Groth16 verifier | verify any snarkjs/circom Groth16 proof | 33.6 Tgas | g16v.poseidon.registry-nostrgov.testnet |
| Poseidon t=3 | circomlib-exact hash over Fr | 146 Tgas | poseidon.registry-nostrgov.testnet |
| BN254 CIOS mul | field mul over base field (p) | 0.16 Tgas/mul | fp254.registry-nostrgov.testnet |
| alt_bn128 hosts | pairing/multiexp/g1sum (in near-mock) | host-priced | (mock) |

### Published crates

| crate | version | what |
|---|---|---|
| lisp-rlm-wasm | 0.1.6 | TS→NEAR wasm compiler (the frontend) |
| near-compile | 0.1.7 | CLI: build/deploy/call/create |
| near-mock | 0.7.1 | local runner, calibrated gas, real crypto hosts |

### Test infrastructure

- **95 tests green** across 26 suites (lisp-rlm) + 46 tests (near-mock)
- Gas calibration: mock matches testnet within 0.2% (fp254 receipts)
- Regression tests pin every bug we've fixed

### zk pipeline (in repo, `zk/` directory)

```
zk/circuit/circuit.circom    — private x>y with Poseidon commitments
zk/circuit/bridge.py         — snarkjs hex → nearcore LE-halves wire format
zk/circuit/mock_flow.py      — pre-flight in near-mock
zk/circuit/tamper_test.py    — soundness check (valid→OK, tampered→BAD)
```

### Key accounts (testnet, keys in ~/.near-credentials/testnet/)

| account | purpose | funded by |
|---|---|---|
| g16v.poseidon.registry-nostrgov.testnet | Groth16 verifier | poseidon.* |
| poseidon.registry-nostrgov.testnet | Poseidon hash | (has ~4 NEAR) |
| fp254.registry-nostrgov.testnet | CIOS probe | (empty) |
| registry-nostrgov.testnet | funder (top-level) | (empty) |

⚠️ Funder accounts are low/empty. `poseidon.registry-nostrgov.testnet` has the most (~4 NEAR) and a working key. Its key file uses `secret_key` field (not `private_key`) — fixed once already, may need re-fixing if regenerated.

---

## What We Fixed (the bug graveyard — don't re-fix, don't regress)

### Compiler bugs (lisp-rlm)

| bug | symptom | fix | date |
|---|---|---|---|
| Hoist-order reversal | loop-body `let` re-inits ran in REVERSE — "values vanish" | in-source-order emission | 09-11 |
| In-place declarations | mid-body `const s16 = t[16]+C` evaluated at body top with stale values | set! at source position | 09-11 |
| Array-literal aliasing | two live arrays from same literal site shared one buffer (mulTwice all-zero) | runtime-heap alloc for array/list ops | 09-11 |
| alt_bn128 hex bridge | raw hex ASCII passed to hosts instead of decoded binary | hex⇄binary bridge in emitter | 09-11 |
| Bool-in-if always-true | `const take = r < 2; if (take)` → numeric compare, always true | pass raw to tag-aware if emitter | 09-12 |
| + concat dispatch (2 shapes) | top-level const strings + nullish-seeded locals → numeric + | CONST_FOLDS stringy + paren/nullish look-through | 09-12 |
| M2 impure declarations | `const b = host_call()` after early return still executed (state corruption) | hoist to nil + guarded set! | 09-12 |
| Nested returns vanish | return in inner while only stopped inner loop | function-level __fn_done/__fn_res flags | 09-11 |

### near-mock bugs

| bug | fix | date |
|---|---|---|
| BN254 pairing stride 128B→192B | real gates trapped "Invalid input length" | POINT_SIZE + POINT_SIZE*2 | 09-11 |
| Error chains hidden | cross-mode traps printed messageless | TxOutcome.error carries full chain | 09-11 |
| Deprecated host wording | didn't match mainnet exactly | "Attempted to call deprecated..." | 09-11 |
| Embedded copy stale | lisp-rlm's embedded near-mock was 6 months behind | full re-sync | 09-11 |

### Operational gotchas (learned the hard way)

- Top-level `const` arrays re-execute their literal per access — **always use function-local constants**
- `cargo build` can timeout in foreground on lisp-rlm — use `nohup ... &` and poll
- Disk fills: clean `target/debug/` (~16GB), check `df -h` before long builds
- `near-compile` binary can be stale — rebuild after any frontend change (`cargo build --release -p near-compile`)
- Account creation: `near-compile create <name> <funder>` appends funder as suffix parent — keep names short
- snarkjs vkey key is `IC` not `VK`
- circomlib `LessThan` max is 252 bits, not 254

---

## Open Fronts (ranked by leverage)

### 1. 🟢 Merkle tree + nullifiers (the mixer — closest to done)

**What**: incremental Merkle tree with Poseidon, nullifier set, membership proof circuit
**Why**: unlocks privacy pools, mixers, anonymous signaling — the classic zk app
**Missing**: the tree contract (~2 days), the membership circuit (~1 day), Poseidon gas cut
**Blocker**: Poseidon at 146 Tgas → 30-level insert = 15 calls. Needs optimization (see #4)
**Files**: none yet — would go in `zk/merkle/`

### 2. 🟡 Honk verifier port (the Noir unlock — now plausible)

**What**: port Barretenberg's Honk/Shplemini verifier to our TS, with Grumpkin as stitched wasm
**Why**: native on-chain Noir proof verification — best architectural outcome
**Breakthrough**: the schnorr module proved the pattern — stitched raw-wasm Grumpkin point ops at ~0.4 Tgas each (vs 2+ TS-level), plausibly fitting in ~150-250 Tgas total
**Missing**: anatomy study (1-2 days), then the port (2-4 weeks)
**Next step**: fetch bb's current Ethereum verifier Solidity, count actual Grumpkin ops, multiply by stitched-wasm cost model
**Key files**: `schnorr/src/lib.rs` (the pattern), `wasm_link.rs` (the stitcher), `near-mock/src/bn254.rs` (host formats)

### 3. 🟡 zkVM receipt verifier (general-purpose — RISC Zero/SP1)

**What**: port SP1 or RISC Zero's receipt verifier (~600-1500 lines Solidity → TS)
**Why**: "prove arbitrary Rust programs on NEAR" — rollups, coprocessors, cross-chain
**Missing**: the port (1-2 weeks), has a reference implementation to diff against
**Note**: same pairing hosts + keccak underneath, no new primitives needed
**Also**: this is the fallback Noir path (prove bb verification inside the zkVM)

### 4. 🟡 Poseidon optimization (the gas multiplier)

**What**: 29-bit limbs (9 limbs vs 16) + sparse partial rounds (circomlibjs poseidon_opt)
**Why**: 146 → ~35-50 Tgas per hash → Merkle insert 15 calls → 3-5 calls
**Missing**: mechanical port of both optimizations (~2-3 days combined)
**Impact**: makes #1 (mixer) ergonomic, not just possible
**Files**: `fixtures/poseidon_bn254.ts` (current), would become `poseidon_opt.ts`

### 5. ⚪ Noir language support (watch, don't build)

**What**: author circuits in Noir, prove with bb, verify on NEAR
**Status**: arkworks backend dead (2 years stale, arithmetic-only). Path B (#2 above) is the real route. Watch for Aztec shipping a maintained Groth16 wrapper — that would be a ~50-line bridge for us.
**NOT doing**: reviving the arkworks backend (weeks, restricted language)

### 6. ⚪ Additional precompiles / hosts

- BLS12-381 msig (tests pass, not deployed)
- ML-DSA-65 (post-quantum, host exists in protocol)
- Grumpkin host (would need protocol-level ask — not ours to add)

---

## Architecture (the pieces and how they connect)

```
┌─────────────────────────────────────────────────────────────┐
│ USER (browser/CLI)                                          │
│   private data → witness → proof (snarkjs, seconds)        │
│   proof + commitments → tx to NEAR                          │
└─────────────────────────┬───────────────────────────────────┘
                          │
┌─────────────────────────▼───────────────────────────────────┐
│ NEAR CONTRACT (lisp-rlm TS → wasm)                          │
│                                                             │
│  Groth16 verifier:                                          │
│    multiexp(IC, inputs) + pairing_check(4 pairs) → OK/BAD  │
│    → alt_bn128 hosts (in-protocol, gas-priced)             │
│                                                             │
│  Poseidon (for Merkle/state commitments):                   │
│    CIOS 16-limb → 146 Tgas (or 35-50 after optimization)  │
│                                                             │
│  [future] Honk verifier:                                    │
│    stitched Grumpkin wasm (~0.4 Tgas/point op)             │
│    + BN254 pairing host + keccak transcript                │
└─────────────────────────┬───────────────────────────────────┘
                          │
┌─────────────────────────▼───────────────────────────────────┐
│ LOCAL DEV LOOP (near-mock)                                  │
│   compile → run → gas (0.2% of mainnet) → deploy → verify  │
│   fork mainnet state / replay any tx / calibrated fees     │
└─────────────────────────────────────────────────────────────┘
```

---

## Session Protocol

### At session start:
1. Read this file
2. Check testnet contract health (see accounts table)
3. Run `cargo test --release` on both repos to confirm green

### At session end:
1. Update "Where We Are" paragraph
2. Update any changed tables
3. Add new bugs to the graveyard
4. Add new accounts/contracts
5. Re-rank the open fronts if priorities shifted
6. Commit + push this file

### When context gets compacted:
This file IS the recovery point. It should contain everything needed to resume without re-deriving.

---

## Quick Reference

### Build commands
```bash
# lisp-rlm (compiler + tools)
cd /Users/j-p/dev/stuff/lisp-rlm
CARGO_INCREMENTAL=0 cargo build --release --bin compile --bin near-mock
CARGO_INCREMENTAL=0 cargo build --release -p near-compile

# near-mock (standalone)
cd /Users/j-p/dev/stuff/near-mock
CARGO_INCREMENTAL=0 cargo build --release

# test a TS contract locally
./target/release/compile input.ts output.wasm
./target/release/near-mock output.wasm method '{}' --prepaid 300

# deploy + call
./target/release/near-compile build /tmp/project
./target/release/near-compile deploy /tmp/project
./target/release/near-compile call <account> <method> '<json>' /tmp/project
```

### The zk pipeline (circom → on-chain)
```bash
cd zk/circuit
/tmp/circom2 circuit.circom --r1cs --wasm --sym -o .          # compile circuit
node make_input.js                                              # compute commitments
node circuit_js/generate_witness.js circuit_js/circuit.wasm input.json witness.wtns
snarkjs groth16 prove circuit_final.zkey witness.wtns proof.json  # prove
python3 bridge.py .                                             # convert formats
# → init_args.json (VK), verify_args.json (proof)
# → deploy verifier, call init(vk), call verify(proof)
```

### Publishing
```bash
cd lisp-rlm && git add -A && git commit -m "..." && git push
cargo publish -p lisp-rlm-wasm
sleep 15 && cargo publish -p near-compile
cd ../near-mock && git add -A && git commit -m "..." && git push && cargo publish
```

### Important paths
```
/Users/j-p/dev/stuff/lisp-rlm/          — compiler + tools + tests
/Users/j-p/dev/stuff/lisp-rlm/zk/       — circuit + bridge + pipeline
/Users/j-p/dev/stuff/lisp-rlm/schnorr/  — the stitched-wasm pattern (KEY for Honk)
/Users/j-p/dev/stuff/near-mock/         — the local runner
/Users/j-p/dev/stuff/lisp-rlm/fixtures/ — verified contract sources
/Users/j-p/dev/stuff/lisp-rlm/tests/    — regression suite
```
