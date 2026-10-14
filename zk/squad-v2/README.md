# zk-squad v2 — Sealed-Tally Threshold Voting

*"Encode a yes or no after a threshold is reached, based on everyone who
voted, at a said time."* — Construction #2 skeleton (Arch A), per SPEC.md.

Implements the e-voting protocol of **ePrint 2026/1119** (Baghery–Makri–
Suryakari, SCN 2026) — Λ_indv packed pre-constructed PVSS + Schoenmakers
binary-vote OR-proof — composed with the deployed zk-identity anonymity
layer, on NEAR, **native hosts only**.

## What it does

- **n members** (voters = talliers, self-tallying; n=8 demo, t=5)
- **Anonymous enrollment**: each member registers a tallier ed25519 pubkey
  with a zk-identity proof (nullifier nonce=1). The chain never learns
  which member owns which key.
- **Sealed ballots**: during the window [t₀, t₁], every member posts
  `C = zk-proof (nonce=2) + Λ_indv blob`. The blob seals bit v inside
  `U = G^{s+v}` (plus encrypted shares yᵢ = PKᵢ^f⁽ⁱ⁾, commitments, Fiat-
  Shamir proofs). All members ballot (cover traffic) — the chain sees
  blobs it cannot read. **bn254-pairing landmine avoided: the tally group
  is curve25519 (DDH-hard, pairing-free).**
- **Deadline close**: after t₁, closers verify blobs off-chain (ms),
  homomorphically tally: `V* = Ū·S̄⁻¹ = G^Σv`, dLog-by-table → Σv.
- **Quorum open**: t distinct talliers ed25519-sign the statement
  `sha256("zk-squad-v2:close|" ‖ root ‖ V* ‖ Σv ‖ result)`; contract
  verifies each sig with the **native ed25519 host**, dedups PKs,
  checks YES/NO consistency against threshold.
- **Challenge window**, then `execute`: YES → FIRED, NO → HELD.

## Files

- `src/main.ts` — contract (init/enroll/ballot/close/dispute/execute/status)
- `lib/pvss.js` — Λ_indv on curve25519 (@noble/curves): share/verify/
  decrypt+DLEQ/homomorphic tally + self-test (`node lib/pvss.js`)
- `circuit.circom` — identity circuit v2: nullifier_nonce is now PUBLIC
  (action-separated nullifiers: enroll=1, ballot=2). 4 public signals.
- `demo.js` — full e2e: 8 enroll proofs + 8 ballot proofs + 2 scenarios
  (YES fires at Σv=5; NO holds at Σv=2) + 7 negative tests
- `3pvss-eprint-2026-1119.pdf` / `-extract.txt` — the source paper, archived

## Run

```bash
npm install                      # @noble/curves circomlibjs snarkjs
node lib/pvss.js                 # PVSS self-test (verify+tally+tamper)
export PATH=~/dev/lisp-rlm/target/release:$PATH
near-compile build               # → target/squad-v2.wasm
node demo.js                     # full lifecycle under near-mock
```

Expected tail: `ALL SCENARIOS ✅` — status A: `phase:executed, result:YES,
sum:5, fired:1` · status B: `result:NO, sum:2, fired:0`.

## Toolchain notes (2026-10-11 build log)

- `near.sha256()` returns **raw digest bytes** (not hex) — pass directly
  to the raw-byte ed25519 form; `hexDecode` only for hex sig/pk args.
- TS member `near.ed25519_verify` (underscore) lowers to the RUNTIME
  raw-byte form; camelCase `ed25519Verify` is compile-time-literals only.
- Tagged num range is ±2^60 — **ns timestamps overflow**; slice the last
  9 chars off the ns string instead of dividing (`nowSecs()` in main.ts).
- @noble/curves ed25519: scalars must be < L (clamped RFC8032 scalars can
  exceed it — reduce mod L for point ops); Point.toHex (not toRawBytes);
  Point.multiply rejects the 0 scalar (guard Lagrange coefficients).
- Schoenmakers OR-proof: the `U/G` limb belongs to **branch index 1**,
  not "the branch where v≠0" — keyed on branch, not on v.
- PVSS decrypt-DLEQ proves knowledge of sk with `Ybar = X^sk`
  (not `log_G X = log_PK Y` form).
- Lagrange at 0: `L_i(0) = Π (0−j)/(i−j)` — denominator is `(i−j)⁻¹`;
  `(j−i)⁻¹` silently flips the coefficient by (−1)^t (dLog fails).

## Status / honest caveats (for review)

- Bulletin-board model: Λ_indv + OR-proofs verified OFF-chain (every
  member runs lib/pvss.js); the chain gates on zk-membership proofs,
  nullifier dedup, window, and quorum signatures. Fraud (dishonest
  closers) is caught by universal verifiability + `dispute()` freeze —
  adjudication is manual in this skeleton.
- Demo secrets (1001–1008) are repo-public; production members generate
  secrets client-side. Tallier keys here derive from deterministic seeds.
- Dual-use keys (PVSS decrypt + ed25519 sign) accepted for the squad
  threat model; separate keys is a one-line upgrade.
- pot12 phase-2 (dev-grade, same as v0). Gas: ~33.7 Tgas per zk call,
  close ~0.11 Tgas + n sig verifies.
- Testnet deploy deliberately NOT done yet — local near-mock e2e green;
  deploy after review.
