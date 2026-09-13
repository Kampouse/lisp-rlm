pragma circom 2.0.0;

include "node_modules/circomlib/circuits/poseidon.circom";
include "node_modules/circomlib/circuits/comparators.circom";
include "node_modules/circomlib/circuits/mux1.circom";

// ── zk-Vote v3: choice INSIDE the circuit ─────────────────────────
//
// Private:  identity_secret, merkle path, choice, blinding
// Public:   merkle_root, nullifier, choice_commitment
//
// The proof says:
//   "I'm in the voter set (Merkle membership)
//    This is my nullifier (Poseidon of my secret)
//    My choice is 0 or 1 (valid binary choice)
//    choice_commitment = Poseidon(choice, blinding)"
//
// The chain sees: root, nullifier, commitment
// The chain CANNOT see: who voted, what they voted for
//
// At reveal: voter publishes (choice, blinding), anyone can verify
// Poseidon(choice, blinding) == commitment

template ZkVote(DEPTH) {
    // ── private inputs ──
    signal input identity_secret;
    signal input pathElements[DEPTH];
    signal input pathIndices[DEPTH];
    signal input choice;              // 0 = no, 1 = yes
    signal input blinding;            // random blinding factor

    // ── public inputs ──
    signal input merkle_root;
    signal input nullifier;
    signal input choice_commitment;

    // ── identity commitment = Poseidon(secret) ──
    component commitmentHasher = Poseidon(1);
    commitmentHasher.inputs[0] <== identity_secret;

    // ── nullifier = Poseidon(secret, 0) ──
    component nullifierHasher = Poseidon(2);
    nullifierHasher.inputs[0] <== identity_secret;
    nullifierHasher.inputs[1] <== 0;
    nullifierHasher.out === nullifier;

    // ── choice commitment = Poseidon(choice, blinding) ──
    component choiceCommitter = Poseidon(2);
    choiceCommitter.inputs[0] <== choice;
    choiceCommitter.inputs[1] <== blinding;
    choiceCommitter.out === choice_commitment;

    // ── choice must be binary (0 or 1) ──
    component choiceCheck = IsZero();
    choiceCheck.in <== choice * (choice - 1);
    // choice * (choice - 1) == 0 iff choice is 0 or 1
    // IsZero outputs 1 when input is 0, so we need to flip:
    // if choice*(choice-1) == 0, IsZero.out = 1 → valid
    // we need to FORCE it to be zero, so we assert:
    choice * (choice - 1) === 0;

    // ── Merkle membership ──
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

    intermediate[DEPTH] === merkle_root;
}

component main {public [merkle_root, nullifier, choice_commitment]} = ZkVote(4);
