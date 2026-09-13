# PLONK Verifier on NEAR — Operation Mapping

## What the Solidity verifier does (876 lines), mapped to our hosts

| Solidity function | What it computes | NEAR host needed | Have? |
|---|---|---|---|
| `calculateChallenges` | keccak256 Fiat-Shamir transcript → 5 scalars (beta, gamma, alpha, xi, v) | `keccak256` host | ✅ |
| `calculateLagrange` | Lagrange basis evaluation at xi (Fr arithmetic) | our CIOS (TS) | ✅ |
| `calculatePI` | Public input polynomial evaluation | our CIOS (TS) | ✅ |
| `calculateR0` | Fr arithmetic (scalars) | our CIOS (TS) | ✅ |
| `calculateD` | G1 linear combination (~8 multiexp + 3 add) | `alt_bn128_g1_multiexp` | ✅ |
| `calculateF` | G1 accumulation (5 multiexp + 1 add) | `alt_bn128_g1_multiexp` | ✅ |
| `calculateE` | G1 scalar mul (1 multiexp) | `alt_bn128_g1_multiexp` | ✅ |
| `checkPairing` | ONE pairing check with 2 pairs | `alt_bn128_pairing_check` | ✅ |

## The pairing check specifically

```
Solidity (line 813-855):
  Pair 1: (A1, X_2)   where A1 = u·W_xiw + W_xi (negated y)
  Pair 2: (B1, G2_gen) where B1 = xi·W_xi + (u·xi·w)·W_xiw + F + E

Both G2 points are STATIC:
  X_2 = from the VK (trusted setup output)
  G2_gen = constant generator

NEVER does the Solidity verifier compute a G2 point at runtime.
The "problem" I flagged earlier (needing [xi]_2) doesn't exist in
this implementation — the xi multiplication happens on the G1 side!

Our alt_bn128_pairing_check takes exactly this shape:
  input = (G1_A ‖ G2_X2 ‖ G1_B ‖ G2_gen)
  → 384 bytes → checks e(A1,X_2) · e(B1,G2_gen) == 1
```

## Estimated gas

| operation | count | cost each | total |
|---|---|---|---|
| keccak256 host calls | ~5 | 0.005 Tgas | 0.025 |
| Fr arithmetic (CIOS in TS) | ~50 muls | 0.163 Tgas | 8.2 |
| G1 multiexp calls | ~6 (batched) | 1-2 Tgas | 6-12 |
| pairing check (2 pairs) | 1 | 20 Tgas | 20 |
| storage reads (~15) | 15 | 0.06 | 0.9 |
| **TOTAL** | | | **~35-41 Tgas** |

Comparable to Groth16's 34 Tgas!

## Key differences vs Groth16

| | Groth16 | PLONK |
|---|---|---|
| trusted setup | per-circuit ceremony | universal (one, reusable) |
| proof size | 200 B | ~2.1 KB |
| verify gas | 34 Tgas | ~35-41 Tgas (est) |
| circuit change | new ceremony needed | same setup works |
| Ethereum ceremony | not reusable | reusable (powers of tau) |

## What we need to build

1. A PLONK verifier contract (~300-400 lines TS, port of the Solidity)
2. A bridge for PLONK proof/VK format (similar to bridge.py)
3. Test with real snarkjs PLONK proof

Effort estimate: **2-3 days** (mostly the TS port + keccak transcript logic)
