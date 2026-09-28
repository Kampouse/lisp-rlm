pragma circom 2.0.0;

include "node_modules/circomlib/circuits/poseidon.circom";
include "node_modules/circomlib/circuits/comparators.circom";

// Private statement: "I know x, y (with salts sx, sy) opening commitments
// Cx = Poseidon(x, sx), Cy = Poseidon(y, sy) — and x > y."
// Private: x, y, sx, sy. Public: Cx, Cy.
//
// Matches the on-chain verifier's public-input order: inputs = [Cx, Cy].
template PrivateGreater() {
    signal input x;
    signal input y;
    signal input sx;
    signal input sy;
    signal input Cx;  // public commitment to x (constrained)
    signal input Cy;  // public commitment to y (constrained)

    component hx = Poseidon(2);
    hx.inputs[0] <== x;
    hx.inputs[1] <== sx;
    hx.out === Cx;

    component hy = Poseidon(2);
    hy.inputs[0] <== y;
    hy.inputs[1] <== sy;
    hy.out === Cy;

    component gt = GreaterThan(252);
    gt.in[0] <== x;
    gt.in[1] <== y;
    gt.out === 1;
}

component main {public [Cx, Cy]} = PrivateGreater();
