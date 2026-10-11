# zk-squad — threshold anonymous authorization (live on testnet)

`squad.gork-agent.testnet` — 8-member roster, threshold 5.
Fired Oct 10, 2026 (tx FSNrn25YpVrRkxLBb2H6jjJvfE5tJq9UPsJWnPj4YWKJ).

Construction A from SPEC.md: roster = Poseidon Merkle tree; approvals =
zk-identity proofs (on-chain Groth16 verify, 33.7 Tgas); tally = distinct
nullifiers; fire at count >= t. Nobody learns WHICH members approved.

- `src/main.ts` — contract (init / approve / status / fired)
- `demo.js` — 8-member tree + 5 proofs + init/approve payloads
- `prove_one.js <i>` — prove member i on demand
- `SPEC.md` — full design + ladder (v0 coordinator, sealed variants)

Lessons: nonce races on rapid `near-compile call` bursts (pace 3-4s);
one count-vs-nullifier bookkeeping anomaly during the burst chaos —
soundness unaffected (fired only on 5 valid distinct proofs), receipt
audit pending (daylight).

Circuit: reuse of zk/identity (recompiled — old zkey predated circuit:
witness mismatch 3019 vs 1429; regen via pot12, `squad_circuit_final.zkey`).
