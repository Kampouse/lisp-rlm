#![cfg_attr(target_arch = "wasm32", no_std)]

#[cfg(target_arch = "wasm32")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

const P: [u64; 4] = [0xFFFFFFFEFFFFFC2F, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF];
const N: [u64; 4] = [0xBFD25E8CD0364141, 0xBAAEDCE6AF48A03B, 0xFFFFFFFFFFFFFFFE, 0xFFFFFFFFFFFFFFFF];
const RED: u64 = 0x1000003D1;
const FE_ONE: [u64; 4] = [1, 0, 0, 0];
const GX: [u64; 4] = [0x59F2815B16F81798, 0x029BFCDB2DCE28D9, 0x55A06295CE870B07, 0x79BE667EF9DCBBAC];
const GY: [u64; 4] = [0x9C47D08FFB10D4B8, 0xFD17B448A6855419, 0x5DA4FBFC0E1108A8, 0x483ADA7726A3C465];

#[inline(always)] fn ge_p(a: [u64; 4]) -> bool {
    for i in (0..4).rev() { if a[i] > P[i] { return true; } if a[i] < P[i] { return false; } }
    true
}

#[inline(always)] fn fe_reduce(r: &mut [u64; 4]) { if ge_p(*r) { fe_sub_p(r); } if ge_p(*r) { fe_sub_p(r); } }

#[inline(always)] fn fe_sub_p(r: &mut [u64; 4]) {
    let mut b = 0u128;
    for i in 0..4 { b = (r[i] as u128).wrapping_add((P[i] as u128).wrapping_neg()).wrapping_add(b); r[i] = b as u64; b = (b >> 127) & 1; }
}

#[inline(always)] pub fn fe_add(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut r = [0u64; 4]; let mut c = 0u128;
    for i in 0..4 { c = c.wrapping_add(a[i] as u128).wrapping_add(b[i] as u128); r[i] = c as u64; c >>= 64; }
    if c > 0 {
        let mut c2 = RED as u128;
        for i in 0..4 { let t = (r[i] as u128).wrapping_add(c2); r[i] = t as u64; c2 = t >> 64; }
        if c2 > 0 { let mut c3 = RED as u128; for i in 0..4 { let t = (r[i] as u128).wrapping_add(c3); r[i] = t as u64; c3 = t >> 64; } }
    }
    fe_reduce(&mut r); r
}

#[inline(always)] pub fn fe_sub(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut r = [0u64; 4]; let mut b2: i128 = 0;
    for i in 0..4 { b2 += a[i] as i128 - b[i] as i128; r[i] = b2 as u64; b2 >>= 64; }
    if b2 < 0 { let mut c = 0u128; for i in 0..4 { c = c.wrapping_add(P[i] as u128).wrapping_add(r[i] as u128); r[i] = c as u64; c >>= 64; } }
    fe_reduce(&mut r); r
}

#[inline(always)] pub fn fe_mul(a: [u64; 4], b: [u64; 4]) -> [u64; 4] {
    let mut t = [0u64; 8];
    for i in 0..4 { let ai = a[i] as u128; let mut c: u128 = 0; for j in 0..4 { let prod = ai * b[j] as u128; let (lo, ov1) = c.overflowing_add(prod); let (lo, ov2) = lo.overflowing_add(t[i + j] as u128); t[i + j] = lo as u64; c = (lo >> 64) | ((ov1 as u128 | ov2 as u128) << 64); } t[i + 4] = c as u64; }
    let mut r = [0u64; 5]; let mut carry: u128 = 0;
    for i in 0..4 { carry = carry.wrapping_add(t[i] as u128).wrapping_add((t[i + 4] as u128).wrapping_mul(RED as u128)); r[i] = carry as u64; carry >>= 64; }
    r[4] = carry as u64;
    let mut fold = r[4] as u128;
    for _ in 0..3 { if fold == 0 { break; } let mut c: u128 = fold.wrapping_mul(RED as u128); fold = 0; for i in 0..3 { c = c.wrapping_add(r[i] as u128); r[i] = c as u64; c >>= 64; } c = c.wrapping_add(r[3] as u128); r[3] = c as u64; fold = c >> 64; }
    let mut result = [r[0], r[1], r[2], r[3]]; fe_reduce(&mut result); result
}

#[inline(always)] pub fn fe_pow(mut base: [u64; 4], exp: [u64; 4]) -> [u64; 4] {
    let mut result = FE_ONE; for i in 0..256 { if (exp[i / 64] >> (i % 64)) & 1 == 1 { result = fe_mul(result, base); } base = fe_mul(base, base); } result
}

#[inline(always)] pub fn fe_inv(a: [u64; 4]) -> [u64; 4] { fe_pow(a, [0xFFFFFFFEFFFFFC2D, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF]) }

/// Jacobian projective point: (X, Y, Z) represents affine (X/Z^2, Y/Z^3)
/// Point at infinity: Z == 0
fn jac_is_infinity(p: &([u64; 4], [u64; 4], [u64; 4])) -> bool {
    p.2 == [0, 0, 0, 0]
}

/// Jacobian point addition (complete, handles infinity and P == Q)
fn jac_add(p: ([u64; 4], [u64; 4], [u64; 4]), q: ([u64; 4], [u64; 4], [u64; 4])) -> ([u64; 4], [u64; 4], [u64; 4]) {
    let (x1, y1, z1) = p;
    let (x2, y2, z2) = q;
    let inf1 = jac_is_infinity(&p);
    let inf2 = jac_is_infinity(&q);
    if inf1 && inf2 { return ([0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]); }
    if inf1 { return q; }
    if inf2 { return p; }

    let z1sq = fe_mul(z1, z1);
    let z2sq = fe_mul(z2, z2);
    let u1 = fe_mul(x1, z2sq);
    let u2 = fe_mul(x2, z1sq);
    let z1cu = fe_mul(z1sq, z1);
    let z2cu = fe_mul(z2sq, z2);
    let s1 = fe_mul(y1, z2cu);
    let s2 = fe_mul(y2, z1cu);

    let h = fe_sub(u2, u1);
    let r = fe_sub(s2, s1);

    // If H == 0 and R == 0, points are the same -> double
    if h == [0, 0, 0, 0] && r == [0, 0, 0, 0] {
        return jac_double(p);
    }
    // If H == 0 and R != 0, points are inverses -> infinity
    if h == [0, 0, 0, 0] {
        return ([0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]);
    }

    let hsq = fe_mul(h, h);
    let hcu = fe_mul(hsq, h);
    let rsg = fe_mul(r, r);
    let u1hsq = fe_mul(u1, hsq);

    let x3 = fe_sub(fe_sub(rsg, hcu), fe_mul(u1hsq, [2, 0, 0, 0]));
    let y3 = fe_sub(fe_mul(r, fe_sub(u1hsq, x3)), fe_mul(s1, hcu));
    let z3 = fe_mul(fe_mul(z1, z2), h);

    (x3, y3, z3)
}

/// Jacobian point doubling (a=0 for secp256k1)
fn jac_double(p: ([u64; 4], [u64; 4], [u64; 4])) -> ([u64; 4], [u64; 4], [u64; 4]) {
    if jac_is_infinity(&p) { return p; }
    let (x, y, z) = p;
    let ysq = fe_mul(y, y);
    let a = ysq;
    let b = fe_mul(fe_mul(x, a), [4, 0, 0, 0]);
    let c = fe_mul(fe_mul(a, a), [8, 0, 0, 0]);
    let d = fe_mul(fe_mul(x, x), [3, 0, 0, 0]);
    let dsq = fe_mul(d, d);
    let x3 = fe_sub(dsq, fe_mul(b, [2, 0, 0, 0]));
    let y3 = fe_sub(fe_mul(d, fe_sub(b, x3)), c);
    let z3 = fe_mul(fe_mul(y, z), [2, 0, 0, 0]);
    (x3, y3, z3)
}

/// Scalar multiplication returning Jacobian coordinates
/// Returns None only for k == 0 (point at infinity)
fn point_mul(p: ([u64; 4], [u64; 4]), k: [u64; 4]) -> Option<([u64; 4], [u64; 4], [u64; 4])> {
    let mut r: ([u64; 4], [u64; 4], [u64; 4]) = ([0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 0, 0]);
    let mut q: ([u64; 4], [u64; 4], [u64; 4]) = (p.0, p.1, [1, 0, 0, 0]);
    for i in 0..256 {
        if (k[i / 64] >> (i % 64)) & 1 == 1 {
            r = jac_add(r, q);
        }
        q = jac_double(q);
    }
    if jac_is_infinity(&r) { None } else { Some(r) }
}

/// Convert Jacobian point to affine coordinates
fn jac_to_affine(p: ([u64; 4], [u64; 4], [u64; 4])) -> ([u64; 4], [u64; 4]) {
    let (_, _, z) = p;
    let z_inv = fe_inv(z);
    let z2_inv = fe_mul(z_inv, z_inv);
    let z3_inv = fe_mul(z2_inv, z_inv);
    let x = fe_mul(p.0, z2_inv);
    let y = fe_mul(p.1, z3_inv);
    (x, y)
}

fn sc_lt_n(a: [u64; 4]) -> bool { for i in (0..4).rev() { if a[i] > N[i] { return false; } if a[i] < N[i] { return true; } } true }
fn sc_sub_n(a: [u64; 4]) -> [u64; 4] { let mut r = [0u64; 4]; let mut b: i128 = 0; for i in 0..4 { b += N[i] as i128 - a[i] as i128; r[i] = b as u64; b >>= 64; } r }

pub fn fe_bytes_to_fe(b: &[u8]) -> [u64; 4] { let mut r = [0u64; 4]; for i in 0..4 { r[3 - i] = u64::from_be_bytes(b[i * 8..(i + 1) * 8].try_into().unwrap_or([0u8; 8])); } r }

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
const SHA_IV: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
#[inline(always)] fn sha_ch(x: u32, y: u32, z: u32) -> u32 { (x & y) ^ (!x & z) }
#[inline(always)] fn sha_maj(x: u32, y: u32, z: u32) -> u32 { (x & y) ^ (x & z) ^ (y & z) }
#[inline(always)] fn sha_ep0(x: u32) -> u32 { x.rotate_right(2) ^ x.rotate_right(13) ^ x.rotate_right(22) }
#[inline(always)] fn sha_ep1(x: u32) -> u32 { x.rotate_right(6) ^ x.rotate_right(11) ^ x.rotate_right(25) }
#[inline(always)] fn sha_sig0(x: u32) -> u32 { x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3) }
#[inline(always)] fn sha_sig1(x: u32) -> u32 { x.rotate_right(17) ^ x.rotate_right(19) ^ (x >> 10) }

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let len = data.len(); let blocks = if len < 56 { 1 } else if len < 120 { 2 } else { 3 };
    let mut msg = [0u8; 192]; msg[..len].copy_from_slice(data); msg[len] = 0x80;
    let bits = (len as u64) * 8; msg[blocks * 64 - 8..blocks * 64].copy_from_slice(&bits.to_be_bytes());
    let mut h = SHA_IV;
    for b in 0..blocks {
        let off = b * 64; let mut w = [0u32; 64];
        for i in 0..16 { w[i] = u32::from_be_bytes([msg[off + i * 4], msg[off + i * 4 + 1], msg[off + i * 4 + 2], msg[off + i * 4 + 3]]); }
        for i in 16..64 { w[i] = sha_sig1(w[i - 2]).wrapping_add(w[i - 7]).wrapping_add(sha_sig0(w[i - 15])).wrapping_add(w[i - 16]); }
        let [mut a, mut b2, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 { let t1 = hh.wrapping_add(sha_ep1(e)).wrapping_add(sha_ch(e, f, g)).wrapping_add(SHA_K[i]).wrapping_add(w[i]); let t2 = sha_ep0(a).wrapping_add(sha_maj(a, b2, c)); hh = g; g = f; f = e; e = d.wrapping_add(t1); d = c; c = b2; b2 = a; a = t1.wrapping_add(t2); }
        h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b2); h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e); h[5] = h[5].wrapping_add(f); h[6] = h[6].wrapping_add(g); h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32]; for i in 0..8 { out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes()); } out
}

fn tagged_hash(tag: &[u8], msg: &[u8]) -> [u8; 32] {
    let th = sha256(tag); let mut buf = [0u8; 192];
    buf[..32].copy_from_slice(&th); buf[32..64].copy_from_slice(&th); buf[64..64 + msg.len()].copy_from_slice(msg);
    sha256(&buf[..64 + msg.len()])
}

pub fn schnorr_verify(pk_bytes: &[u8; 32], sig_bytes: &[u8; 64], msg: &[u8; 32]) -> bool {
    let pk_x = fe_bytes_to_fe(pk_bytes); let r = fe_bytes_to_fe(&sig_bytes[..32]); let s = fe_bytes_to_fe(&sig_bytes[32..64]);
    if ge_p(pk_x) { return false; }
    let x3 = fe_mul(fe_mul(pk_x, pk_x), pk_x); let y_sq = fe_add(x3, [7, 0, 0, 0]);
    let y = fe_pow(y_sq, [0xFFFFFFFFBFFFFF0C, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF, 0x3FFFFFFFFFFFFFFF]);
    if fe_mul(y, y) != y_sq { return false; }
    let py = if (y[0] & 1) != 0 { fe_sub(P, y) } else { y };
    if ge_p(r) || !sc_lt_n(s) { return false; }
    let mut cd = [0u8; 96]; cd[..32].copy_from_slice(&sig_bytes[..32]); cd[32..64].copy_from_slice(pk_bytes); cd[64..96].copy_from_slice(msg);
    let e_hash = tagged_hash(b"BIP0340/challenge", &cd); let e_fe = fe_bytes_to_fe(&e_hash);

    // Jacobian scalar multiplications (no inversions in inner loop)
    let sg = point_mul((GX, GY), s);
    let ne = sc_sub_n(e_fe);
    let neg_eP = point_mul((pk_x, py), ne);

    // Add the two Jacobian points and convert to affine
    let R = match (sg, neg_eP) {
        (None, None) => return false,
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (Some(a), Some(b)) => jac_add(a, b),
    };
    if jac_is_infinity(&R) { return false; }
    let (rx, ry) = jac_to_affine(R);
    rx == r && (ry[0] & 1) == 0
}

#[no_mangle] pub extern "C" fn schnorr_verify_bip340(pk_ptr: u32, sig_ptr: u32, msg_ptr: u32, msg_len: u32) -> u32 {
    unsafe {
        let pk = core::slice::from_raw_parts(pk_ptr as *const u8, 32);
        let sig = core::slice::from_raw_parts(sig_ptr as *const u8, 64);
        let msg = core::slice::from_raw_parts(msg_ptr as *const u8, msg_len as usize);
        let Ok(pk): Result<[u8; 32], _> = pk.try_into() else { return 0 };
        let Ok(sig): Result<[u8; 64], _> = sig.try_into() else { return 0 };
        let Ok(msg): Result<[u8; 32], _> = msg.try_into() else { return 0 };
        if schnorr_verify(&pk, &sig, &msg) { 1 } else { 0 }
    }
}

const TEST_PK: [u8; 32] = [0xF9,0x30,0x8A,0x01,0x92,0x58,0xC3,0x10,0x49,0x34,0x4F,0x85,0xF8,0x9D,0x52,0x29,0xB5,0x31,0xC8,0x45,0x83,0x6F,0x99,0xB0,0x86,0x01,0xF1,0x13,0xBC,0xE0,0x36,0xF9];
const TEST_SIG: [u8; 64] = [0xE9,0x07,0x83,0x1F,0x80,0x84,0x8D,0x10,0x69,0xA5,0x37,0x1B,0x40,0x24,0x10,0x36,0x4B,0xDF,0x1C,0x5F,0x83,0x07,0xB0,0x08,0x4C,0x55,0xF1,0xCE,0x2D,0xCA,0x82,0x15,0x25,0xF6,0x6A,0x4A,0x85,0xEA,0x8B,0x71,0xE4,0x82,0xA7,0x4F,0x38,0x2D,0x2C,0xE5,0xEB,0xEE,0xE8,0xFD,0xB2,0x17,0x2F,0x47,0x7D,0xF4,0x90,0x0D,0x31,0x05,0x36,0xC0];
const TEST_MSG: [u8; 32] = [0u8; 32];

#[no_mangle] pub extern "C" fn run() -> u32 {
    unsafe { schnorr_verify_bip340(TEST_PK.as_ptr() as u32, TEST_SIG.as_ptr() as u32, TEST_MSG.as_ptr() as u32, 32) }
}

#[cfg(test)] mod tests {
    use super::*;
    fn h(s: &str) -> Vec<u8> { (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i+2], 16).unwrap()).collect() }
    fn a32(v: Vec<u8>) -> [u8; 32] { let mut a = [0u8; 32]; a.copy_from_slice(&v); a }
    fn a64(v: Vec<u8>) -> [u8; 64] { let mut a = [0u8; 64]; a.copy_from_slice(&v); a }
    #[test] fn test_sha256() { assert_eq!(sha256(b""), [0xe3,0xb0,0xc4,0x42,0x98,0xfc,0x1c,0x14,0x9a,0xfb,0xf4,0xc8,0x99,0x6f,0xb9,0x24,0x27,0xae,0x41,0xe4,0x64,0x9b,0x93,0x4c,0xa4,0x95,0x99,0x1b,0x78,0x52,0xb8,0x55]); }
    #[test] fn test_fe_mul() { let two = [2,0,0,0]; assert_eq!(fe_mul(two, fe_inv(two)), FE_ONE); }
    #[test] fn test_2g() { let (x, y, z) = point_mul((GX, GY), [2,0,0,0]).unwrap(); let ax = fe_mul(x, fe_mul(fe_inv(z), fe_inv(z))); assert_eq!(ax, fe_bytes_to_fe(&h("C6047F9441ED7D6D3045406E95C07CD85C778E4B8CEF3CA7ABAC09B95C709EE5"))); }
    #[test] fn test_bip340_valid() { let pk = a32(h("F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9")); let sig = a64(h("E907831F80848D1069A5371B402410364BDF1C5F8307B0084C55F1CE2DCA821525F66A4A85EA8B71E482A74F382D2CE5EBEEE8FDB2172F477DF4900D310536C0")); let msg = a32(h("0000000000000000000000000000000000000000000000000000000000000000")); assert!(schnorr_verify(&pk, &sig, &msg)); }
    #[test] fn test_bip340_invalid() { let pk = a32(h("F9308A019258C31049344F85F89D5229B531C845836F99B08601F113BCE036F9")); let sig = a64(h("E907831F80848D1069A5371B402410364BDF1C5F8307B0084C55F1CE2DCA821525F66A4A85EA8B71E482A74F382D2CE5EBEEE8FDB2172F477DF4900D310536C0")); let msg = a32(h("0000000000000000000000000000000000000000000000000000000000000001")); assert!(!schnorr_verify(&pk, &sig, &msg)); }

    #[test]
    fn test_trace_mul() {
        let a = GX;
        let mut t = [0u64; 8];
        for i in 0..4 { let ai = a[i] as u128; let mut c: u128 = 0; for j in 0..4 { let prod = ai * (a[j] as u128); c = c.wrapping_add(prod).wrapping_add(t[i + j] as u128); t[i + j] = c as u64; c >>= 64; } t[i + 4] = c as u64; }
    }

}