//! SHA-256 and BIP-340 Schnorr verification builtins for lisp-rlm.
//! Adapted from schnorr-wasm/src/lib.rs (no_std, zero-allocation).

use crate::types::LispVal;

const SHA256_K: [u32; 64] = [
    0x428A2F98,0x71374491,0xB5C0FBCF,0xE9B5DBA5,0x3956C25B,0x59F111F1,0x923F82A4,0xAB1C5ED5,
    0xD807AA98,0x12835B01,0x243185BE,0x550C7DC3,0x72BE5D74,0x80DEB1FE,0x9BDC06A7,0xC19BF174,
    0xE49B69C1,0xEFBE4786,0x0FC19DC6,0x240CA1CC,0x2DE92C6F,0x4A7484AA,0x5CB0A9DC,0x76F988DA,
    0x983E5152,0xA831C66D,0xB00327C8,0xBF597FC7,0xC6E00BF3,0xD5A79147,0x06CA6351,0x14292967,
    0x27B70A85,0x2E1B2138,0x4D2C6DFC,0x53380D13,0x650A7354,0x766A0ABB,0x81C2C92E,0x92722C85,
    0xA2BFE8A1,0xA81A664B,0xC24B8B70,0xC76C51A3,0xD192E819,0xD6990624,0xF40E3585,0x106AA070,
    0x19A4C116,0x1E376C08,0x2748774C,0x34B0BCB5,0x391C0CB3,0x4ED8AA4A,0x5B9CCA4F,0x682E6FF3,
    0x748F82EE,0x78A5636F,0x84C87814,0x8CC70208,0x90BEFFFA,0xA4506CEB,0xBEF9A3F7,0xC67178F2,
];

fn sha256_block_impl(h: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 { w[i] = u32::from_be_bytes([block[i*4],block[i*4+1],block[i*4+2],block[i*4+3]]); }
    for i in 16..64 {
        let s0 = w[i-15].rotate_right(7) ^ w[i-15].rotate_right(18) ^ (w[i-15] >> 3);
        let s1 = w[i-2].rotate_right(17) ^ w[i-2].rotate_right(19) ^ (w[i-2] >> 10);
        w[i] = w[i-16].wrapping_add(s0).wrapping_add(w[i-7]).wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        hh = g; g = f; f = e; e = d.wrapping_add(t1); d = c; c = b; b = a; a = t1.wrapping_add(t2);
    }
    h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e); h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g); h[7] = h[7].wrapping_add(hh);
}

fn sha256_impl(data: &[u8]) -> [u8; 32] {
    let mut h = [0x6A09E667u32,0xBB67AE85,0x3C6EF372,0xA54FF53A,0x510E527F,0x9B05688C,0x1F83D9AB,0x5BE0CD19];
    let len = data.len();
    let mut buf = [0u8; 64];
    let mut off = 0;
    while off + 64 <= len { sha256_block_impl(&mut h, data[off..off+64].try_into().unwrap()); off += 64; }
    let rem = len - off; buf[..rem].copy_from_slice(&data[off..]); buf[rem] = 0x80;
    let bit_len = (len as u64) * 8;
    if rem >= 56 { sha256_block_impl(&mut h, &buf); buf = [0; 64]; }
    buf[56..64].copy_from_slice(&bit_len.to_be_bytes());
    sha256_block_impl(&mut h, &buf);
    let mut out = [0u8; 32];
    for i in 0..8 { out[i*4..i*4+4].copy_from_slice(&h[i].to_be_bytes()); }
    out
}

fn tagged_hash_impl(tag: &[u8], msg: &[u8]) -> [u8; 32] {
    let tag_hash = sha256_impl(tag);
    let total = 64 + msg.len();
    let mut buf = vec![0u8; total];
    buf[..32].copy_from_slice(&tag_hash); buf[32..64].copy_from_slice(&tag_hash); buf[64..].copy_from_slice(msg);
    sha256_impl(&buf)
}

// --- Field arithmetic (secp256k1, 4-limb u64, LE) ---

const P: [u64; 4] = [0xFFFFFFFEFFFFFC2F, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF];
const DELTA: u64 = 0x1000003D1;
const N: [u64; 4] = [0xBFD25E8CD0364141, 0xBAAEDCE6AF48A03B, 0xFFFFFFFFFFFFFFFE, 0xFFFFFFFFFFFFFFFF];
const GX: [u64; 4] = [0x59F2815B16F81798, 0x029BFCDB2DCE28D9, 0x55A06295CE870B07, 0x79BE667EF9DCBBAC];
const GY: [u64; 4] = [0x9C47D08FFB10D4B8, 0xFD17B448A6855419, 0x5DA4FBFC0E1108A8, 0x483ADA7726A3C465];

fn is_zero(a: &[u64; 4]) -> bool { a[0]==0 && a[1]==0 && a[2]==0 && a[3]==0 }

fn add256(a: &[u64; 4], b: &[u64; 4]) -> ([u64; 4], u64) {
    let mut r = [0u64; 4]; let mut c = 0u64;
    for i in 0..4 {
        let (s1, c1) = a[i].overflowing_add(b[i]);
        let (s2, c2) = s1.overflowing_add(c);
        r[i] = s2; c = (c1 as u64) + (c2 as u64);
    }
    (r, c)
}

fn sub256(a: &[u64; 4], b: &[u64; 4]) -> ([u64; 4], u64) {
    let mut r = [0u64; 4]; let mut borrow = 0u64;
    for i in 0..4 {
        let ai = a[i]; let bi = b[i].wrapping_add(borrow);
        borrow = if bi < borrow { 1 } else if ai < bi { 1 } else { 0 };
        r[i] = ai.wrapping_sub(bi);
    }
    (r, borrow)
}

fn cond_sub_p(a: &[u64; 4]) -> [u64; 4] { let (r, b) = sub256(a, &P); if b == 0 { r } else { *a } }

fn fe_add(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
    let (sum, c) = add256(a, b);
    if c > 0 { let (s, _) = add256(&sum, &[DELTA, 0, 0, 0]); cond_sub_p(&s) } else { cond_sub_p(&sum) }
}

fn fe_sub(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
    let (d, b) = sub256(a, b);
    if b > 0 { let (s, b2) = sub256(&d, &[DELTA, 0, 0, 0]); let s = if b2 > 0 { add256(&s, &P).0 } else { s }; cond_sub_p(&s) } else { d }
}

fn fe_mul(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
    let mask: u128 = (1u128 << 64) - 1;
    let mut r = [0u128; 8];
    for i in 0..4 { let mut carry = 0u128; for j in 0..4 { let prod = (a[i] as u128) * (b[j] as u128); let (s, o1) = r[i+j].overflowing_add(prod); let (s2, o2) = s.overflowing_add(carry); r[i+j] = s2 & mask; carry = ((o1 as u128 + o2 as u128) << 64) + (s2 >> 64); } r[i+4] += carry; }
    for k in 0..7 { r[k+1] += r[k] >> 64; r[k] &= mask; }
    let mut low = [r[0] as u64, r[1] as u64, r[2] as u64, r[3] as u64];
    let mut carry: u128 = 0;
    for i in 0..4 { let prod = (r[i+4] as u128) * (DELTA as u128) + (low[i] as u128) + carry; low[i] = prod as u64; carry = prod >> 64; }
    let cd = carry * (DELTA as u128);
    let lo_cd = cd as u64; let hi_cd = (cd >> 64) as u64;
    let (s, c) = add256(&low, &[lo_cd, hi_cd, 0, 0]);
    if c > 0 { let (s2, _) = add256(&s, &[DELTA, 0, 0, 0]); cond_sub_p(&s2) } else { cond_sub_p(&s) }
}

fn fe_sqr(a: &[u64; 4]) -> [u64; 4] { fe_mul(a, a) }

fn fe_inv(a: &[u64; 4]) -> [u64; 4] {
    let mut r: [u64; 4] = [1, 0, 0, 0];
    let exp: [u64; 4] = [0xFFFFFFFEFFFFFC2D, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF];
    for w in 0..4 { let mut bits = exp[3-w]; for _ in 0..64 { r = fe_sqr(&r); if bits >> 63 != 0 { r = fe_mul(&r, a); } bits <<= 1; } }
    r
}

fn fe_sqrt(a: &[u64; 4]) -> [u64; 4] {
    let exp: [u64; 4] = [0xFFFFFFFFBFFFFF0C, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF, 0x3FFFFFFFFFFFFFFF];
    let mut r: [u64; 4] = [1, 0, 0, 0];
    for w in 0..4 { let mut bits = exp[3-w]; for _ in 0..64 { r = fe_sqr(&r); if bits >> 63 != 0 { r = fe_mul(&r, a); } bits <<= 1; } }
    r
}

fn fe_mod_n(a: &[u64; 4]) -> [u64; 4] { let (r, b) = sub256(a, &N); if b == 0 { r } else { *a } }

fn fe_to_be_bytes(a: &[u64; 4]) -> [u8; 32] {
    let b0 = a[0].to_le_bytes(); let b1 = a[1].to_le_bytes(); let b2 = a[2].to_le_bytes(); let b3 = a[3].to_le_bytes();
    let le: [u8; 32] = [b0[0],b0[1],b0[2],b0[3],b0[4],b0[5],b0[6],b0[7],
        b1[0],b1[1],b1[2],b1[3],b1[4],b1[5],b1[6],b1[7],
        b2[0],b2[1],b2[2],b2[3],b2[4],b2[5],b2[6],b2[7],
        b3[0],b3[1],b3[2],b3[3],b3[4],b3[5],b3[6],b3[7]];
    let mut be = [0u8; 32]; for i in 0..32 { be[i] = le[31-i]; } be
}

fn be_bytes_to_fe(b: &[u8; 32]) -> [u64; 4] {
    let mut rb = [0u8; 32]; for i in 0..32 { rb[i] = b[31-i]; }
    [u64::from_le_bytes([rb[0],rb[1],rb[2],rb[3],rb[4],rb[5],rb[6],rb[7]]),
     u64::from_le_bytes([rb[8],rb[9],rb[10],rb[11],rb[12],rb[13],rb[14],rb[15]]),
     u64::from_le_bytes([rb[16],rb[17],rb[18],rb[19],rb[20],rb[21],rb[22],rb[23]]),
     u64::from_le_bytes([rb[24],rb[25],rb[26],rb[27],rb[28],rb[29],rb[30],rb[31]])]
}

struct Pt { x: [u64; 4], y: [u64; 4], inf: bool }

fn point_double(p: &Pt) -> Pt {
    if p.inf { return Pt { x:[0;4], y:[0;4], inf:true }; }
    let two_y = fe_add(&p.y, &p.y);
    if is_zero(&two_y) { return Pt { x:[0;4], y:[0;4], inf:true }; }
    let three = [3u64,0,0,0];
    let x2 = fe_sqr(&p.x);
    let lambda = fe_mul(&fe_mul(&three, &x2), &fe_inv(&two_y));
    let lx = fe_sqr(&lambda);
    let nx = fe_sub(&lx, &fe_add(&p.x, &p.x));
    let ny = fe_sub(&fe_mul(&lambda, &fe_sub(&p.x, &nx)), &p.y);
    Pt { x: nx, y: ny, inf: false }
}

fn point_add(a: &Pt, b: &Pt) -> Pt {
    if a.inf { return Pt { x:b.x, y:b.y, inf:b.inf }; }
    if b.inf { return Pt { x:a.x, y:a.y, inf:a.inf }; }
    if a.x == b.x { if a.y == b.y { return point_double(a); } return Pt { x:[0;4], y:[0;4], inf:true }; }
    let lambda = fe_mul(&fe_sub(&b.y, &a.y), &fe_inv(&fe_sub(&b.x, &a.x)));
    let lx = fe_sqr(&lambda);
    let nx = fe_sub(&fe_sub(&lx, &a.x), &b.x);
    let ny = fe_sub(&fe_mul(&lambda, &fe_sub(&a.x, &nx)), &a.y);
    Pt { x: nx, y: ny, inf: false }
}

fn scalar_mul(k: &[u64; 4], base: &Pt) -> Pt {
    let mut r = Pt { x:[0;4], y:[0;4], inf:true };
    let mut p = Pt { x:base.x, y:base.y, inf:base.inf };
    for w in 0..4 { let mut bits = k[w]; for _ in 0..64 { if bits & 1 != 0 { r = point_add(&r, &p); } p = point_double(&p); bits >>= 1; } }
    r
}

pub fn schnorr_verify_impl(pk: &[u8; 32], sig: &[u8; 64], msg: &[u8]) -> bool {
    let pk_x = be_bytes_to_fe(pk);
    let r_arr: [u8; 32] = sig[..32].try_into().unwrap();
    let s_arr: [u8; 32] = sig[32..].try_into().unwrap();
    let r = be_bytes_to_fe(&r_arr);
    let s = be_bytes_to_fe(&s_arr);
    if is_zero(&fe_mod_n(&r)) || is_zero(&fe_mod_n(&s)) { return false; }

    let y_sq = fe_add(&fe_mul(&fe_mul(&pk_x, &pk_x), &pk_x), &[7,0,0,0]);
    let y = fe_sqrt(&y_sq);
    if fe_sqr(&y) != y_sq { return false; }
    let y_bytes = fe_to_be_bytes(&y);
    let py = if (y_bytes[31] & 1) != 0 { fe_sub(&P, &y) } else { y };
    let p_pt = Pt { x: pk_x, y: py, inf: false };

    let mut buf = [0u8; 256];
    let total = 64 + msg.len();
    if total > 256 { return false; }
    buf[..32].copy_from_slice(&sig[..32]);
    buf[32..64].copy_from_slice(pk);
    buf[64..total].copy_from_slice(msg);
    let e = fe_mod_n(&be_bytes_to_fe(&tagged_hash_impl(b"BIP0340/challenge", &buf[..total])));

    let g = Pt { x: GX, y: GY, inf: false };
    let r_pt = point_add(&scalar_mul(&s, &g), &Pt { x: scalar_mul(&e, &p_pt).x, y: fe_sub(&P, &scalar_mul(&e, &p_pt).y), inf: false });
    if r_pt.inf { return false; }
    if fe_mod_n(&r_pt.x) != fe_mod_n(&r) { return false; }
    let even_y = (fe_to_be_bytes(&r_pt.y)[31] & 1) == 0;
    even_y
}

// --- Public interface for the bytecode VM ---

pub fn lisp_val_to_bytes(v: &LispVal) -> Result<Vec<u8>, String> {
    match v {
        LispVal::List(items) => items.iter().map(|item| {
            match item {
                LispVal::Num(n) if (0..=255).contains(n) => Ok(*n as u8),
                LispVal::U64(n) if (0..=255).contains(n) => Ok(*n as u8),
                _ => Err(format!("byte values must be 0-255, got {}", item)),
            }
        }).collect(),
        LispVal::Str(s) => Ok(s.as_bytes().to_vec()),
        _ => Err(format!("expected list of bytes or string, got {}", v)),
    }
}

pub fn bytes_to_lisp_list(bytes: &[u8]) -> LispVal {
    LispVal::List(bytes.iter().map(|&b| LispVal::Num(b as i64)).collect())
}

pub fn builtin_sha256(args: &[LispVal]) -> Result<LispVal, String> {
    let data = lisp_val_to_bytes(args.get(0).ok_or("sha256: expected 1 argument")?)?;
    Ok(bytes_to_lisp_list(&sha256_impl(&data)))
}

pub fn builtin_tagged_hash(args: &[LispVal]) -> Result<LispVal, String> {
    let tag = lisp_val_to_bytes(args.get(0).ok_or("tagged-hash: expected 2 arguments")?)?;
    let msg = lisp_val_to_bytes(args.get(1).ok_or("tagged-hash: expected 2 arguments")?)?;
    Ok(bytes_to_lisp_list(&tagged_hash_impl(&tag, &msg)))
}

pub fn builtin_schnorr_verify(args: &[LispVal]) -> Result<LispVal, String> {
    let pk = lisp_val_to_bytes(args.get(0).ok_or("schnorr-verify: expected (pk sig msg)")?)?;
    let sig = lisp_val_to_bytes(args.get(1).ok_or("schnorr-verify: expected (pk sig msg)")?)?;
    let msg = lisp_val_to_bytes(args.get(2).ok_or("schnorr-verify: expected (pk sig msg)")?)?;
    if pk.len() != 32 { return Err("schnorr-verify: pk must be 32 bytes".into()); }
    if sig.len() != 64 { return Err("schnorr-verify: sig must be 64 bytes".into()); }
    let pk_arr: [u8; 32] = pk.try_into().map_err(|_| "schnorr-verify: pk must be 32 bytes")?;
    let sig_arr: [u8; 64] = sig.try_into().map_err(|_| "schnorr-verify: sig must be 64 bytes")?;
    Ok(LispVal::Bool(schnorr_verify_impl(&pk_arr, &sig_arr, &msg)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Field arithmetic against Python reference values ===

    #[test]
    fn test_fe_add() {
        let a = [7u64, 0, 0, 0];
        let b = [13u64, 0, 0, 0];
        assert_eq!(fe_add(&a, &b), [20, 0, 0, 0]);
    }

    #[test]
    fn test_fe_sub() {
        let a = [20u64, 0, 0, 0];
        let b = [13u64, 0, 0, 0];
        assert_eq!(fe_sub(&a, &b), [7, 0, 0, 0]);
    }

    #[test]
    fn test_fe_mul_small() {
        let a = [7u64, 0, 0, 0];
        let b = [13u64, 0, 0, 0];
        assert_eq!(fe_mul(&a, &b), [91, 0, 0, 0]);
    }

    #[test]
    fn test_fe_mul_large() {
        // GX^2 computed in Python
        assert_eq!(fe_sqr(&GX), [4095181814372826185, 16385539742293079483, 7757922937955756701, 9606432895266517768]);
    }

    #[test]
    fn test_fe_inv_small() {
        // 7 * 7^-1 = 1
        let inv7 = fe_inv(&[7, 0, 0, 0]);
        assert_eq!(inv7, [15811494916641071437, 7905747460161236406, 13176245766935394011, 15811494920322472813]);
        assert_eq!(fe_mul(&[7, 0, 0, 0], &inv7), [1, 0, 0, 0]);
    }

    #[test]
    fn test_fe_inv_gx() {
        // GX * GX^-1 = 1
        let inv_gx = fe_inv(&GX);
        assert_eq!(inv_gx, [16581409637254471414, 7473978207347869547, 9730782053094875754, 2556634953548008838]);
        assert_eq!(fe_mul(&GX, &inv_gx), [1, 0, 0, 0]);
    }

    #[test]
    fn test_fe_sqrt() {
        // sqrt(GX^2) should be GX
        let sq = fe_sqr(&GX);
        let root = fe_sqrt(&sq);
        assert_eq!(fe_sqr(&root), sq);
    }

    // === SHA-256 ===

    #[test]
    fn test_sha256_empty() {
        assert_eq!(sha256_impl(&[]),
            [227,176,196,66,152,252,28,20,154,251,244,200,153,111,185,36,39,174,65,228,100,155,147,76,164,149,153,27,120,82,184,85]);
    }

    #[test]
    fn test_sha256_abc() {
        assert_eq!(sha256_impl(b"abc").as_slice(),
            [186,120,22,191,143,1,207,234,65,65,64,222,93,174,34,35,176,3,97,163,150,23,122,156,180,16,255,97,242,0,21,173]);
    }

    // === EC point operations ===

    #[test]
    fn test_scalar_mul_1g() {
        let g = Pt { x: GX, y: GY, inf: false };
        let r = scalar_mul(&[1,0,0,0], &g);
        assert_eq!(r.x, GX);
        assert_eq!(r.y, GY);
        assert!(!r.inf);
    }

    #[test]
    fn test_scalar_mul_2g() {
        let g = Pt { x: GX, y: GY, inf: false };
        let r = scalar_mul(&[2,0,0,0], &g);
        assert_eq!(r.x, [12370272968204394213, 6662950628856118439, 3478257130916576472, 14268669794154544493]);
        assert_eq!(r.y, [2550217892273579306, 17867523981857706209, 11800983642684844782, 1936944757666071353]);
    }

    #[test]
    fn test_scalar_mul_3g() {
        let g = Pt { x: GX, y: GY, inf: false };
        let r = scalar_mul(&[3,0,0,0], &g);
        assert_eq!(r.x, [9656264143134537465, 13056436995607206320, 5274928500377997865, 17956003453681058576]);
        assert_eq!(r.y, [7834571707967399538, 7278003473310950171, 1144820191972553558, 4075611493812267028]);
    }

    #[test]
    fn test_scalar_mul_7g() {
        let g = Pt { x: GX, y: GY, inf: false };
        let r = scalar_mul(&[7,0,0,0], &g);
        assert_eq!(r.x, [16801766848214661564, 4413980075321516956, 11788439643834972686, 6682761736226714858]);
        assert_eq!(r.y, [11891796769454056666, 12111253311957362613, 11752017254187422939, 7704473966897092960]);
    }

    #[test]
    fn test_point_double_vs_scalar_mul() {
        let g = Pt { x: GX, y: GY, inf: false };
        let d = point_double(&g);
        let m = scalar_mul(&[2,0,0,0], &g);
        assert_eq!(d.x, m.x);
        assert_eq!(d.y, m.y);
    }

    #[test]
    fn test_2g_on_curve() {
        let g = Pt { x: GX, y: GY, inf: false };
        let r = scalar_mul(&[2,0,0,0], &g);
        // y^2 = x^3 + 7 mod p
        let x3 = fe_mul(&fe_mul(&r.x, &r.x), &r.x);
        let rhs = fe_add(&x3, &[7,0,0,0]);
        assert_eq!(fe_sqr(&r.y), rhs);
    }

    // === BIP-340 Schnorr verification ===
    // Vector 0: msg is 32 zero bytes, NOT empty

    #[test]
    fn test_schnorr_v0_direct() {
        let pk: [u8; 32] = [249,48,138,1,146,88,195,16,73,52,79,133,248,157,82,41,181,49,200,69,131,111,153,176,134,1,241,19,188,224,54,249];
        let sig: [u8; 64] = [233,7,131,31,128,132,141,16,105,165,55,27,64,36,16,54,75,223,28,95,131,7,176,8,76,85,241,206,45,202,130,21,37,246,106,74,133,234,139,113,228,130,167,79,56,45,44,229,235,238,232,253,178,23,47,71,125,244,144,13,49,5,54,192];
        let msg: [u8; 32] = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
        assert!(schnorr_verify_impl(&pk, &sig, &msg));
    }

    #[test]
    fn test_schnorr_v1_direct() {
        let pk: [u8; 32] = [223,241,215,127,42,103,28,95,54,24,55,38,219,35,65,190,88,254,174,29,162,222,206,216,67,36,15,123,80,43,166,89];
        let sig: [u8; 64] = [104,150,189,96,238,174,41,109,180,138,34,159,247,29,254,7,27,222,65,62,109,67,249,23,220,141,207,140,120,222,51,65,137,6,209,26,201,118,171,204,178,11,9,18,146,191,244,234,137,126,252,182,57,234,135,28,250,149,246,222,51,158,75,10];
        let msg: [u8; 32] = [36,63,106,136,133,163,8,211,19,25,138,46,3,112,115,68,164,9,56,34,41,159,49,208,8,46,250,152,236,78,108,137];
        assert!(schnorr_verify_impl(&pk, &sig, &msg));
    }

    #[test]
    fn test_schnorr_v2_direct() {
        let pk: [u8; 32] = [221,48,138,254,197,119,126,19,18,31,167,43,156,193,183,204,1,57,113,83,9,176,134,201,96,225,143,217,105,119,78,184];
        let sig: [u8; 64] = [88,49,170,238,215,180,75,183,78,94,171,148,186,157,66,148,196,155,207,42,96,114,141,139,76,32,15,80,221,49,60,27,171,116,88,121,165,173,149,74,114,196,90,145,195,165,29,60,122,222,169,141,130,248,72,30,14,30,3,103,74,111,63,183];
        let msg: [u8; 32] = [126,45,88,216,179,188,223,26,186,222,199,130,144,84,249,13,218,152,5,170,181,108,119,51,48,36,185,208,165,8,183,92];
        assert!(schnorr_verify_impl(&pk, &sig, &msg));
    }

    #[test]
    fn test_schnorr_bad_sig_direct() {
        let pk: [u8; 32] = [0xF9,0x30,0x8A,0x01,0x92,0x58,0xC3,0x10,0x49,0x34,0x4F,0x85,0xF8,0x9D,0x52,0x29,0xB5,0x31,0xC8,0x45,0x83,0x6F,0x99,0xB0,0x86,0x01,0xF1,0x13,0xBC,0xE0,0x36,0xF9];
        let sig: [u8; 64] = [0u8; 64];
        let msg: [u8; 32] = [0u8; 32];
        assert!(!schnorr_verify_impl(&pk, &sig, &msg));
    }

    // === Negative BIP-340 test vectors (should reject) ===

    #[test]
    fn test_schnorr_v5_false() {
        let pk: [u8; 32] = [238,253,234,76,219,103,119,80,164,32,254,232,7,234,207,33,235,152,152,174,121,185,118,135,102,228,250,160,74,45,74,52];
        let sig: [u8; 64] = [108,255,92,59,168,108,105,234,75,115,118,243,26,155,203,79,116,193,151,96,137,178,217,150,61,162,229,84,62,23,119,105,105,232,155,76,85,100,208,3,73,16,107,132,151,120,93,215,209,215,19,168,174,130,179,47,167,157,95,127,196,7,211,155];
        let msg: [u8; 32] = [36,63,106,136,133,163,8,211,19,25,138,46,3,112,115,68,164,9,56,34,41,159,49,208,8,46,250,152,236,78,108,137];
        assert!(!schnorr_verify_impl(&pk, &sig, &msg));
    }

    #[test]
    fn test_schnorr_v6_false() {
        let pk: [u8; 32] = [223,241,215,127,42,103,28,95,54,24,55,38,219,35,65,190,88,254,174,29,162,222,206,216,67,36,15,123,80,43,166,89];
        let sig: [u8; 64] = [255,249,123,213,117,94,238,164,32,69,58,20,53,82,53,211,130,246,71,47,133,104,161,139,47,5,122,20,96,41,117,86,60,194,121,68,100,10,198,7,205,16,122,225,9,35,217,239,122,115,198,67,225,102,190,94,190,175,163,75,26,197,83,226];
        let msg: [u8; 32] = [36,63,106,136,133,163,8,211,19,25,138,46,3,112,115,68,164,9,56,34,41,159,49,208,8,46,250,152,236,78,108,137];
        assert!(!schnorr_verify_impl(&pk, &sig, &msg));
    }

    #[test]
    fn test_schnorr_v9_false() {
        let pk: [u8; 32] = [223,241,215,127,42,103,28,95,54,24,55,38,219,35,65,190,88,254,174,29,162,222,206,216,67,36,15,123,80,43,166,89];
        let sig: [u8; 64] = [0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,18,61,218,131,40,175,156,35,169,76,31,238,207,209,35,186,79,183,52,118,240,213,148,220,182,92,100,37,189,24,96,81];
        let msg: [u8; 32] = [36,63,106,136,133,163,8,211,19,25,138,46,3,112,115,68,164,9,56,34,41,159,49,208,8,46,250,152,236,78,108,137];
        assert!(!schnorr_verify_impl(&pk, &sig, &msg));
    }

    #[test]
    fn test_scalar_mul_order_g_is_infinity() {
        let g = Pt { x: GX, y: GY, inf: false };
        let r = scalar_mul(&N, &g);
        assert!(r.inf);
    }
}
