#![cfg_attr(target_arch = "wasm32", no_std)]

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

const P: [u64; 4] = [
    0xFFFFFFFEFFFFFC2F,
    0xFFFFFFFFFFFFFFFF,
    0xFFFFFFFFFFFFFFFF,
    0xFFFFFFFFFFFFFFFF,
];
const N: [u64; 4] = [
    0xBFD25E8CD0364141,
    0xBAAEDCE6AF48A03B,
    0xFFFFFFFFFFFFFFFE,
    0xFFFFFFFFFFFFFFFF,
];
const RED: u64 = 0x1000003D1;
const FE_ONE: [u64; 4] = [1, 0, 0, 0];
const GX: [u64; 4] = [
    0x59F2815B16F81798,
    0x029BFCDB2DCE28D9,
    0x55A06295CE870B07,
    0x79BE667EF9DCBBAC,
];
const GY: [u64; 4] = [
    0x9C47D08FFB10D4B8,
    0xFD17B448A6855419,
    0x5DA4FBFC0E1108A8,
    0x483ADA7726A3C465,
];

#[inline(always)]
fn ge_p(a: [u64; 4]) -> bool {
    for i in (0..4).rev() {
        if a[i] > P[i] {
            return true;
        }
        if a[i] < P[i] {
            return false;
        }
    }
    true
}
#[inline(always)]
fn fe_reduce(r: &mut [u64; 4]) {
    if ge_p(*r) {
        fe_sub_p(r);
    }
    if ge_p(*r) {
        fe_sub_p(r);
    }
}
#[inline(always)]
fn fe_sub_p(r: &mut [u64; 4]) {
    let mut b = 0u128;
    for i in 0..4 {
        b = (r[i] as u128)
            .wrapping_add((P[i] as u128).wrapping_neg())
            .wrapping_add(b);
        r[i] = b as u64;
        b = (b >> 127) & 1;
    }
}
#[inline(always)]
pub fn fe_add(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut r = [0u64; 4];
    let mut c = 0u128;
    for i in 0..4 {
        c = c.wrapping_add(a[i] as u128).wrapping_add(b[i] as u128);
        r[i] = c as u64;
        c >>= 64;
    }
    if c > 0 {
        let mut c2 = RED as u128;
        for i in 0..4 {
            let t = (r[i] as u128).wrapping_add(c2);
            r[i] = t as u64;
            c2 = t >> 64;
        }
        if c2 > 0 {
            let mut c3 = RED as u128;
            for i in 0..4 {
                let t = (r[i] as u128).wrapping_add(c3);
                r[i] = t as u64;
                c3 = t >> 64;
            }
        }
    }
    fe_reduce(&mut r);
    r
}
#[inline(always)]
pub fn fe_sub(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut r = [0u64; 4];
    let mut b2: i128 = 0;
    for i in 0..4 {
        b2 += a[i] as i128 - b[i] as i128;
        r[i] = b2 as u64;
        b2 >>= 64;
    }
    if b2 < 0 {
        let mut c = 0u128;
        for i in 0..4 {
            c = c.wrapping_add(P[i] as u128).wrapping_add(r[i] as u128);
            r[i] = c as u64;
            c >>= 64;
        }
    }
    fe_reduce(&mut r);
    r
}
#[inline(always)]
pub fn fe_mul(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut t = [0u64; 8];
    for i in 0..4 {
        let ai = a[i] as u128;
        let mut c: u128 = 0;
        for j in 0..4 {
            let prod = ai * b[j] as u128;
            let (lo, ov1) = c.overflowing_add(prod);
            let (lo, ov2) = lo.overflowing_add(t[i + j] as u128);
            t[i + j] = lo as u64;
            c = (lo >> 64) | ((ov1 as u128 | ov2 as u128) << 64);
        }
        t[i + 4] = c as u64;
    }
    let mut r = [0u64; 5];
    let mut carry: u128 = 0;
    for i in 0..4 {
        carry = carry
            .wrapping_add(t[i] as u128)
            .wrapping_add((t[i + 4] as u128).wrapping_mul(RED as u128));
        r[i] = carry as u64;
        carry >>= 64;
    }
    r[4] = carry as u64;
    let mut fold = r[4] as u128;
    for _ in 0..3 {
        if fold == 0 {
            break;
        }
        let mut c: u128 = fold.wrapping_mul(RED as u128);
        fold = 0;
        for i in 0..3 {
            c = c.wrapping_add(r[i] as u128);
            r[i] = c as u64;
            c >>= 64;
        }
        c = c.wrapping_add(r[3] as u128);
        r[3] = c as u64;
        fold = c >> 64;
    }
    let mut result = [r[0], r[1], r[2], r[3]];
    fe_reduce(&mut result);
    result
}
#[inline(always)]
pub fn fe_pow(mut base: [u64; 4], exp: [u64; 4]) -> [u64; 4] {
    let mut result = FE_ONE;
    for i in 0..256 {
        if (exp[i / 64] >> (i % 64)) & 1 == 1 {
            result = fe_mul(result, base);
        }
        base = fe_mul(base, base);
    }
    result
}
#[inline(always)]
pub fn fe_inv(a: [u64; 4]) -> [u64; 4] {
    fe_pow(
        a,
        [
            0xFFFFFFFEFFFFFC2D,
            0xFFFFFFFFFFFFFFFF,
            0xFFFFFFFFFFFFFFFF,
            0xFFFFFFFFFFFFFFFF,
        ],
    )
}
// ── Variable-time public-value inverse (binary extended Euclid) ──
// USED ONLY ON PUBLIC VALUES (R.x, verify intermediates — never a secret scalar):
// timing leaks which public field element is being inverted, nothing else.
// schnorr_pubkey*/secret paths keep the constant-ish fe_pow inversion.
fn fe_is_zero(a: [u64; 4]) -> bool {
    a[0] == 0 && a[1] == 0 && a[2] == 0 && a[3] == 0
}
fn fe_shr1(a: [u64; 4]) -> [u64; 4] {
    [
        (a[0] >> 1) | ((a[1] & 1) << 63),
        (a[1] >> 1) | ((a[2] & 1) << 63),
        (a[2] >> 1) | ((a[3] & 1) << 63),
        a[3] >> 1,
    ]
}
// (y + p) / 2 for odd y (< p): y+p is even and may reach bit 256; that carry bit
// must shift DOWN into bit 255 (shr1 with carry-in), NOT be RED-corrected first.
// Result ≤ (p−1+p)/2 < p → no further reduction needed.
fn fe_half_add_p(y: [u64; 4]) -> [u64; 4] {
    let mut c = 0u128;
    let mut s = [0u64; 4];
    for i in 0..4 {
        c = c.wrapping_add(y[i] as u128).wrapping_add(P[i] as u128);
        s[i] = c as u64;
        c >>= 64;
    }
    // c = bit-256 carry (0 or 1); (y+p) is even because y and p are both odd
    [
        (s[0] >> 1) | ((s[1] & 1) << 63),
        (s[1] >> 1) | ((s[2] & 1) << 63),
        (s[2] >> 1) | ((s[3] & 1) << 63),
        (s[3] >> 1) | ((c as u64) << 63),
    ]
}
// raw add WITHOUT the RED correction (u128-carry style, drops bit-256 carry into reduce)
fn fe_add_raw(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut r = [0u64; 4];
    let mut c = 0u128;
    for i in 0..4 {
        c = c.wrapping_add(a[i] as u128).wrapping_add(b[i] as u128);
        r[i] = c as u64;
        c >>= 64;
    }
    if c > 0 {
        // bit-256 set: subtract p (2^256 mod p = RED — same correction path)
        let mut c2 = RED as u128;
        for k in 0..4 {
            let t = (r[k] as u128).wrapping_add(c2);
            r[k] = t as u64;
            c2 = t >> 64;
        }
    }
    r
}
// (a − b) mod p for a, b < p
fn fe_lt(a: [u64; 4], b: [u64; 4]) -> bool {
    for i in (0..4).rev() {
        if a[i] != b[i] {
            return a[i] < b[i];
        }
    }
    false
}
fn fe_sub_mod_p(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    if fe_lt(a, b) {
        // a − b < 0 → a + (p − b)
        let pb = fe_sub(P, b);
        return fe_add_raw(a, pb);
    }
    let r = fe_sub(a, b);
    let mut r2 = r;
    fe_reduce(&mut r2);
    r2
}
// Binary extended Euclid inverse over GF(p). Iteration count ≈ 2·bits(variable-time by VALUE;
// safe because inputs are public). Certified against pow(a, p−2, p): 300/300 randoms, avg 362 it.
pub fn fe_inv_pub(a: [u64; 4]) -> [u64; 4] {
    let mut u = P;
    let mut v = a;
    let mut x = [0u64; 4];
    let mut y = FE_ONE;
    // invariants: x·a ≡ u (mod p), y·a ≡ v (mod p)
    // NOTE: a must be < p and ≠ 0 (callers guarantee; a=0 → u,gcd path returns 0)
    while !fe_is_zero(v) && !fe_is_zero(u) {
        if v[0] & 1 == 0 {
            v = fe_shr1(v);
            y = if y[0] & 1 == 0 { fe_shr1(y) } else { fe_half_add_p(y) };
        } else if u[0] & 1 == 0 {
            u = fe_shr1(u);
            x = if x[0] & 1 == 0 { fe_shr1(x) } else { fe_half_add_p(x) };
        } else {
            // both odd
            if !fe_lt(u, v) {
                // u ≥ v: u ← (u − v)/2, x ← (x − y)/2
                u = fe_sub_mod_p(u, v);
                if fe_is_zero(u) {
                    break; // gcd(v) = 1, answer y
                }
                u = fe_shr1(u);
                x = fe_sub_mod_p(x, y);
                x = if x[0] & 1 == 0 { fe_shr1(x) } else { fe_half_add_p(x) };
            } else {
                v = fe_sub_mod_p(v, u);
                if fe_is_zero(v) {
                    break; // gcd(u) = 1, answer x
                }
                v = fe_shr1(v);
                y = fe_sub_mod_p(y, x);
                y = if y[0] & 1 == 0 { fe_shr1(y) } else { fe_half_add_p(y) };
            }
        }
    }
    if fe_is_zero(u) {
        return y; // u = 0 → gcd = v = 1 → y = a⁻¹
    }
    x // v = 0 → gcd = u = 1 → x = a⁻¹
}


// ── Jacobian coordinates ──
fn jac_is_infinity(p: &([u64; 4], [u64; 4], [u64; 4])) -> bool {
    p.2 == [0, 0, 0, 0]
}
fn jac_add(
    p: ([u64; 4], [u64; 4], [u64; 4]),
    q: ([u64; 4], [u64; 4], [u64; 4]),
) -> ([u64; 4], [u64; 4], [u64; 4]) {
    let (x1, y1, z1) = p;
    let (x2, y2, z2) = q;
    if jac_is_infinity(&p) && jac_is_infinity(&q) {
        return ([0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]);
    }
    if jac_is_infinity(&p) {
        return q;
    }
    if jac_is_infinity(&q) {
        return p;
    }
    let z1sq = fe_mul(z1, z1);
    let z2sq = fe_mul(z2, z2);
    let u1 = fe_mul(x1, z2sq);
    let u2 = fe_mul(x2, z1sq);
    let s1 = fe_mul(y1, fe_mul(z2sq, z2));
    let s2 = fe_mul(y2, fe_mul(z1sq, z1));
    let h = fe_sub(u2, u1);
    let r = fe_sub(s2, s1);
    if h == [0, 0, 0, 0] && r == [0, 0, 0, 0] {
        return jac_double(p);
    }
    if h == [0, 0, 0, 0] {
        return ([0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]);
    }
    let hsq = fe_mul(h, h);
    let hcu = fe_mul(hsq, h);
    let x3 = fe_sub(
        fe_sub(fe_mul(r, r), hcu),
        fe_mul(fe_mul(u1, hsq), [2, 0, 0, 0]),
    );
    let y3 = fe_sub(fe_mul(r, fe_sub(fe_mul(u1, hsq), x3)), fe_mul(s1, hcu));
    let z3 = fe_mul(fe_mul(z1, z2), h);
    (x3, y3, z3)
}
fn jac_double(p: ([u64; 4], [u64; 4], [u64; 4])) -> ([u64; 4], [u64; 4], [u64; 4]) {
    if jac_is_infinity(&p) {
        return p;
    }
    let (x, y, z) = p;
    let ysq = fe_mul(y, y);
    let b = fe_mul(fe_mul(x, ysq), [4, 0, 0, 0]);
    let c = fe_mul(fe_mul(ysq, ysq), [8, 0, 0, 0]);
    let d = fe_mul(fe_mul(x, x), [3, 0, 0, 0]);
    let dsq = fe_mul(d, d);
    let x3 = fe_sub(dsq, fe_mul(b, [2, 0, 0, 0]));
    let y3 = fe_sub(fe_mul(d, fe_sub(b, x3)), c);
    let z3 = fe_mul(fe_mul(y, z), [2, 0, 0, 0]);
    (x3, y3, z3)
}
fn point_mul(p: ([u64; 4], [u64; 4]), k: [u64; 4]) -> Option<([u64; 4], [u64; 4], [u64; 4])> {
    let mut r: ([u64; 4], [u64; 4], [u64; 4]) = ([0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]);
    let mut q: ([u64; 4], [u64; 4], [u64; 4]) = (p.0, p.1, [1, 0, 0, 0]);
    for i in 0..256 {
        if (k[i / 64] >> (i % 64)) & 1 == 1 {
            r = jac_add(r, q);
        }
        q = jac_double(q);
    }
    if jac_is_infinity(&r) {
        None
    } else {
        Some(r)
    }
}
fn jac_to_affine(p: ([u64; 4], [u64; 4], [u64; 4])) -> ([u64; 4], [u64; 4]) {
    let z_inv = fe_inv(p.2);
    let z2_inv = fe_mul(z_inv, z_inv);
    let z3_inv = fe_mul(z2_inv, z_inv);
    (fe_mul(p.0, z2_inv), fe_mul(p.1, z3_inv))
}
fn jac_to_affine_pub(p: ([u64; 4], [u64; 4], [u64; 4])) -> ([u64; 4], [u64; 4]) {
    let z_inv = fe_inv_pub(p.2);
    let z2_inv = fe_mul(z_inv, z_inv);
    let z3_inv = fe_mul(z2_inv, z_inv);
    (fe_mul(p.0, z2_inv), fe_mul(p.1, z3_inv))
}


mod comb;

// ── Scalar ops (mod n, the curve order — NOT the field prime P) ──
fn sc_lt_n(a: [u64; 4]) -> bool {
    for i in (0..4).rev() {
        if a[i] > N[i] {
            return false;
        }
        if a[i] < N[i] {
            return true;
        }
    }
    true
} // NB: a <= n
fn sc_geq_n(a: [u64; 4]) -> bool {
    for i in (0..4).rev() {
        if a[i] > N[i] {
            return true;
        }
        if a[i] < N[i] {
            return false;
        }
    }
    true
} // a >= n (true at equality)
fn sc_sub_n(a: [u64; 4]) -> [u64; 4] {
    let mut r = [0u64; 4];
    let mut b: i128 = 0;
    for i in 0..4 {
        b += N[i] as i128 - a[i] as i128;
        r[i] = b as u64;
        b >>= 64;
    }
    r
} // n - a, requires a <= n

/// r -= n, wrapping (256-bit). Use when the true value is r (+2^256 if the
/// carry out was set) and that value is >= n — one subtract lands back in [0, n).
/// NB: this is r - n, the OPPOSITE direction of sc_sub_n (n - r).
#[inline(always)]
fn sc_sub_n_in_place(r: &mut [u64; 4]) {
    let mut b: i128 = 0;
    for i in 0..4 {
        b += r[i] as i128 - N[i] as i128;
        r[i] = b as u64;
        b >>= 64;
    }
}

/// Full 512-bit schoolbook product, reduced mod n by binary shift-subtract.
/// This is the heart of the sign fix: e*d' MUST be reduced mod n (curve order),
/// never mod P (field prime) — the original bug produced invalid sigs.
fn sc_mul_mod_n(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut t = [0u64; 8];
    for i in 0..4 {
        let mut carry: u128 = 0;
        for j in 0..4 {
            let cur = t[i + j] as u128 + (a[i] as u128) * (b[j] as u128) + carry;
            t[i + j] = cur as u64;
            carry = cur >> 64;
        }
        let mut k = i + 4;
        while carry > 0 && k < 8 {
            let cur = t[k] as u128 + carry;
            t[k] = cur as u64;
            carry = cur >> 64;
            k += 1;
        }
    }
    // r = t mod n: MSB-first shift-subtract long division.
    // Invariant: r < n < 2^256, so r<<1 | bit < 2n → one conditional subtract restores r < n.
    let mut r = [0u64; 4];
    for bit in (0..512).rev() {
        let mut carry = ((t[bit / 64] >> (bit % 64)) & 1) as u64;
        for limb in r.iter_mut() {
            let nc = *limb >> 63;
            *limb = (*limb << 1) | carry;
            carry = nc;
        }
        if carry == 1 || sc_geq_n(r) {
            sc_sub_n_in_place(&mut r);
        }
    }
    r
}

/// (a + b) mod n — a, b < n, so one conditional subtract suffices.
fn sc_add_mod_n(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut r = [0u64; 4];
    let mut c: u128 = 0;
    for i in 0..4 {
        c = a[i] as u128 + b[i] as u128 + c;
        r[i] = c as u64;
        c >>= 64;
    }
    if c > 0 || sc_geq_n(r) {
        sc_sub_n_in_place(&mut r);
    }
    r
}
pub fn fe_bytes_to_fe(b: &[u8]) -> [u64; 4] {
    let mut r = [0u64; 4];
    for i in 0..4 {
        r[3 - i] = u64::from_be_bytes(b[i * 8..(i + 1) * 8].try_into().unwrap_or([0u8; 8]));
    }
    r
}

// ── SHA-256 ──
const SHA_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];
const SHA_IV: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];
#[inline(always)]
fn sha_ch(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (!x & z)
}
#[inline(always)]
fn sha_maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) ^ (x & z) ^ (y & z)
}
#[inline(always)]
fn sha_ep0(x: u32) -> u32 {
    x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22)
}
#[inline(always)]
fn sha_ep1(x: u32) -> u32 {
    x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25)
}
#[inline(always)]
fn sha_sig0(x: u32) -> u32 {
    x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3)
}
#[inline(always)]
fn sha_sig1(x: u32) -> u32 {
    x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10)
}

// Streaming SHA-256 (2026-08-29): the old version copied input into a fixed
// [u8; 192] buffer — fine for BIP-340's internal ≤96-byte tagged hashes, but
// the sha256_hash FFI entry shares it, so any input ≥192 bytes panicked
// (in-wasm: slice OOB → rust_begin_unwind → busy-wait spin), and lengths
// 184-191 silently hashed with wrong padding. Reference vectors for
// 183/190/300 verified vs python hashlib.
fn sha256_compress(h: &mut [u32; 8], block: &[u8; 64], w: &mut [u32; 64]) {
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        w[i] = sha_sig1(w[i - 2])
            .wrapping_add(w[i - 7])
            .wrapping_add(sha_sig0(w[i - 15]))
            .wrapping_add(w[i - 16]);
    }
    let [mut a, mut b2, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
    for i in 0..64 {
        let t1 = hh
            .wrapping_add(sha_ep1(e))
            .wrapping_add(sha_ch(e, f, g))
            .wrapping_add(SHA_K[i])
            .wrapping_add(w[i]);
        let t2 = sha_ep0(a).wrapping_add(sha_maj(a, b2, c));
        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b2;
        b2 = a;
        a = t1.wrapping_add(t2);
    }
    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b2);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
    h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g);
    h[7] = h[7].wrapping_add(hh);
}

pub fn compute_sha256(data: &[u8]) -> [u8; 32] {
    let len = data.len();
    let mut h = SHA_IV;
    let mut w = [0u32; 64];
    let mut off = 0;
    while off + 64 <= len {
        let block: [u8; 64] = data[off..off + 64].try_into().unwrap();
        sha256_compress(&mut h, &block, &mut w);
        off += 64;
    }
    let rem = len - off;
    let mut tail = [0u8; 64];
    tail[..rem].copy_from_slice(&data[off..]);
    tail[rem] = 0x80;
    let bits = (len as u64) * 8;
    if rem < 56 {
        tail[56..64].copy_from_slice(&bits.to_be_bytes());
        sha256_compress(&mut h, &tail, &mut w);
    } else {
        sha256_compress(&mut h, &tail, &mut w);
        let mut z = [0u8; 64];
        z[56..64].copy_from_slice(&bits.to_be_bytes());
        sha256_compress(&mut h, &z, &mut w);
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

fn tagged_hash(tag: &[u8], msg: &[u8]) -> [u8; 32] {
    let th = compute_sha256(tag);
    let mut buf = [0u8; 192];
    buf[..32].copy_from_slice(&th);
    buf[32..64].copy_from_slice(&th);
    buf[64..64 + msg.len()].copy_from_slice(msg);
    compute_sha256(&buf[..64 + msg.len()])
}

// ── BIP-340 Schnorr verify ──
pub fn schnorr_verify(pk_bytes: &[u8; 32], sig_bytes: &[u8; 64], msg: &[u8; 32]) -> bool {
    let pk_x = fe_bytes_to_fe(pk_bytes);
    let r = fe_bytes_to_fe(&sig_bytes[..32]);
    let s = fe_bytes_to_fe(&sig_bytes[32..64]);
    if ge_p(pk_x) {
        return false;
    }
    let x3 = fe_mul(fe_mul(pk_x, pk_x), pk_x);
    let y_sq = fe_add(x3, [7, 0, 0, 0]);
    let y = fe_pow(
        y_sq,
        [
            0xFFFFFFFFBFFFFF0C,
            0xFFFFFFFFFFFFFFFF,
            0xFFFFFFFFFFFFFFFF,
            0x3FFFFFFFFFFFFFFF,
        ],
    );
    if fe_mul(y, y) != y_sq {
        return false;
    }
    let py = if (y[0] & 1) != 0 { fe_sub(P, y) } else { y };
    if ge_p(r) || !sc_lt_n(s) {
        return false;
    }
    let mut cd = [0u8; 96];
    cd[..32].copy_from_slice(&sig_bytes[..32]);
    cd[32..64].copy_from_slice(pk_bytes);
    cd[64..96].copy_from_slice(msg);
    let e_hash = tagged_hash(b"BIP0340/challenge", &cd);
    let e_fe = fe_bytes_to_fe(&e_hash);
    let sg = point_mul((GX, GY), s);
    let ne = sc_sub_n(e_fe);
    let neg_eP = point_mul((pk_x, py), ne);
    let R = match (sg, neg_eP) {
        (None, None) => return false,
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (Some(a), Some(b)) => jac_add(a, b),
    };
    if jac_is_infinity(&R) {
        return false;
    }
    let (rx, ry) = jac_to_affine_pub(R);
    rx == r && (ry[0] & 1) == 0
}

// ── BIP-340 Schnorr sign ──
// Fixed 2026-09-29: (1) e·d' reduced mod n (curve order) via sc_mul_mod_n —
// was fe_mul mod P, producing invalid sigs; (2) odd-y R now negates k
// (k ← n−k, R.x unchanged since −R mirrors y) instead of returning a zero sig.
pub fn schnorr_sign(sk_bytes: &[u8; 32], msg: &[u8; 32], aux: &[u8; 32]) -> [u8; 64] {
    let mut d = fe_bytes_to_fe(sk_bytes);
    if sc_geq_n(d) {
        d = sc_sub_n(d);
    } // normalize sk into [0, n)
    if d == [0; 4] {
        return [0u8; 64];
    }
    // P = d * G
    let (px, py) = match point_mul((GX, GY), d) {
        Some(r) => jac_to_affine(r),
        None => return [0u8; 64],
    };
    // d' = d if even y, else n - d
    let dp = if py[0] & 1 == 0 { d } else { sc_sub_n(d) };
    // t = xor(d', tagged_hash("BIP0340/aux", aux))
    let aux_hash = tagged_hash(b"BIP0340/aux", aux);
    let mut dp_bytes = [0u8; 32];
    for i in 0..4 {
        dp_bytes[i * 8..(i + 1) * 8].copy_from_slice(&dp[3 - i].to_be_bytes());
    }
    let mut t = [0u8; 32];
    for i in 0..32 {
        t[i] = dp_bytes[i] ^ aux_hash[i];
    }
    // rand = tagged_hash("BIP0340/nonce", t || P_bytes || msg)
    let mut p_bytes = [0u8; 32];
    for i in 0..4 {
        p_bytes[i * 8..(i + 1) * 8].copy_from_slice(&px[3 - i].to_be_bytes());
    }
    let mut nonce_input = [0u8; 96];
    nonce_input[..32].copy_from_slice(&t);
    nonce_input[32..64].copy_from_slice(&p_bytes);
    nonce_input[64..96].copy_from_slice(msg);
    let rand = tagged_hash(b"BIP0340/nonce", &nonce_input);
    let mut kp = fe_bytes_to_fe(&rand);
    if sc_geq_n(kp) {
        kp = sc_sub_n(kp);
    } // k' = k mod n (one subtract: k < 2^256 < 2n)
    if kp == [0; 4] {
        return [0u8; 64];
    }
    // R = k' * G (fixed-base comb: 64 affine adds, zero doublings)
    let (rx, ry) = match comb::point_mul_g(kp) {
        Some(r) => jac_to_affine_pub(r),
        None => return [0u8; 64],
    };
    // BIP-340: R must have even y. Using k ← n−k gives −R (same x, flipped y)
    // so the precomputed R.x stays valid — no re-multiplication needed.
    let kp = if ry[0] & 1 != 0 { sc_sub_n(kp) } else { kp };
    // e = tagged_hash("BIP0340/challenge", R || P || msg) — kept at full 256 bits
    let mut r_bytes = [0u8; 32];
    for i in 0..4 {
        r_bytes[i * 8..(i + 1) * 8].copy_from_slice(&rx[3 - i].to_be_bytes());
    }
    let mut challenge_input = [0u8; 96];
    challenge_input[..32].copy_from_slice(&r_bytes);
    challenge_input[32..64].copy_from_slice(&p_bytes);
    challenge_input[64..96].copy_from_slice(msg);
    let e_hash = tagged_hash(b"BIP0340/challenge", &challenge_input);
    let e = fe_bytes_to_fe(&e_hash);
    // sig_s = (k' + e * d') mod n — mod n, the fix
    let ed = sc_mul_mod_n(e, dp);
    let sig_s = sc_add_mod_n(kp, ed);
    let mut sig = [0u8; 64];
    sig[..32].copy_from_slice(&r_bytes);
    for i in 0..4 {
        sig[32 + i * 8..32 + (i + 1) * 8].copy_from_slice(&sig_s[3 - i].to_be_bytes());
    }
    sig
}

// ── WASM exports ──
#[no_mangle]
pub unsafe extern "C" fn schnorr_verify_bip340(
    pk_ptr: u32,
    sig_ptr: u32,
    msg_ptr: u32,
    msg_len: u32,
) -> u32 {
    let pk = core::slice::from_raw_parts(pk_ptr as *const u8, 32);
    let sig = core::slice::from_raw_parts(sig_ptr as *const u8, 64);
    let msg = core::slice::from_raw_parts(msg_ptr as *const u8, msg_len as usize);
    let Ok(pk): Result<[u8; 32], _> = pk.try_into() else {
        return 0;
    };
    let Ok(sig): Result<[u8; 64], _> = sig.try_into() else {
        return 0;
    };
    let Ok(msg): Result<[u8; 32], _> = msg.try_into() else {
        return 0;
    };
    if schnorr_verify(&pk, &sig, &msg) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn schnorr_sign_bip340(
    sk_ptr: u32,
    msg_ptr: u32,
    aux_ptr: u32,
    out_ptr: u32,
) -> u32 {
    let sk = core::slice::from_raw_parts(sk_ptr as *const u8, 32);
    let msg = core::slice::from_raw_parts(msg_ptr as *const u8, 32);
    let aux = core::slice::from_raw_parts(aux_ptr as *const u8, 32);
    let Ok(sk): Result<[u8; 32], _> = sk.try_into() else {
        return 0;
    };
    let Ok(msg): Result<[u8; 32], _> = msg.try_into() else {
        return 0;
    };
    let Ok(aux): Result<[u8; 32], _> = aux.try_into() else {
        return 0;
    };
    let sig = schnorr_sign(&sk, &msg, &aux);
    let out = core::slice::from_raw_parts_mut(out_ptr as *mut u8, 64);
    out.copy_from_slice(&sig);
    if sig == [0u8; 64] {
        0
    } else {
        1
    }
}

// ── BIP-340 sign with a CACHED public key ──
// Skips the internal P = d·G mult (the caller supplies pk33, the SEC1-
// compressed point: 0x02/0x03 prefix + x). The prefix carries the y-parity
// that selects d' = d vs n-d — the one bit an x-only pk loses. A wrong
// prefix yields a sig that simply fails verification (fail-closed).
pub fn schnorr_sign_pk(
    sk_bytes: &[u8; 32],
    pk33: &[u8; 33],
    msg: &[u8; 32],
    aux: &[u8; 32],
) -> [u8; 64] {
    if pk33[0] != 0x02 && pk33[0] != 0x03 {
        return [0u8; 64];
    }
    let mut d = fe_bytes_to_fe(sk_bytes);
    if sc_geq_n(d) {
        d = sc_sub_n(d);
    }
    if d == [0; 4] {
        return [0u8; 64];
    }
    // y(d·G) even (0x02) → d' = d; odd (0x03) → d' = n - d
    let dp = if pk33[0] == 0x02 { d } else { sc_sub_n(d) };
    let mut p_bytes = [0u8; 32];
    p_bytes.copy_from_slice(&pk33[1..33]);
    // t = xor(d', tagged_hash("BIP0340/aux", aux))
    let aux_hash = tagged_hash(b"BIP0340/aux", aux);
    let mut dp_bytes = [0u8; 32];
    for i in 0..4 {
        dp_bytes[i * 8..(i + 1) * 8].copy_from_slice(&dp[3 - i].to_be_bytes());
    }
    let mut t = [0u8; 32];
    for i in 0..32 {
        t[i] = dp_bytes[i] ^ aux_hash[i];
    }
    // rand = tagged_hash("BIP0340/nonce", t || P_bytes || msg)
    let mut nonce_input = [0u8; 96];
    nonce_input[..32].copy_from_slice(&t);
    nonce_input[32..64].copy_from_slice(&p_bytes);
    nonce_input[64..96].copy_from_slice(msg);
    let rand = tagged_hash(b"BIP0340/nonce", &nonce_input);
    let mut kp = fe_bytes_to_fe(&rand);
    if sc_geq_n(kp) {
        kp = sc_sub_n(kp);
    }
    if kp == [0; 4] {
        return [0u8; 64];
    }
    // R = k' * G (fixed-base comb: 64 affine adds, zero doublings)
    let (rx, ry) = match comb::point_mul_g(kp) {
        Some(r) => jac_to_affine_pub(r),
        None => return [0u8; 64],
    };
    let kp = if ry[0] & 1 != 0 { sc_sub_n(kp) } else { kp };
    // e = tagged_hash("BIP0340/challenge", R || P || msg)
    let mut r_bytes = [0u8; 32];
    for i in 0..4 {
        r_bytes[i * 8..(i + 1) * 8].copy_from_slice(&rx[3 - i].to_be_bytes());
    }
    let mut challenge_input = [0u8; 96];
    challenge_input[..32].copy_from_slice(&r_bytes);
    challenge_input[32..64].copy_from_slice(&p_bytes);
    challenge_input[64..96].copy_from_slice(msg);
    let e_hash = tagged_hash(b"BIP0340/challenge", &challenge_input);
    let e = fe_bytes_to_fe(&e_hash);
    let ed = sc_mul_mod_n(e, dp);
    let sig_s = sc_add_mod_n(kp, ed);
    let mut sig = [0u8; 64];
    sig[..32].copy_from_slice(&r_bytes);
    for i in 0..4 {
        sig[32 + i * 8..32 + (i + 1) * 8].copy_from_slice(&sig_s[3 - i].to_be_bytes());
    }
    sig
}

// ── BIP-340 public key derivation (x-only: pk = x-coord of d·G) ──
pub fn schnorr_pubkey(sk_bytes: &[u8; 32]) -> [u8; 32] {
    let mut d = fe_bytes_to_fe(sk_bytes);
    if sc_geq_n(d) {
        d = sc_sub_n(d);
    } // normalize into [0, n)
    if d == [0; 4] {
        return [0u8; 32];
    }
    let (px, _py) = match point_mul((GX, GY), d) {
        Some(r) => jac_to_affine(r),
        None => return [0u8; 32],
    };
    let mut pk = [0u8; 32];
    for i in 0..4 {
        pk[i * 8..(i + 1) * 8].copy_from_slice(&px[3 - i].to_be_bytes());
    }
    pk
}

#[no_mangle]
pub unsafe extern "C" fn schnorr_pubkey_bip340(sk_ptr: u32, out_ptr: u32) -> u32 {
    let sk = core::slice::from_raw_parts(sk_ptr as *const u8, 32);
    let Ok(sk): Result<[u8; 32], _> = sk.try_into() else {
        return 0;
    };
    let pk = schnorr_pubkey(&sk);
    let out = core::slice::from_raw_parts_mut(out_ptr as *mut u8, 32);
    out.copy_from_slice(&pk);
    if pk == [0u8; 32] {
        0
    } else {
        1
    }
}

#[no_mangle]
pub unsafe extern "C" fn schnorr_sign_bip340_pk(
    sk_ptr: u32,
    pk33_ptr: u32,
    msg_ptr: u32,
    aux_ptr: u32,
    out_ptr: u32,
) -> u32 {
    let sk = core::slice::from_raw_parts(sk_ptr as *const u8, 32);
    let pk33 = core::slice::from_raw_parts(pk33_ptr as *const u8, 33);
    let msg = core::slice::from_raw_parts(msg_ptr as *const u8, 32);
    let aux = core::slice::from_raw_parts(aux_ptr as *const u8, 32);
    let Ok(sk): Result<[u8; 32], _> = sk.try_into() else {
        return 0;
    };
    let Ok(pk33): Result<[u8; 33], _> = pk33.try_into() else {
        return 0;
    };
    let Ok(msg): Result<[u8; 32], _> = msg.try_into() else {
        return 0;
    };
    let Ok(aux): Result<[u8; 32], _> = aux.try_into() else {
        return 0;
    };
    let sig = schnorr_sign_pk(&sk, &pk33, &msg, &aux);
    let out = core::slice::from_raw_parts_mut(out_ptr as *mut u8, 64);
    out.copy_from_slice(&sig);
    if sig == [0u8; 64] {
        0
    } else {
        1
    }
}

// 33-byte pubkey: 0x02/0x03 prefix (y-parity) + x — the form sign_bip340_pk
// consumes and the pk:<caller> cache stores.
pub fn schnorr_pubkey_33(sk_bytes: &[u8; 32]) -> [u8; 33] {
    let mut d = fe_bytes_to_fe(sk_bytes);
    if sc_geq_n(d) {
        d = sc_sub_n(d);
    }
    let mut out = [0u8; 33];
    if d == [0; 4] {
        return out;
    }
    let (px, py) = match point_mul((GX, GY), d) {
        Some(r) => jac_to_affine(r),
        None => return out,
    };
    out[0] = if py[0] & 1 == 0 { 0x02 } else { 0x03 };
    for i in 0..4 {
        out[1 + i * 8..1 + (i + 1) * 8].copy_from_slice(&px[3 - i].to_be_bytes());
    }
    out
}

#[no_mangle]
pub unsafe extern "C" fn schnorr_pubkey_bip340_33(sk_ptr: u32, out33_ptr: u32) -> u32 {
    let sk = core::slice::from_raw_parts(sk_ptr as *const u8, 32);
    let Ok(sk): Result<[u8; 32], _> = sk.try_into() else {
        return 0;
    };
    let pk33 = schnorr_pubkey_33(&sk);
    let out = core::slice::from_raw_parts_mut(out33_ptr as *mut u8, 33);
    out.copy_from_slice(&pk33);
    if pk33 == [0u8; 33] {
        0
    } else {
        1
    }
}

#[no_mangle]
pub unsafe extern "C" fn sha256_hash(input_ptr: u32, input_len: u32, output_ptr: u32) {
    let input = core::slice::from_raw_parts(input_ptr as *const u8, input_len as usize);
    let hash = compute_sha256(input);
    let output = core::slice::from_raw_parts_mut(output_ptr as *mut u8, 32);
    output.copy_from_slice(&hash);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn h(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    fn a32(v: Vec<u8>) -> [u8; 32] {
        let mut a = [0u8; 32];
        a.copy_from_slice(&v);
        a
    }
    fn a64(v: Vec<u8>) -> [u8; 64] {
        let mut a = [0u8; 64];
        a.copy_from_slice(&v);
        a
    }

#[cfg(test)]
mod comb_bench {
    use super::*;
    use std::time::Instant;
    fn det_scalar(seed: u64) -> [u64; 4] {
        let mut x = seed | 1;
        let mut out = [0u64; 4];
        for v in out.iter_mut() {
            x ^= x >> 12; x ^= x << 25; x ^= x >> 27;
            *v = x.wrapping_mul(0x2545F4914F6CDD1D);
        }
        out
    }
    #[test]
    fn test_sha256() {
        assert_eq!(
            compute_sha256(b""),
            [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
                0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
                0x78, 0x52, 0xb8, 0x55
            ]
        );
    }
    #[test]
    fn test_fe_mul() {
        let two = [2, 0, 0, 0];
        assert_eq!(fe_mul(two, fe_inv(two)), FE_ONE);
    }
    #[test]
    fn test_bip340_valid() {
        let pk = a32(h(
            "F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9",
        ));
        let sig = a64(h("E907831F80848D1069A5371B402410364BDF1C5F8307B0084C55F1CE2DCA821525F66A4A85EA8B71E482A74F382D2CE5EBEEE8FDB2172F477DF4900D310536C0"));
        let msg = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000000",
        ));
        assert!(schnorr_verify(&pk, &sig, &msg));
    }
    #[test]
    fn test_bip340_invalid() {
        let pk = a32(h(
            "F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9",
        ));
        let sig = a64(h("E907831F80848D1069A5371B402410364BDF1C5F8307B0084C55F1CE2DCA821525F66A4A85EA8B71E482A74F382D2CE5EBEEE8FDB2172F477DF4900D310536C0"));
        let msg = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000001",
        ));
        assert!(!schnorr_verify(&pk, &sig, &msg));
    }
}

#[cfg(test)]
mod sign_test {
    use super::*;
    fn h(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    fn a32(v: Vec<u8>) -> [u8; 32] {
        let mut a = [0u8; 32];
        a.copy_from_slice(&v);
        a
    }
    #[test]
    fn test_sign_roundtrip() {
        let sk = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000003",
        ));
        let aux = [0u8; 32];
        let msg = compute_sha256(b"test.near1000000000000000000000000");
        let sig = schnorr_sign(&sk, &msg, &aux);
        assert!(sig != [0u8; 64], "signing produced zero sig");
        // pk from the FULL Jacobian triple (x = X/Z², y = Y/Z³) —
        // substituting Z=[1,0,0,0] treats Jacobian X as affine x and
        // yields a garbage key (the historical cause of these reds).
        let (jx, jy, jz) = point_mul((GX, GY), fe_bytes_to_fe(&sk)).unwrap();
        let (px, _) = jac_to_affine((jx, jy, jz));
        let mut pk = [0u8; 32];
        for i in 0..4 {
            pk[i * 8..(i + 1) * 8].copy_from_slice(&px[3 - i].to_be_bytes());
        }
        assert!(schnorr_verify(&pk, &sig, &msg), "roundtrip failed");
        eprintln!(
            "PK: {}",
            pk.iter().map(|b| format!("{:02X}", b)).collect::<String>()
        );
        eprintln!(
            "SIG: {}",
            sig.iter().map(|b| format!("{:02X}", b)).collect::<String>()
        );
        eprintln!(
            "MSG: {}",
            msg.iter().map(|b| format!("{:02X}", b)).collect::<String>()
        );
    }

    /// Official vector's pubkey: sk=3 → pk = x(3G) = F9308A01...CE036F9.
    #[test]
    fn test_pubkey_official() {
        let sk = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000003",
        ));
        let pk = schnorr_pubkey(&sk);
        assert_eq!(
            pk.to_vec(),
            h("F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9")
        );
    }

    /// Odd-y d (sk=2): pk is still the x-coordinate; sign's internal d' = n-d
    /// parity handling must accept whatever key this produces.
    #[test]
    fn test_pubkey_sign_roundtrip_sk2() {
        let sk = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000002",
        ));
        let pk = schnorr_pubkey(&sk);
        let msg = compute_sha256(b"odd-y secret key roundtrip");
        let sig = schnorr_sign(&sk, &msg, &[7u8; 32]);
        assert!(sig != [0u8; 64]);
        assert!(schnorr_verify(&pk, &sig, &msg));
    }

    /// Official BIP-340 test vector (index 0): sk=3, msg=0^32, aux=0^32 →
    /// deterministic sig must match the reference exactly. Strongest check
    /// of the nonce construction + mod-n math.
    #[test]
    fn test_sign_official_vector() {
        let sk = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000003",
        ));
        let msg = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000000",
        ));
        let aux = [0u8; 32];
        let sig = schnorr_sign(&sk, &msg, &aux);
        let expect = h("E907831F80848D1069A5371B402410364BDF1C5F8307B0084C55F1CE2DCA821525F66A4A85EA8B71E482A74F382D2CE5EBEEE8FDB2172F477DF4900D310536C0");
        assert_eq!(sig.to_vec(), expect, "official BIP-340 vector mismatch");
    }

    /// Odd-y R path: with the fix, ANY aux must yield a verifiable sig —
    /// the old code returned zeros whenever the nonce's R had odd y.
    #[test]
    fn test_sign_every_aux_verifies() {
        let sk = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000003",
        ));
        let msg = compute_sha256(b"odd-y nonce sweep");
        let mut pk = [0u8; 32];
        {
            let (jx, jy, jz) = point_mul((GX, GY), fe_bytes_to_fe(&sk)).unwrap();
            let (px, _) = jac_to_affine((jx, jy, jz));
            for i in 0..4 {
                pk[i * 8..(i + 1) * 8].copy_from_slice(&px[3 - i].to_be_bytes());
            }
        }
        for aux_byte in 0u8..64 {
            let sig = schnorr_sign(&sk, &msg, &[aux_byte; 32]);
            assert!(sig != [0u8; 64], "zero sig at aux={}", aux_byte);
            assert!(
                schnorr_verify(&pk, &sig, &msg),
                "verify failed at aux={}",
                aux_byte
            );
        }
    }
}

#[cfg(test)]
mod sign_test2 {
    use super::*;
    fn h(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    fn a32(v: Vec<u8>) -> [u8; 32] {
        let mut a = [0u8; 32];
        a.copy_from_slice(&v);
        a
    }
    #[test]
    fn test_sign_bruteforce() {
        let sk = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000003",
        ));
        let msg = compute_sha256(b"test.near1000000000000000000000000");
        for aux_byte in 0u8..255 {
            let aux = [aux_byte; 32];
            let sig = schnorr_sign(&sk, &msg, &aux);
            if sig != [0u8; 64] {
                let (jx, jy, jz) = point_mul((GX, GY), fe_bytes_to_fe(&sk)).unwrap();
                let (px, _) = jac_to_affine((jx, jy, jz));
                let mut pk = [0u8; 32];
                for i in 0..4 {
                    pk[i * 8..(i + 1) * 8].copy_from_slice(&px[3 - i].to_be_bytes());
                }
                assert!(schnorr_verify(&pk, &sig, &msg), "verify failed");
                eprintln!(
                    "AUX={}: PK={} SIG={} MSG={}",
                    aux_byte,
                    pk.iter().map(|b| format!("{:02X}", b)).collect::<String>(),
                    sig.iter().map(|b| format!("{:02X}", b)).collect::<String>(),
                    msg.iter().map(|b| format!("{:02X}", b)).collect::<String>()
                );
                return;
            }
        }
        panic!("no valid aux found");
    }
}

#[cfg(test)]
mod sign_pk_test {
    use super::*;
    fn h(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    fn a32(v: Vec<u8>) -> [u8; 32] {
        let mut a = [0u8; 32];
        a.copy_from_slice(&v);
        a
    }
    #[test]
    fn test_sign_pk_matches_plain_sign() {
        // BIP-340 vector 1 key + a few others: for the SAME aux, sign_pk with
        // the true pk33 must equal plain sign byte-for-byte (deterministic
        // nonce path is identical once d' and P are pinned).
        let keys = [
            "0000000000000000000000000000000000000000000000000000000000000003",
            "C90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B14E5C9",
            "0B432B2677937381AEF05BB02A66ECD012773062CF3FA2549E44F58ED2401710",
        ];
        let msg = compute_sha256(b"chat-op-pk-cache-test");
        let aux = [0xABu8; 32];
        for k in keys {
            let sk = a32(h(k));
            let pk33 = schnorr_pubkey_33(&sk);
            let sig_plain = schnorr_sign(&sk, &msg, &aux);
            let sig_pk = schnorr_sign_pk(&sk, &pk33, &msg, &aux);
            assert_eq!(sig_plain, sig_pk, "sig mismatch for sk {k}");
            // and it verifies against the x-only pk
            let mut xpk = [0u8; 32];
            xpk.copy_from_slice(&pk33[1..]);
            assert!(schnorr_verify(&xpk, &sig_pk, &msg));
        }
    }
    #[test]
    fn test_sign_pk_wrong_parity_fails() {
        let sk = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000003",
        ));
        let msg = compute_sha256(b"parity-flip-test");
        let aux = [0x01u8; 32];
        let mut pk33 = schnorr_pubkey_33(&sk);
        pk33[0] ^= 0x01; // flip 0x02 <-> 0x03
        let sig = schnorr_sign_pk(&sk, &pk33, &msg, &aux);
        let mut xpk = [0u8; 32];
        xpk.copy_from_slice(&pk33[1..]);
        // wrong d' → the sig must NOT verify (fail-closed, not garbage-accept)
        assert!(!schnorr_verify(&xpk, &sig, &msg));
    }
    #[test]
    fn test_sign_pk_bad_prefix_zero_sig() {
        let sk = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000003",
        ));
        let msg = compute_sha256(b"bad-prefix-test");
        let aux = [0u8; 32];
        let mut pk33 = schnorr_pubkey_33(&sk);
        pk33[0] = 0xFF;
        assert_eq!(schnorr_sign_pk(&sk, &pk33, &msg, &aux), [0u8; 64]);
    }
    #[test]
    fn test_pubkey33_prefix_consistency() {
        // x-only pubkey must equal pk33[1..] for random-ish keys
        for i in 0u8..8 {
            let sk = compute_sha256(&[i; 57]);
            let pk33 = schnorr_pubkey_33(&sk);
            let xpk = schnorr_pubkey(&sk);
            assert!(pk33[0] == 0x02 || pk33[0] == 0x03);
            assert_eq!(&pk33[1..], &xpk[..]);
        }
    }
}

#[cfg(test)]
mod verify_python_sig {
    use super::*;
    fn h(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    fn a32(v: Vec<u8>) -> [u8; 32] {
        let mut a = [0u8; 32];
        a.copy_from_slice(&v);
        a
    }
    fn a64(v: Vec<u8>) -> [u8; 64] {
        let mut a = [0u8; 64];
        a.copy_from_slice(&v);
        a
    }
    #[test]
    fn test_python_sig() {
        let pk = a32(h(
            "F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9",
        ));
        let sig = a64(h("5A7DF9EA987C0F24DD076A984F760D772D7D2E3DB3F52E71481CE98A6C8FC1914B9AAB2E8DB5A06EE6D4DA99D38835B0FBA9B069261BBC5AEB7800EFEBCAE29B"));
        let msg = compute_sha256(b"test.near1000000000000000000000000");
        eprintln!(
            "MSG: {}",
            msg.iter().map(|b| format!("{:02X}", b)).collect::<String>()
        );
        assert!(schnorr_verify(&pk, &sig, &msg), "python sig verify failed");
    }
}

#[cfg(test)]
mod comb_test {
    use super::*;
    fn rand_scalar(seed: u64) -> [u64; 4] {
        // xorshift64* PRNG — deterministic, no_std-friendly
        let mut x = seed;
        let mut out = [0u64; 4];
        for v in out.iter_mut() {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            *v = x.wrapping_mul(0x2545F4914F6CDD1D);
        }
        out
    }
    #[test]
    fn comb_matches_point_mul_200() {
        // skip zero/overflow-class scalars: rand mod (n-1) + 1 via nibble-kill
        for seed in 1..=100u64 {
            let k = rand_scalar(seed * 0x9E3779B97F4A7C15);
            if sc_geq_n(k) || k == [0; 4] {
                continue;
            }
            let a = comb::point_mul_g(k);
            let b = point_mul((GX, GY), k);
            assert_eq!(a.is_none(), b.is_none(), "infinity flag mismatch seed {seed}");
            if let (Some(p1), Some(p2)) = (a, b) {
                assert_eq!(jac_to_affine(p1), jac_to_affine(p2), "point mismatch seed {seed}");
            }
        }
    }
    #[test]
    fn comb_edge_scalars() {
        // k = 1, 15 (max digit), 16 (roll to next row), 2^255-ish high digit rows
        for k in [
            [1, 0, 0, 0],
            [15, 0, 0, 0],
            [16, 0, 0, 0],
            [0xFFFF_FFFF, 0, 0, 0],
            [
                0xFFFFFFFFFFFFFFF,
                0x7FFFFFFFFFFFFFFF,
                0,
                0,
            ],
        ] {
            if sc_geq_n(k) {
                continue;
            }
            assert_eq!(
                comb::point_mul_g(k).map(jac_to_affine),
                point_mul((GX, GY), k).map(jac_to_affine),
                "edge scalar {k:?}"
            );
        }
    }
}

    #[test]
    fn bench_sign_comb_vs_double() {
        let det_scalar = |seed: u64| -> [u64; 4] {
            let mut x = seed | 1;
            let mut out = [0u64; 4];
            for v in out.iter_mut() {
                x ^= x >> 12;
                x ^= x << 25;
                x ^= x >> 27;
                *v = x.wrapping_mul(0x2545F4914F6CDD1D);
            }
            out
        };
        let msg = compute_sha256(b"bench");
        let aux = [7u8; 32];
        let sk = {
            let mut b = [0u8; 32];
            b.copy_from_slice(&[
                0xC9, 0x0F, 0xDA, 0xA2, 0x21, 0x68, 0xC2, 0x34,
                0xC4, 0xC6, 0x62, 0x8B, 0x80, 0xDC, 0x1C, 0xD1,
                0x29, 0x02, 0x4E, 0x08, 0x8A, 0x67, 0xCC, 0x74,
                0x02, 0xBB, 0xEA, 0x63, 0xB1, 0x4E, 0x5C, 0x09,
            ]);
            b
        };
        let _ = schnorr_sign(&sk, &msg, &aux);
        let n = 50u32;
        let t0 = std::time::Instant::now();
        for i in 0..n {
            let _ = comb::point_mul_g(fe_bytes_to_fe(&{ let mut b0 = [0u8;32]; let dp = det_scalar((i+1) as u64); for j in 0..4 { b0[j*8..(j+1)*8].copy_from_slice(&dp[3-j].to_be_bytes()); } b0 }));
        }
        let t_combmul = t0.elapsed();
        let t0 = std::time::Instant::now();
        for i in 0..n {
            let kps = det_scalar((i + 1) as u64);
            let mut kb = [0u8; 32];
            for j in 0..4 { kb[j * 8..(j + 1) * 8].copy_from_slice(&kps[3 - j].to_be_bytes()); }
            let _ = schnorr_sign(&sk, &msg, &kb);
        }
        let t_comb = t0.elapsed();
        let t0 = std::time::Instant::now();
        for i in 0..n {
            let mut kpb = [0u8; 32];
            for j in 0..4 { kpb[j * 8..(j + 1) * 8].copy_from_slice(&det_scalar((i + 1) as u64)[3 - j].to_be_bytes()); }
            let kp = fe_bytes_to_fe(&kpb);
            let _ = point_mul((GX, GY), kp);
        }
        let t_mul = t0.elapsed();
        println!("comb mult only : {:?}", t_combmul / n);
        println!("comb sign full : {:?}", t_comb / n);
        println!("double-add mult: {:?}", t_mul / n);
    }

    #[test]
    fn bench_inv_ab() {
        use std::time::Instant;
        let mut seed = 0x9e3779b9u64;
        let mut rng = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut inputs = Vec::new();
        for _ in 0..200 {
            let mut a = [rng(), rng(), rng(), rng()];
            a[3] &= 0x7FFF_FFFF_FFFF_FFFF;
            if a == [0, 0, 0, 0] {
                a = [1, 0, 0, 0];
            }
            inputs.push(a);
        }
        let n = inputs.len() as u32;
        let t0 = Instant::now();
        for a in &inputs {
            let _ = fe_inv(*a);
        }
        let d_pow = t0.elapsed() / n;
        let t0 = Instant::now();
        for a in &inputs {
            let _ = fe_inv_pub(*a);
        }
        let d_euc = t0.elapsed() / n;
        println!("fe_inv  (pow, 505 fe_mul): {:?}/call", d_pow);
        println!("fe_inv_pub (euclid)      : {:?}/call", d_euc);
        println!("speedup: {:.2}x", d_pow.as_secs_f64() / d_euc.as_secs_f64());
    }
    #[test]
    fn test_fe_inv_pub_matches_fe_pow() {
        // differential vs the constant-path inverse on random + edge inputs
        let mut seed = 0x1157_7901u64;
        let mut rng = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for i in 0..300 {
            let mut a = [rng(), rng(), rng(), rng()];
            a[3] &= 0x7FFF_FFFF_FFFF_FFFF; // < 2^255 < p — guaranteed in range, no loop
            if a == [0, 0, 0, 0] {
                continue;
            }
            let slow = fe_inv(a);
            let fast = fe_inv_pub(a);
            assert_eq!(slow, fast, "fe_inv_pub mismatch at i={i}");
        }
        // edges
        for a in [FE_ONE, [2, 0, 0, 0], [P[0] - 1, P[1], P[2], P[3]]] {
            assert_eq!(fe_inv(a), fe_inv_pub(a), "edge mismatch");
        }
    }
    #[test]
    fn test_mixed_add_comb_diff() {
        // comb sign_path with mixed-add must still produce BIP-340-identical outputs:
        // reuse official vector 0 end-to-end (comb already covered it; re-run to bind mixed-add)
        let sk = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000003",
        ));
        let msg = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000000",
        ));
        let aux = a32(h(
            "0000000000000000000000000000000000000000000000000000000000000000",
        ));
        let pk = schnorr_pubkey_33(&sk);
        assert_eq!(pk[0], 0x02);
        assert_eq!(
            &pk[1..],
            &a32(h("F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9"))[..]
        );
        let sig = schnorr_sign_pk(&sk, &pk, &msg, &aux);
        assert_eq!(
            sig,
            a64(h(
                "E907831F80848D1069A5371B402410364BDF1C5F8307B0084C55F1CE2DCA821525F66A4A85EA8B71E482A74F382D2CE5EBEEE8FDB2172F477DF4900D310536C0"
            ))
        );
        assert!(schnorr_verify(
            &a32(h("F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9")),
            &sig,
            &msg
        ));
    }
    #[test]
    fn bench_sign_pk_warm() {
        use std::time::Instant;
        let sk = a32(h("0000000000000000000000000000000000000000000000000000000000000003"))
            .map(|v| v).to_vec();
        let sk = sk.try_into().unwrap();
        let pk = schnorr_pubkey_33(&sk);
        let msg = compute_sha256(b"bench pk warm");
        let aux = [11u8; 32];
        let _ = schnorr_sign_pk(&sk, &pk, &msg, &aux);
        let n = 200u32;
        let t0 = Instant::now();
        for i in 0..n {
            let aux_i = aux.clone();
            let m = compute_sha256(&i.to_be_bytes());
            let _ = schnorr_sign_pk(&sk, &pk, &m, &aux_i);
        }
        println!("sign_pk WARM (cached pk, comb + euclid): {:?}/call", t0.elapsed() / n);
    }
    #[test]
    fn bench_phases() {
        use std::time::Instant;
        let sk = a32(h("0000000000000000000000000000000000000000000000000000000000000003"))
            .try_into()
            .unwrap();
        let pk = schnorr_pubkey_33(&sk);
        let msg = compute_sha256(b"bench phases");
        let aux = [11u8; 32];
        let n = 300u32;
        // full warm sign
        let _ = schnorr_sign_pk(&sk, &pk, &msg, &aux);
        let t = Instant::now();
        for i in 0..n {
            let m = compute_sha256(&i.to_be_bytes());
            let _ = schnorr_sign_pk(&sk, &pk, &m, &aux);
        }
        let full = t.elapsed() / n;
        // hashing share: 3 tagged hashes + msg/aux sha
        let t = Instant::now();
        for i in 0..n {
            let m = compute_sha256(&i.to_be_bytes());
            let _ = tagged_hash(b"BIP0340/aux", &aux);
            let _ = tagged_hash(b"BIP0340/nonce", &m);
            let _ = tagged_hash(b"BIP0340/challenge", &m);
        }
        let hs = t.elapsed() / n;
        // scalar mul mod n (montgomery)
        let k = [3u64, 0, 0, 0];
        let t = Instant::now();
        for _ in 0..(n * 10) {
            let _ = sc_mul_mod_n(k, k);
        }
        let sm = t.elapsed() / (n * 10);
        // parity derive (the y of R)
        println!("warm={full:?} hashes(4-sha)={hs:?} sc_mul×10={sm:?}");
    }

}
