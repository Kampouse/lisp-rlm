# zk-squad — Threshold Anonymous Authorization ("firing squad" primitive)

**Statement proven:** "≥ T distinct members of the registered squad set approved,
each exactly once, for this squad instance" — and nothing else.

Public learns: threshold reached. Public NEVER learns: who approved, how many
beyond T, who abstained.

## Ladder

- **v0 (this spec):** trusted coordinator + one batched Groth16 proof. Pure
  composition of existing parts. Ships fast.
- **v1:** coordinator's knowledge thresholded away — approval packages
  encrypted to a FROST t-of-n aggregate key; no single party ever sees who.
- **v2:** hiding accumulator (research; not needed until v1 hurts).

## v0 flow

1. **Enroll.** Squad members register identity commitments
   `Poseidon(secret_i)` in a Merkle tree (depth 3 fits 7 shooters; root `R`
   pinned in the squad contract, alongside `instance_id`, `T`, action hash).
2. **Approve (off-chain, private).** Approver sends the *coordinator* an
   encrypted package: `{identity_secret witness, path, blinding_i}`.
3. **Trigger.** Coordinator holds packages until ≥ T, then submits ONE
   transaction: Groth16 proof + public inputs below. Contract verifies
   (existing deployed verifier), checks nullifiers unused, fires the action.
4. **After.** Nullifiers only say "T distinct identities fired once" — no
   attribution, ever.

## Circuit interface (composes PrivateIdentity + ZkVote commitment)

```circom
template SquadTrigger(T, DEPTH) {          // T = threshold slots, e.g. 5
    // private, per slot j in [0, T)
    signal input identity_secret[T];
    signal input pathElements[T][DEPTH];
    signal input pathIndices[T][DEPTH];
    signal input blinding[T];

    // public
    signal input merkle_root;              // squad set root (same as identity)
    signal input instance_id;              // binds to this execution instance
    signal input nullifier[T];             // Poseidon(secret_j, instance_id)
    signal input approval_commitment[T];   // Poseidon(1, blinding_j)

    // per slot j — exactly the PrivateIdentity + ZkVote shapes:
    //   leaf_j   = Poseidon(identity_secret_j)
    //   merkle walk ⇒ leaf_j ∈ tree(root)
    //   nullifier_j == Poseidon(identity_secret_j, instance_id)   // + dedup on-chain
    //   approval_commitment_j == Poseidon(1, blinding_j)
    //     NB: "1" is a circuit CONSTANT → statement is literally
    //     "this commitment opens to APPROVE" — no bit-proof needed.

    // disjointness gadget — t(t-1)/2 constraints, trivial:
    //   for i<j: IsZero(identity_secret_i - identity_secret_j).out === 0
    //   (forces pairwise distinct secrets ⇒ distinct members)
}
component main {public [merkle_root, instance_id, nullifier, approval_commitment]}
          = SquadTrigger(5, 3);
```

## Sizes & costs (real numbers, from the repo's deployments)

| item | value |
|---|---|
| constraints | T × identity ≈ 5 × ~800 = **~4–5k** (T=5, DEPTH=3) |
| ptau | pot12 is borderline; use **pot16** (same snarkjs flow) |
| proof | Groth16 constant size |
| on-chain verify | **33.6 Tgas** (deployed, proven — circuit size irrelevant) |
| coordinator prove time | seconds (snarkjs, <5k constraints) |

7 members ⇒ DEPTH 3 suffices. 100-member squads ⇒ DEPTH 7, still cheap.

## Trust model (v0 — read this)

- Coordinator **does** learn who approved (sees witnesses before aggregation).
  Chain + public never do. For "firing squad" semantics that's often exactly
  right (the state knows, the crowd doesn't). If not → v1 FROST.
- Double-fire prevention: nullifier set on-chain per instance. Replay across
  instances: instance_id in the nullifier domain.
- Count beyond T: sealed by construction (circuit has exactly T slots;
  releasing 6 approvals through a T=5 circuit is impossible without re-proving
  with a different circuit hash — contract pins the verifying key).

## Reused vs new

| part | status |
|---|---|
| identity membership + nullifier | ✅ zk/identity/circuit.circom verbatim shape |
| approval commitment | ✅ zk/vote v3 Poseidon(1, blinding) shape |
| Groth16 verifier on NEAR | ✅ deployed, 33.6 Tgas |
| contract trigger + nullifier state | ✅ pattern exists (vote v3 contract) |
| SquadTrigger.circom | 🆕 ~150 lines |
| assemble/prove script | 🆕 prove.js pattern |
| squad registry + trigger lisp contract | 🆕 ~200 lines |
| pot16 ptau | 🆕 one-time ceremony download |

## Effort

- Testnet demo: a focused weekend.
- Hardened v0 (registry, nullifier state, tampered-proof tests): 1–2 weeks.
- v1 FROST coordinator-less: separate 2–3 week lane after.
