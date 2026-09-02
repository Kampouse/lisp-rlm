# lisp-rlm BLS threshold client (TypeScript)

Reference client for the #16 `bls_msig` contract (`fixtures/bls_msig.ts`).
Independently reproduces the py_ecc oracle scenario — the two must agree
byte-for-byte on `msgPoint`, `coeffs`, partials, and σ.

```
npm install
node client.ts        # node ≥23 (type stripping)
```

Crypto: @noble/curves `hashToCurve` (XMD:SHA-256_SSWU_RO, PoP DST), BigInt
Fr arithmetic, client-side Lagrange weighting (the contract's P1Sum is
unweighted — partials arrive as cᵢ·skᵢ·H(m)).

Wire ABI (testnet-verified 2026-09-02, lisp6/lisp7.kampy.testnet):
- G1: X(48B) ‖ Y(48B) sign-free, big-endian
- G2: X_im ‖ X_re ‖ Y_im ‖ Y_re (imaginary part first)
- scalars: 32B little-endian
- submit sig: `00 ‖ ser_g1(partial)` (sign flag consumed by p1_sum)
- coeffs: `hex(id,2) ‖ le32(cᵢ)` per signer, id = 1-based pk-array slot
