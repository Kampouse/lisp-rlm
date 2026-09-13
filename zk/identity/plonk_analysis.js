// Analyze what the snarkjs PLONK verifier actually computes
// We need to know exactly which host calls our NEAR contract needs to make
const fs = require("fs");

const vkey = JSON.parse(fs.readFileSync("vkey_plonk.json"));
const proof = JSON.parse(fs.readFileSync("proof_plonk.json"));
const pub = JSON.parse(fs.readFileSync("public_plonk.json"));

console.log("=== PLONK VERIFICATION ANATOMY ===\n");

// The PLONK verification (from the snarkjs Solidity verifier) does:
// 1. Hash transcript (proof + public inputs) to get challenges:
//    beta, gamma, alpha, xi, v (5 Fiat-Shamir challenges)
//    → uses keccak256 → WE HAVE keccak host

// 2. Compute a public input polynomial evaluation:
//    PI_eval = sum(pubInput_i * L_i(xi))
//    where L_i are Lagrange basis evaluations at xi
//    → this is scalar math in Fr → our Poseidon/CIOS territory

// 3. Group operation: compute a linear combination of G1 points
//    LHS = sum of (coefficient_i * point_i) for ~20 points
//    points: A, B, C, Z, T1, T2, T3, Wxi, Wxiw (from proof, G1)
//            + Qm, Ql, Qr, Qo, Qc, S1, S2, S3 (from VK, G1)
//    coefficients: derived from challenges + evals
//    → ONE big G1 multiexp → WE HAVE (alt_bn128_g1_multiexp)

// 4. Compute [xi]_2 (the challenge as a G2 point)
//    This is a SCALAR MUL on G2 → WE DON'T HAVE G2 MULTIEXP
//    BUT: can we restructure to avoid this?
//    Alternative: use the pairing differently

// 5. Final check: e(LHS_1, [1]_2) == e(RHS_1, [xi]_2)
//    → ONE pairing check → WE HAVE
//    BUT involves [xi]_2 which requires G2 scalar mul

console.log("Steps needed:");
console.log("  1. Keccak transcript (challenges)    → HAVE (host)");
console.log("  2. Fr field arithmetic (evals)       → HAVE (our CIOS)");
console.log("  3. G1 multiexp (~20 points)         → HAVE (host 56)");
console.log("  4. G1 sum                            → HAVE (host)");
console.log("  5. G2 scalar mul (for [xi]_2)       → MISSING ⚠️");
console.log("  6. Pairing check                     → HAVE (host 58)");
console.log();

// The G2 scalar mul problem:
// [xi]_2 = xi * G2_generator
// xi is a Fr scalar, G2_generator is a known fixed point
// We need: xi * [1]_2
//
// Options:
// A) Add a G2 multiexp host to NEAR (protocol change — not our call)
// B) Use the pairing differently: instead of e(A, [xi]_2), use e(A, [1]_2)
//    and restructure the equation... but PLONK specifically needs [xi]_2
// C) Compute [xi]_2 via G2 point addition (double-and-add) in TS:
//    xi is 254 bits → ~254 doublings + ~127 additions on G2
//    G2 point add = ~3x G1 point add (Fq2 coordinates)
//    This is ~380 G2 point ops × ~36 field muls each = ~13,680 field muls
//    at 0.163 Tgas = ~2,230 Tgas → WAY over 300 cap
// D) Split across multiple calls (multi-call verification)
// E) Use the pairing check to avoid G2 mul entirely:
//    Instead of computing [xi]_2 = xi * [1]_2,
//    use e(Wxi, [1]_2) directly and restructure the check
//    (This is what some PLONK variants do)
//
// Actually — let me look at this more carefully. The snarkjs Solidity
// verifier for PLONK uses precompile 0x08 (bn254_pairing) with specific
// G2 points. Let me check what those are.

console.log("The key insight:");
console.log("  snarkjs PLONK Solidity verifier uses pairing check:");
console.log("  e(P1, P2) == e(Q1, Q2)");
console.log("  where P2 and Q2 are G2 points");
console.log("  P2 = [1]_2 (generator, constant)");
console.log("  Q2 = X_2 * something (from trusted setup)");
console.log("  Neither requires runtime G2 scalar mul!");
console.log();
console.log("If both G2 points are STATIC (from the VK),");
console.log("we can do this with our existing pairing host!");
console.log();

// Check: what G2 points does the verification actually use?
console.log("VK G2 point (X_2):", JSON.stringify(vkey.X_2).slice(0, 60) + "…");
console.log("This is STATIC — part of the VK, precomputed at setup");
console.log();

// The actual PLONK verification from snarkjs verifier.sol:
// Uses pairing with:
//   P1 = computed from proof points (G1, via multiexp)
//   P2 = [1]_2 (G2 generator, hardcoded)
//   Q1 = computed from proof points (G1, via multiexp)
//   Q2 = [xi]_2 (G2, needs to be computed at runtime from challenge xi)
//
// So [xi]_2 IS a runtime G2 scalar mul. This IS a problem.
//
// BUT: there's a trick. We can use a different pairing arrangement:
//   Instead of e(A, [xi]_2), we compute e(xi*A, [1]_2)
//   Since xi*A can be done as G1 scalar mul (which we have!)
//   The pairing equation becomes:
//   e(xi*A, [1]_2) == e(B, X_2)
//   All G2 points are static!

console.log("=== SOLUTION ===");
console.log("Restructure the pairing to avoid G2 scalar mul:");
console.log("  Instead of:  e(A, [xi]_2) == e(B, C_2)");
console.log("  Compute:     e(xi·A, [1]_2) == e(B, C_2)");
console.log("");
console.log("  xi·A is a G1 multiexp → WE HAVE");
console.log("  [1]_2 is hardcoded     → WE HAVE (as constant)");
console.log("  e(B, C_2) both G1/G2  → pairing host");
console.log("");
console.log("ALL OPERATIONS NOW USE EXISTING HOSTS ✓");
console.log("");
console.log("Setup: universal (one ceremony, all circuits, reuse Ethereum's)");
console.log("Proof size: ~2.1 KB (vs Groth16's 200B)");
console.log("Verify cost: estimated ~40-60 Tgas (more points + keccak)");
