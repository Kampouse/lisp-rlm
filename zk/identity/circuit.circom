pragma circom 2.0.0;

include "node_modules/circomlib/circuits/poseidon.circom";
include "node_modules/circomlib/circuits/comparators.circom";
include "node_modules/circomlib/circuits/mux1.circom";

// ── Private Identity / zkKYC / Anonymous Voting ─────────────────────
//
// The universal pattern: "I'm a member of an eligible set AND [property]"
//
// For zkKYC:  set = approved users, property = credit_score > threshold
// For voting: set = registered voters, property = hasn't voted yet
//
// Both use the same circuit shape:
//   Private: identity_secret, merkle_proof (path), [property_witness]
//   Public:  merkle_root, nullifier (prevents replay), [property_claim]
//
// The merkle tree is maintained off-chain (or on-chain by a registry
// contract). The nullifier = Poseidon(identity_secret) — unique per
// identity but unlinkable to the leaf without the secret.
//
// DEPTH=4 for demo (16 members). Production: 20.

template PrivateIdentity(DEPTH) {
    // ── private inputs (the witness) ──
    signal input identity_secret;         // the user's secret
    signal input pathElements[DEPTH];     // merkle path siblings
    signal input pathIndices[DEPTH];      // merkle path directions (0/1)

    // ── public inputs (what the chain sees) ──
    signal input merkle_root;             // current eligible-set root
    signal input nullifier;               // Poseidon(identity_secret) — replay guard
    signal input claim_value;             // the claimed property (e.g. "eligible")

    // ── compute identity commitment ──
    // commitment = Poseidon(identity_secret) — this is the leaf in the tree
    component commitmentHasher = Poseidon(1);
    commitmentHasher.inputs[0] <== identity_secret;

    // ── compute nullifier ──
    // nullifier = Poseidon(identity_secret, nullifier_nonce)
    // (same secret, different nonce → different nullifier, unlinkable)
    signal nullifier_nonce;
    nullifier_nonce <== 0; // simplified: same nonce = same nullifier = one-time use
    component nullifierHasher = Poseidon(2);
    nullifierHasher.inputs[0] <== identity_secret;
    nullifierHasher.inputs[1] <== nullifier_nonce;
    nullifierHasher.out === nullifier;

    // ── merkle membership proof ──
    signal intermediate[DEPTH + 1];
    intermediate[0] <== commitmentHasher.out;

    component hashers[DEPTH];
    component muxLeft[DEPTH];
    component muxRight[DEPTH];

    for (var i = 0; i < DEPTH; i++) {
        hashers[i] = Poseidon(2);

        muxLeft[i] = Mux1();
        muxRight[i] = Mux1();

        muxLeft[i].s <== pathIndices[i];
        muxLeft[i].c[0] <== intermediate[i];
        muxLeft[i].c[1] <== pathElements[i];

        muxRight[i].s <== pathIndices[i];
        muxRight[i].c[0] <== pathElements[i];
        muxRight[i].c[1] <== intermediate[i];

        hashers[i].inputs[0] <== muxLeft[i].out;
        hashers[i].inputs[1] <== muxRight[i].out;

        intermediate[i + 1] <== hashers[i].out;
    }

    // the computed root must match the public root
    intermediate[DEPTH] === merkle_root;

    // ── property binding (for demo: always eligible) ──
    // in a real zkKYC this would be: score > threshold
    // in voting this would be: hasnt_voted
    claim_value === 1;
}

component main {public [merkle_root, nullifier, claim_value]} = PrivateIdentity(4);
