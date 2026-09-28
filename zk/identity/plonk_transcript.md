# PLONK Fiat-Shamir Transcript — Exact Byte Ordering (from Solidity)

## Challenge derivation (each keccak256 hashes the listed values, in order, as 32-byte words)

### beta = keccak256(800 bytes):
```
Offset  Size  Content
0       32    Qm.x        (VK)
32      32    Qm.y        (VK)
64      32    Ql.x        (VK)
96      32    Ql.y        (VK)
128     32    Qr.x        (VK)
160     32    Qr.y        (VK)
192     32    Qo.x        (VK)
224     32    Qo.y        (VK)
256     32    Qc.x        (VK)
288     32    Qc.y        (VK)
320     32    S1.x        (VK)
352     32    S1.y        (VK)
384     32    S2.x        (VK)
416     32    S2.y        (VK)
448     32    S3.x        (VK)
480     32    S3.y        (VK)
512     32    pubSignal[0]  (or 0 if no public signals — padded to 3 slots)
544     32    pubSignal[1]  (or 0)
576     32    pubSignal[2]  (or 0)
608     32    proof.A.x   (proof)
640     32    proof.A.y   (proof)
672     32    proof.B.x   (proof)
704     32    proof.B.y   (proof)
736     32    proof.C.x   (proof)
768     32    proof.C.y   (proof)
```
beta = keccak256(these 800 bytes) mod q

### gamma = keccak256(beta as 32 bytes) mod q
```
Offset  Size  Content
0       32    beta (previous challenge, as 32-byte word)
```

### alpha = keccak256(128 bytes) mod q
```
Offset  Size  Content
0       32    beta
32      32    gamma
64      32    proof.Z.x
96      32    proof.Z.y
```
alpha = keccak256(128 bytes) mod q

### xi = keccak256(224 bytes) mod q
```
Offset  Size  Content
0       32    alpha
32      32    proof.T1.x
64      32    proof.T1.y
96      32    proof.T2.x
128     32    proof.T2.y
160     32    proof.T3.x
192     32    proof.T3.y
```
xi = keccak256(224 bytes) mod q

### v1 = keccak256(224 bytes) mod q
```
Offset  Size  Content
0       32    xi
32      32    proof.eval_a
64      32    proof.eval_b
96      32    proof.eval_c
128     32    proof.eval_s1
160     32    proof.eval_s2
192     32    proof.eval_zw
```
v1 = keccak256(224 bytes) mod q

### u = keccak256(128 bytes) mod q
```
Offset  Size  Content
0       32    proof.Wxi.x
32      32    proof.Wxi.y
64      32    proof.Wxiw.x
96      32    proof.Wxiw.y
```
u = keccak256(128 bytes) mod q

## CRITICAL NOTES

1. All values are 32-byte words in the SAME byte order as they appear
   in the Solidity calldata / VK storage (big-endian for Fr scalars).

2. The public signals in the beta hash are ALWAYS 3 slots (96 bytes),
   regardless of how many actual public signals exist. If nPublic < 3,
   remaining slots are zero. If nPublic > 3... (need to check —
   likely it's always exactly 3 slots based on the hard-coded offsets).

   WAIT — looking again at the Solidity:
   ```
   mstore(add(mIn, 512), calldataload(add(pPublic, 0)))
   mstore(add(mIn, 544), calldataload(add(pPublic, 32)))
   mstore(add(mIn, 576), calldataload(add(pPublic, 64)))
   ```
   It loads exactly 3 words (96 bytes) from pPublic. But calldata
   for pubSignals starts at a specific offset — if there are only
   3 signals it works; if more... need to check the calling convention.

3. keccak256 output is reduced mod q (the BN254 scalar field).
