#!/usr/bin/env python3
"""Generate bip340.lisp — raw BIP-340 Schnorr verify in lisp-rlm.
30-bit limbs x 9, Montgomery REDC, pure-Lisp SHA-256, Jacobian points.
Step 1: simulate the limb algorithms in Python (same op-for-op bounds).
Step 2: emit the .lisp source.
"""
import csv, hashlib, math, random

LIMB = 30
NL = 9
M = (1 << LIMB) - 1

p = 2**256 - 2**32 - 977
n = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
Gx = 0x79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798
Gy = 0x483ADA7726A3C4655DA4FBFC0E1108A8FD17B448A68554199C47D08FFB10D4B8

R = 1 << 270
R2 = (R * R) % p
R1 = R % p
p_m = (p * R) % p
seven_m = (7 * R) % p
pp = (-pow(p, -1, 1 << 270)) % (1 << 270)   # REDC constant (full 270-bit)
d1 = (p + 1) // 4

def limbs(x, nl=NL):
    return [(x >> (LIMB * i)) & M for i in range(nl)]

def unlimbs(l):
    return sum(l[i] << (LIMB * i) for i in range(len(l)))

assert unlimbs(limbs(p)) == p

# ── Python simulation of the exact REDC we'll emit ──────────────────
def sim_mulmod(a_limbs, b_limbs):
    """Op-for-op simulation of the generated fe-mul. Returns 9 limbs (< p).
    Enforces the 2^61 tag ceiling on every intermediate."""
    TAG = 1 << 61
    def chk(t):
        assert 0 <= t < TAG, f"tag overflow {t}"
        return t
    # schoolbook 9x9 -> 18 limbs
    t = [0] * 19
    for i in range(NL):
        c = 0
        for j in range(NL):
            prod = a_limbs[i] * b_limbs[j]
            v = chk(t[i + j] + chk(prod) + c)
            t[i + j] = v & M
            c = v >> LIMB
            chk(c)
        k = i + NL
        while c:
            v = chk(t[k] + c)
            t[k] = v & M
            c = v >> LIMB
            k += 1
    # m = t_low * PP mod 2^270 (row-wise 9x9 schoolbook, keep low 9 limbs)
    m = [0] * NL
    for i in range(NL):
        c = 0
        for j in range(NL):
            if i + j >= NL:
                break  # only low limbs matter (mod 2^270)
            prod = t[i] * limbs_pp[j]
            v = chk(m[i + j] + chk(prod) + c)
            m[i + j] = v & M
            c = v >> LIMB
            chk(c)
    # S = T + m*p  (accumulate into t, 19 limbs)
    c = 0
    for i in range(NL):
        c = 0
        for j in range(NL):
            prod = m[i] * limbs_p[j]
            v = chk(t[i + j] + chk(prod) + c)
            t[i + j] = v & M
            c = v >> LIMB
            chk(c)
        k = i + NL
        while c:
            v = chk(t[k] + c)
            t[k] = v & M
            c = v >> LIMB
            k += 1
    # result = t[9..18] (10 limbs), then <= 2 conditional subtracts of p
    r = t[9:19]  # 10 limbs
    for _ in range(2):
        # unsigned compare r(10) >= p padded
        rp = limbs_p + [0]
        geq = 0
        for k in range(9, -1, -1):
            if r[k] != rp[k]:
                geq = 1 if r[k] > rp[k] else 0
                break
        if geq:
            br = 0
            for k in range(10):
                v = r[k] - rp[k] - br
                br = 1 if v < 0 else 0
                r[k] = (v + (1 << LIMB)) & M if br else v
            assert br == 0
    assert r[9] == 0
    return r[:9]

limbs_p = limbs(p)
limbs_n = limbs(n)
limbs_pp = limbs(pp)

# verify sim against python bigint
random.seed(42)
Rinv = pow(R, -1, p)
for _ in range(50):
    a = random.randrange(p); b = random.randrange(p)
    got = unlimbs(sim_mulmod(limbs(a), limbs(b)))
    assert got == (a * b * Rinv) % p, (a, b, got)
# Montgomery property: REDC(a_m * b_m) = (a*b)_m  — our fe-mul is REDC(raw*raw):
# to_mont(a) = REDC(a * R2); check chain
a, b = 0x1234567890ABCDEF, 0xFEDCBA0987654321
am = unlimbs(sim_mulmod(limbs(a), limbs(R2)))
bm = unlimbs(sim_mulmod(limbs(b), limbs(R2)))
prod_m = unlimbs(sim_mulmod(limbs(am), limbs(bm)))
expect_m = ((a * b) % p * R) % p
assert prod_m == expect_m
print("[sim] REDC limb algorithm verified (50 random + montgomery chain)")

# simulate d1 exponent square-and-multiply
def sim_pow(x):  # x montgomery limbs -> x^d1 montgomery
    acc = limbs(R1)
    for j in range(254, -1, -1):
        acc = sim_mulmod(acc, acc)
        if (d1 >> j) & 1:
            acc = sim_mulmod(acc, x)
    return acc

# curve sanity: y^2 = x^3+7 for G
gx_m = unlimbs(sim_mulmod(limbs(Gx), limbs(R2)))
gy_m = unlimbs(sim_mulmod(limbs(Gy), limbs(R2)))
c = unlimbs(sim_mulmod(limbs(gx_m), limbs(gx_m)))
c = unlimbs(sim_mulmod(limbs(c), limbs(gx_m)))
c = (c + seven_m) % p  # add not simulated; bigint fine here
# sqrt of montgomery-form c, then check square == c_m in mont domain
y = sim_pow(limbs(c))
y_sq_m = unlimbs(sim_mulmod(y, y))
assert y_sq_m == c % p, "sqrt failed"
# also decompress G: sqrt(gx^3+7) should give ±gy (convert out of mont first)
c2 = (Gx**3 + 7) % p
y_m = sim_pow(limbs(((c2 * R) % p)))
ynorm = unlimbs(sim_mulmod(y_m, limbs(1)))  # from_mont
assert (ynorm * ynorm) % p == c2
assert ynorm in (Gy, p - Gy), (hex(ynorm), hex(Gy))
print("[sim] sqrt via (p+1)/4 verified — decompress G matches ±Gy")
print("[sim] sqrt via (p+1)/4 verified on G")
print("[sim] all limb simulations pass")
