# zk-NEAR Stack — Session Continuity Plan

> Living document. Update at end of each session. Read at start of each session.
> Last updated: 2026-09-15 (session 5 — u128 L1+L1.5 LANDED, PLONK LANDED (53 Tgas, universal setup),, fib −62%; chunked-to_str zero-NUL bug found in published 0.1.12 and fixed; 0.1.13/0.1.14 published)

---

## Where We Are (one paragraph)

We have a working zero-knowledge application layer on NEAR: three zk apps live on testnet (anonymous identity credentials, anonymous voting v3 with choice sealed in circuit, anonymous voting v4 with homomorphic tally where nobody sees individual choices), all built on a Groth16 verifier (34 Tgas), circomlib-exact Poseidon (146 Tgas), and the alt_bn128 hosts. The compiler survived 8 silent-corruption bugs (session 3) + a session-4 bug-fix marathon (26 fixed total, bug backlog empty) + a 2026-09-14/15 gas marathon: hot loops -47%, getter entrypoints -86% (input caching + scanner dedup + eq fast paths), Poseidon 146→124.9 Tgas, JSON API v3 (near.input handles, typed ??, near.args<T>), u128 ~9× (chunked to_str), and **u128 Level 1 limb locals** (session 5: fib(185) 2.452→0.925 Tgas, −62% — u128-pure locals never stringify; the nil-dummy poisoning fix in the loop hoisting was the key unlock). Session 5 also caught a live bug in published 0.1.12: the chunked to_str zero fast-path returned a NUL string for zero values (broke AMM/auction fixtures via downstream parse traps) — fixed + pinned, 0.1.13/0.1.14 published. The PLONK verifier (universal setup) has skeleton + init deployed; transcript implementation is the remaining ~2-3 hours. Key architectural discoveries: (1) Noir is a frontend, not a proof system — the arkworks backend that would slot into our verifier is dead; (2) PLONK works on our hosts and eliminates the trusted setup ceremony; (3) NEAR's MPC network doesn't support BN254 threshold decryption (wrong curve, wrong purpose); (4) homomorphic tally via additive ElGamal over BN254 gives the strongest voting privacy achievable with existing hosts. Open wound CLOSED (session 5): test_ts_flashloan was double-broken — (1) the fixture's balance() view read a deposit-only storage ledger (never updated by transfers; committed red, proven by running the original engine), (2) the scenario runner leaked the execute-once receipt memo across steps — step N+1's receipts REPLAYED step N's memoized outcomes instead of executing (silent no-op: no value moved, no abort). Both fixed (a5adaf4); the same memo leak was in the published near-mock crate → fixed + published as near-mock 0.7.2.

---

## What We Have (verified, with receipts)

### On-chain contracts (testnet, all live)

| contract | what | gas | account |
|---|---|---|---|
| Groth16 verifier | verify any snarkjs/circom Groth16 proof | 34 Tgas | g16v2.poseidon.registry-nostrgov.testnet (identity circuit VK) |
| Groth16 verifier (orig) | x>y circuit VK | 34 Tgas | g16v.poseidon.registry-nostrgov.testnet |
| Poseidon t=3 | circomlib-exact hash over Fr | 146 Tgas | poseidon.registry-nostrgov.testnet |
| BN254 CIOS mul | field mul over base field (p) | 0.16 Tgas/mul | fp254.registry-nostrgov.testnet |
| zk-Identity | anonymous credential state (root + nullifier) | ~1 Tgas | zkid.poseidon.registry-nostrgov.testnet |
| zk-Vote v3 | anonymous voting, choice sealed in circuit | ~1 Tgas/vote | zkvote.poseidon.registry-nostrgov.testnet |
| zk-Vote v4 | homomorphic tally (additive ElGamal) | ~2 Tgas/vote | zkvote.poseidon.registry-nostrgov.testnet |

### Published crates

| crate | version | what |
|---|---|---|
| lisp-rlm-wasm | 0.1.15 | TS→NEAR wasm compiler — + deep-stack compile (256 MiB), forward references |
| | 0.1.14 | — parse-cache, compile-CLI data-loss fix |
| near-compile | 0.1.16 | CLI: build/deploy/call/create |
| near-mock | 0.7.1 | local runner, calibrated gas, real crypto hosts |

### Test infrastructure

- **145+ tests green** across 33+ suites (lisp-rlm) + 46 tests (near-mock)
- Gas calibration: mock matches testnet within 0.2% (fp254 receipts)
- Regression tests pin every bug we've fixed

### zk apps (all in `zk/` directory, all e2e verified on testnet)

```
zk/identity/           — anonymous credential (Merkle membership + nullifier)
  circuit.circom       — Poseidon commitment + Merkle path + nullifier
  prove.js             — client-side proving (snarkjs CLI: 1.4s per proof)
  plonk_verifier.ts    — PLONK verifier skeleton (init works, transcript pending)

zk/vote/               — anonymous voting (3 iterations)
  circuit_v3.circom    — choice sealed inside circuit (Poseidon(choice, blinding))
  contract_v3.ts       — sealed ballots, reveal at close, aggregate tally
  contract_v4.ts       — homomorphic tally (additive ElGamal, nobody sees choices)
  bn254.js             — correct BN254 G1 arithmetic in JS (generator (1,2))
  prove_v4.js          — encrypted ballot generation + homomorphic sum verify

zk/circuit/            — x>y private data proof (first e2e milestone)
zk/bridge.py           — SHARED format bridge (snarkjs → NEAR LE-halves)
```

### Key accounts (testnet, keys in ~/.near-credentials/testnet/)

| account | purpose | status |
|---|---|---|
| poseidon.registry-nostrgov.testnet | funder + Poseidon contract | ~4 NEAR |
| g16v2.poseidon.registry-nostrgov.testnet | Groth16 verifier (identity VK) | funded |
| g16v.poseidon.registry-nostrgov.testnet | Groth16 verifier (x>y VK) | funded |
| zkid.poseidon.registry-nostrgov.testnet | zk-Identity + PLONK verifier | ~1 NEAR |
| zkvote.poseidon.registry-nostrgov.testnet | zk-Vote v3 + v4 | funded |
| registry-nostrgov.testnet | top-level funder | ~0.5 NEAR |

⚠️ Key file quirk: poseidon's key uses `secret_key` field (not `private_key`) — fixed once, check if regenerated.

---

## What We Fixed (the bug graveyard — don't re-fix, don't regress)

### Compiler bugs (lisp-rlm) — 26 total, all fixed and tested

**2026-09-14 sweep note:** probed every "open" GAPS entry live before fixing — ~half were STALE (arity fixed 08-26, lisp-run surface gaps all work, kv asymmetry documented backwards). Probe before fixing claimed bugs.

| bug | symptom | fix | date |
|---|---|---|---|
| Hoist-order reversal | loop-body `let` re-inits ran in REVERSE — "values vanish" | in-source-order emission | 09-11 |
| In-place declarations | mid-body `const s16 = t[16]+C` evaluated at body top | set! at source position | 09-11 |
| Array-literal aliasing | two live arrays from same literal site shared one buffer | runtime-heap alloc | 09-11 |
| alt_bn128 hex bridge | raw hex ASCII passed to hosts instead of decoded binary | hex⇄binary bridge | 09-11 |
| Bool-in-if always-true | `const take = r < 2; if (take)` → numeric compare | pass raw to tag-aware if | 09-12 |
| + concat (const strings) | top-level const string in concat → numeric + | CONST_FOLDS stringy | 09-12 |
| + concat (nullish locals) | `(storageGet() ?? "") + var` → numeric + | paren/nullish look-through | 09-12 |
| M2 impure declarations | `const b = host_call()` after early return still executed | hoist to nil + guard | 09-12 |
| Nested returns vanish | return in inner while only stopped inner loop | function-level flags | 09-11 |
| jsonGetStr literal-only | runtime-built keys (concat, storage read) = hard compile error | json_dyn_lookup_str: key → heap "key": pattern → shared __json_get | 09-13 |
| jsonGetStr clobber | consecutive dynamic reads overwrote each other (stdout_buf scratch) | heap-copy before tagging | 09-13 |
| `continue` unsupported | keyword didn't exist | __wl_done set, __wl_brk kept clear; update guard on __wl_brk | 09-13 |
| Top-level const exprs | `const K = <expr>` → "undefined variable K" (value defines skipped in from_exprs path) | emit 0-param fn + register in value_defines | 09-13 |
| while(true)+exit forms | never compiled: bool/int cond-arm type mismatch; trailing if-return without fn flags | statically_bool false_e; guards gate on fn_bound | 09-13 |
| String repeat/pad missing | `.repeat()/.padStart()/.padEnd()` typed but never lowered | (str-repeat s n) builtin + let-bound pad expr | 09-13 |
| Loop/if-branch decl scoping | `let j` in for/for-of bodies + if-branches → dead binding, "undefined variable" | hoist bind-nil + set! like while bodies | 09-13 |
| Local-slot cross-type reuse | freed i64 slot reused as i32 → type map rewritten → INVALID wasm (whole module fails validation) | type fixed at first alloc; same-type reuse only | 09-13 |
| jsonGetInt literal-only | same as jsonGetStr | shares json_dyn_lookup_str + __str_to_num; miss → TAG_NIL | 09-14 |
| Negative Num returns | both export-wrapper untag sites used ShrU — `return -5` crossed host boundary as 2^61 garbage | I64ShrS at both sites | 09-14 |
| JSON space-before-colon | `"k" : v` silently missed on dynamic keys, dot-paths, json-extract, u128 (only literal scanners were lenient — fix I1 08-27) | bare quoted key + ws-skip + colon-required in __json_get, __json_extract_N, from_buf, dyn, u128 | 09-14 |
| Dispatch table divergence (t13) | str-length chars-vs-bytes, str-split empties, to-int error-vs-0 — same expression, different answer per lookup path | dispatch aligned to wasm anchor (bytes, keep, 0) | 09-14 |
| Value-define re-evaluation | top-level `const K = <expr>` re-ran its initializer on EVERY reference — array consts re-allocated per access (Poseidon RP heap trap) and diverged from interp (letrec: once) + JS (once) | memoize guard: unique zeroed slot, eval-once per tx (alloc_data content-dedupe aliased slots — alloc_memo_slot added) | 09-14 |
| jsonGetStr 2-arg footgun | `jsonGetStr(key, json)` compiled but silently scanned tx input, ignored the buffer arg | routes to the buffer scanner (json-get-str), dot-paths work | 09-14 |
| Object-span truncation | jsonGetStr on `{"o": {...}}` returned `{` — quote-close check ungated by the string flag | ook branch: depth-tracked balanced span, raw copy (no unescape) | 09-14 |
| jsonGetInt silent-0 | found-but-non-numeric (`"n": "abc"`) returned 0 — indistinguishable from real zero | no-digit → TAG_NIL (`??` fires); prefix digits still parse; literal + dynamic | 09-14 |
| extract depth-clobber (latent) | an extracted OBJECT value left depth=0 + scan_i on the closer — all LATER keys silently dropped (lisp json-extract affected too) | depth=1 restore + scan_i past closer at all 3 extraction exits | 09-14 |

### near-mock bugs — 4 total

| bug | fix | date |
|---|---|---|
| BN254 pairing stride 128B→192B | POINT_SIZE + POINT_SIZE*2 | 09-11 |
| Error chains hidden | TxOutcome.error carries full chain | 09-11 |
| Deprecated host wording | "Attempted to call deprecated..." | 09-11 |
| Embedded copy stale | full re-sync | 09-11 |

### Voting app bugs (found by user, fixed same session)

| bug | in version | fix |
|---|---|---|
| Choice visible in plaintext | v2 | encrypted_choice field |
| Choice visible at reveal | v3 | choice inside circuit (v3) / homomorphic (v4) |
| Contract reveal() didn't verify commitment | v3 | known bug, superseded by v4 |
| Tally authority trusted (single party) | v4 | documented as known limitation |

### Operational gotchas

- Top-level `const` **arrays**: memoized since 09-14 — allocate once per tx, safe in hot loops (JS module-const semantics)
- `near.jsonGetStr/Int` dynamic keys: WORKS since 09-14; known edge — whitespace BEFORE the colon (`"k" : v`) doesn't match the dynamic path
- `string + string`: common shapes (const, `storageGet ?? ""`, literals) are typed correctly since 09-12 — `strCat()` remains the safe fallback for exotic receivers
- BN254 G1 generator is **(1, 2)** — NOT (1, P-1). Cost hours to debug.
- `cargo build` can timeout in foreground — use `nohup ... &` and poll
- Disk fills: clean `target/debug/` (~16GB), check `df -h`
- `near-compile` can be stale — rebuild after frontend changes
- Account creation: keep names short (funder appended as suffix)
- snarkjs vkey key is `IC` not `VK`
- circomlib `LessThan` max is 252 bits
- `snarkjs.groth16.fullProve` in JS API is slow (60s+) — use CLI (`snarkjs groth16 prove`) instead (1.4s)

---

## Open Fronts (ranked by leverage)

### 1. 🟢 u128 without the serialization tax (the DeFi gas story — Levels 0+1 LANDED)

**The insight (2026-09-15)**: u128 values live as decimal STRINGS in the value model, so every op pays parse → limbs → compute → limbs → serialize. After the chunked to_str (9× win, landed: per-op ~80 Ggas → ~9), format conversion is still **~80% of every u128 op**. The tax is structural — kill the round-trips and it's gone. Three levels:

- **Level 0 ✅ (done, `74900a4` + zero-fix `1058dae`)**: chunked `__h_u128_to_str` — divide by 10^18 per chunk (NOT 10^19 — overflows i64::MAX, corrupts the digit formatter), ≤3 chunks, one 128-step division each. fib(185) 8→1.6 Tgas; acc(100) 8.04→1.02. Padding rule (interior chunks zero-pad to 18) pinned in `test_u128_chunked`. ⚠️ the zero fast-path shipped a NUL-string bug in 0.1.12 (tagged ptr = buffer BASE instead of the written '0' — `u128Add("0","0")` read back `"\u0000"`, trapped downstream parsers; broke the AMM/auction fixtures). Fixed in 0.1.13 + pinned (`zero_renders_zero`, all four zero-producing ops). Lesson: every op needs a ZERO case pinned, not just the big-value edges.
- **Level 1 ✅ (done, `e1658ab`)**: limb locals — a local whose every store is u128-pure (u128 arith result, ≤u128::MAX digit literal, copy of another limb local) compiles to a (lo, hi) i64 pair. Emitter-side per-function eligibility pre-scan (conservative — any impure store demotes the name function-wide); limb-aware operand paths (limb locals + nested u128 arith never stringify — full fusion of nested arithmetic trees); comparisons run on the locals; lazy materialization at true edges (Sym reads, storage writes, strcat, closure capture); lambdas materialize captured limb locals at closure creation. CRITICAL frontend piece: hoisted loop-body bindings with u128-pure inits bind `"0"` dummies instead of nil — nil poisoned eligibility and silently killed the whole optimization for the fib/accrual shape (all 6 hoist sites fixed: while ×2, for, for-of, loop_body_expr, function-level impure-init).
  - **Measured: fib(185) 2.452 → 0.925 Tgas (−62%), wasm 1441 → 1192 bytes.** Per-iter ~13.3 → ~5.0 Mgas. The predicted ~0.3 Tgas was optimistic — the floor is now loop machinery (~0.35 Tgas numeric counter/cond/guards) + the add-helper call (scratch-store → call → load ≈ 20 instr/iter).
  - **Next cut (Level 1.5, small)**: inline limb add/sub when both operands and target are limb locals (~10 instr vs ~30) — projected fib ≈ 0.7 Tgas. Only worth it if a real contract needs it.
  - **Known limits**: bigint args to helper fns still serialize at the call boundary (calling convention is one tagged i64) — Level 2 (TAG_U128 heap cells) when a real contract hits that ceiling. Params stay tagged. Cosmetic: a stored "007" materializes canonical "7".
- **Level 2 — native TAG_U128** (full elimination, days + invasive): u128 = tagged pointer to a 16-byte heap cell — flows through calls, arrays, storage with ZERO serialization anywhere; ops deref cells (~0.5-1 Ggas). Serialization only at JSON return/log/display. Touches every polymorphic consumer + interp parity + checker. Language-version-sized — do it only when a real contract hits Level 1's call-boundary tax.

**Measured floor for context (corrected 2026-09-15)**: fib-loop machinery ≈ 1.9 Mgas/iter real (the old 27 Mgas/iter figure was wrong — measured on a heavier loop shape); u128 limb-op path ≈ 3 Mgas/iter; i64 checked add ~17 Mgas was also overestimated per-instruction-wise. Re-measure before optimizing further — instruction counts ≠ gas 1:1.

**First benchmark**: fib(185) loop + an accrual loop — ✅ done (fib above; accrual covered by `mixed_generic_operand_accrues_exactly` + acc(100) budget in test_u128_chunked).

### 2. ✅ PLONK verifier COMPLETE (2026-09-15, 80ecb25) — universal setup DONE

**Shipped**: honest → OK, tampered → BAD:pairing, **53.4 Tgas** (300 cap). No per-circuit ceremony — one powers-of-tau for all circuits.

The port that taught us the most (6 distinct bugs, each oracle-pinned):
- string literals can't carry bytes ≥ 0xC0 → hexDecode byte tables
- CIOS was correct Montgomery all along (a·b·R⁻¹) — the oracle was wrong
- zero inversions via D0-scaling both pairing sides (bil. 1^D0=1) — killed the ~380-mul batch inversion
- L2/L3 need ω/ω² prefactors; d2b must NOT double-scale; d3 uses β not βξ
- RETRACTED: the suspected "compiler array-literal bug" was a one-digit typo in F_THREE_M (limb 5: 56886 vs 56866) — ciosMul computed the correct product of a mistyped constant. The compiler is faithful; no open bug (fe1ba6b). Playbook lesson: when constant-input math is wrong, diff the constants digit-by-digit against a FRESH generation, and alias-hunt (multiply against every constant in the file) before suspecting the compiler.

Files: zk/identity/plonk_verifier.ts (+gen_oracle.js oracle), tests/test_plonk_verifier.rs. Transcript exports kept for oracle diffing.

### 3. 🟡 zk-Vote v4 hardening (production trust fixes)

**What**: the homomorphic tally works but has 4 trust assumptions
**Current state**: live on testnet, e2e verified, choices never visible
**Trust gaps**:
  - Tally authority is single party → fix: 2-of-3 threshold (~2 days)
  - No proof of correct tally → fix: DLEQ proof or second circuit
  - Registry admin controls voter set → fix: multisig/DAO governance
  - Tx signer visible → fix: relayer (or accept for now)
**Priority**: 2-of-3 threshold is the most impactful and simplest

### 4. ⚪ Honk verifier port (Noir path — parked pending PLONK completion)

**Status**: stitched-wasm Grumpkin insight validated but anatomy study not done. PLONK is higher leverage (solves trust setup for ALL circuits). Revisit after PLONK.
**Pre-requisite**: PLONK verifier done, Noir still relevant to your use case

### 5. ⚪ Poseidon optimization (gas multiplier)

**What**: 29-bit limbs + sparse partial rounds → 146 → ~35-50 Tgas
**Why**: makes Merkle tree (if we build one) ergonomic
**Effort**: ~2-3 days mechanical port

### 6. ⚪ Noir / MPC / Nova (all parked)

| item | status | why parked |
|---|---|---|
| Noir via arkworks backend | dead | 2yr stale, arithmetic-only |
| Noir via Honk port | viable but expensive | 2-4 weeks, PLONK is better ROI |
| Noir via Nova | bridge doesn't exist | would need ACIR→R1CS lowering (new compiler) |
| NEAR MPC for threshold | doesn't support BN254 | wrong curve, wrong purpose |
| Quantus MPC fork | adds Dilithium not BN254 | still no threshold decryption |

---

## Architecture (the pieces and how they connect)

```
┌─────────────────────────────────────────────────────────────┐
│ USER (browser/CLI)                                          │
│   private data → witness → proof (snarkjs, 1.4s)          │
│   proof + commitments/nullifier → tx to NEAR               │
└─────────────────────────┬───────────────────────────────────┘
                          │
┌─────────────────────────▼───────────────────────────────────┐
│ NEAR CONTRACTS (lisp-rlm TS → wasm)                         │
│                                                              │
│  Groth16 verifier (LIVE, 34 Tgas):                          │
│    multiexp(IC, inputs) + pairing_check(4 pairs) → OK/BAD  │
│                                                              │
│  [IN PROGRESS] PLONK verifier (~35-160 Tgas):               │
│    keccak transcript + G1 multiexp + pairing → OK/BAD       │
│    universal setup — no per-circuit ceremony                │
│                                                              │
│  Poseidon hash (LIVE, 146 Tgas):                            │
│    CIOS 16-limb (or 9-limb optimized)                       │
│                                                              │
│  zk-Identity (LIVE): anonymous credentials                  │
│  zk-Vote v3 (LIVE): sealed ballots, reveal at close         │
│  zk-Vote v4 (LIVE): homomorphic tally — nobody sees choices │
│                                                              │
│  [all use alt_bn128 hosts: multiexp(56), sum, pairing(58)] │
└─────────────────────────┬───────────────────────────────────┘
                          │
┌─────────────────────────▼───────────────────────────────────┐
│ LOCAL DEV LOOP (near-mock)                                  │
│   compile → run → gas (0.2% of mainnet) → deploy → verify  │
│   fork mainnet state / replay any tx / calibrated fees     │
└─────────────────────────────────────────────────────────────┘
```

### Voting system evolution (what each version taught us)

```
v1: plaintext choice           → too obvious
v2: "encrypted" choice        → was actually plaintext (user caught it)
v3: choice in circuit         → visible at reveal (user caught it again)
v4: homomorphic tally         → nobody sees individual choices ✓
    trust gap: tally authority is single party
    fix: 2-of-3 threshold (2 days) or MPC (not available on NEAR)
```

### Proof system comparison (our stack)

| | Groth16 (live) | PLONK (building) | Honk (parked) |
|---|---|---|---|
| trusted setup | per-circuit ceremony | universal (one, reusable) | none |
| proof size | 200 B | ~2.1 KB | ~1-2 KB |
| verify gas | 34 Tgas | ~35-160 Tgas | ~150-250 Tgas (est) |
| circuit authoring | circom | circom (same) | Noir |
| maturity | very mature | production (Aztec v1) | evolving |

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
cd zk/identity  # or zk/vote, or any circuit dir

# 1. Compile circuit
/tmp/circom2 circuit.circom --r1cs --wasm --sym -o .

# 2. Setup (Groth16 or PLONK)
snarkjs powersoftau new bn128 12 pot12_0000.ptau
snarkjs powersoftau contribute pot12_0000.ptau pot12_0001.ptau --entropy="..."
snarkjs powersoftau prepare phase2 pot12_0001.ptau pot12_final.ptau
snarkjs groth16 setup circuit.r1cs pot12_final.ptau circuit_final.zkey
snarkjs zkey export verificationkey circuit_final.zkey vkey.json

# 3. Generate witness + prove (fast: CLI not JS API)
node make_input.js  # or prove_fast.js pattern
node circuit_js/generate_witness.js circuit_js/circuit.wasm input.json witness.wtns
snarkjs groth16 prove circuit_final.zkey witness.wtns proof.json public.json

# 4. Bridge to NEAR format
python3 ../bridge.py .   # or plonk_bridge.js for PLONK

# 5. Deploy verifier + verify
# (deploy contract, call init(vk), call verify(proof))
```

### PLONK pipeline (same circuits, different setup)
```bash
snarkjs plonk setup circuit.r1cs pot12_final.ptau circuit_plonk.zkey
snarkjs zkey export verificationkey circuit_plonk.zkey vkey_plonk.json
snarkjs plonk prove circuit_plonk.zkey witness.wtns proof_plonk.json public_plonk.json
node plonk_bridge.js
```

### Publishing
```bash
cd lisp-rlm && git add -A && git commit -m "..." && git push
cargo publish -p lisp-rlm-wasm
sleep 15 && cargo publish -p near-compile
cd ../near-mock && git add -A && git commit -m "..." && git push && cargo publish
```

⚠️ Packaging (learned 09-14, the hard way): the crate ships a **whitelist** (`include` in Cargo.toml — src/, wit/deps/, skills/, README). The repo tree is ~11k files (zk node_modules was tracked once — now untracked+ignored) and a 2.1MB/11k-file tarball reliably dies on the crates.io upload (HTTP/2 stream resets → WAF 503s). Compile-time includes outside src/ are exactly: `wit/deps/**` (wit_embed.rs, wasi/mod.rs), `skills/SKILL.md` + `skills/example-scenario.json` (near-mock bin). If you add an `include_str!` outside src/, add its target to the whitelist or the verify build breaks. cargo always force-includes README* at any depth — harmless.

### Important paths
```
/Users/j-p/dev/stuff/lisp-rlm/          — compiler + tools + tests
/Users/j-p/dev/stuff/lisp-rlm/zk/       — all zk apps + circuits + bridges
/Users/j-p/dev/stuff/lisp-rlm/schnorr/  — stitched-wasm pattern (for Honk)
/Users/j-p/dev/stuff/lisp-rlm/fixtures/ — verified contract sources
/Users/j-p/dev/stuff/lisp-rlm/tests/    — regression suite
/Users/j-p/dev/stuff/near-mock/         — the local runner
/tmp/circom2                            — circom compiler 2.2.3 binary
```
