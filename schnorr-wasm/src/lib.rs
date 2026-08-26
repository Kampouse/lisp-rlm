#![no_std]
#![no_main]
use core::panic::PanicInfo;
#[panic_handler]
fn _panic(_: &PanicInfo) -> ! { loop {} }

const P: [u64; 4] = [0xFFFFFFFEFFFFFC2F, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF];
const DELTA: u64 = 0x1000003D1;
const N: [u64; 4] = [0xBFD25E8CD0364141, 0xBAAEDCE6AF48A03B, 0xFFFFFFFFFFFFFFFE, 0xFFFFFFFFFFFFFFFF];
const GX: [u64; 4] = [0x59F2815B16F81798, 0x029BFCDB2DCE28D9, 0x55A06295CE870B07, 0x79BE667EF9DCBBAC];
const GY: [u64; 4] = [0x9C47D08FFB10D4B8, 0xFD17B448A6855419, 0x5DA4FBFC0E1108A8, 0x483ADA7726A3C465];

fn is_zero(a: &[u64; 4]) -> bool { a[0] == 0 && a[1] == 0 && a[2] == 0 && a[3] == 0 }
fn add256(a: &[u64; 4], b: &[u64; 4]) -> ([u64; 4], u64) {
    let mut r = [0u64; 4]; let mut c = 0u64;
    for i in 0..4 { let (s1, c1) = a[i].overflowing_add(b[i]); let (s2, c2) = s1.overflowing_add(c); r[i] = s2; c = (c1 as u64) + (c2 as u64); }
    (r, c)
}
fn sub256(a: &[u64; 4], b: &[u64; 4]) -> ([u64; 4], u64) {
    let mut r = [0u64; 4]; let mut borrow = 0u64;
    for i in 0..4 { let ai = a[i]; let bi = b[i].wrapping_add(borrow); borrow = if bi < borrow { 1 } else if ai < bi { 1 } else { 0 }; r[i] = ai.wrapping_sub(bi); }
    (r, borrow)
}
fn cond_sub_p(a: &[u64; 4]) -> [u64; 4] { let (r, borrow) = sub256(a, &P); if borrow == 0 { r } else { *a } }
fn fe_add(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
    let (sum, carry) = add256(a, b);
    if carry > 0 { let (s, _) = add256(&sum, &[DELTA, 0, 0, 0]); cond_sub_p(&s) } else { cond_sub_p(&sum) }
}
fn fe_sub(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
    let (diff, borrow) = sub256(a, b);
    if borrow > 0 { let (s, b2) = sub256(&diff, &[DELTA, 0, 0, 0]); let s = if b2 > 0 { add256(&s, &P).0 } else { s }; cond_sub_p(&s) } else { diff }
}
fn fe_mul(a: &[u64; 4], b: &[u64; 4]) -> [u64; 4] {
    let mask: u128 = (1u128 << 64) - 1;
    let mut r = [0u128; 8];
    for i in 0..4 { let mut carry = 0u128; for j in 0..4 { let prod = (a[i] as u128) * (b[j] as u128); let (s, o1) = r[i + j].overflowing_add(prod); let (s2, o2) = s.overflowing_add(carry); r[i + j] = s2 & mask; carry = ((o1 as u128 + o2 as u128) << 64) + (s2 >> 64); } r[i + 4] += carry; }
    for k in 0..7 { r[k + 1] += r[k] >> 64; r[k] &= mask; }
    let mut low = [r[0] as u64, r[1] as u64, r[2] as u64, r[3] as u64];
    let mut carry: u128 = 0;
    for i in 0..4 { let prod = (r[i + 4] as u128) * (DELTA as u128) + (low[i] as u128) + carry; low[i] = prod as u64; carry = prod >> 64; }
    let cd = carry * (DELTA as u128);
    let lo_cd = cd as u64; let hi_cd = (cd >> 64) as u64;
    let (s, c) = add256(&low, &[lo_cd, hi_cd, 0, 0]);
    if c > 0 { let (s2, _) = add256(&s, &[DELTA, 0, 0, 0]); cond_sub_p(&s2) } else { cond_sub_p(&s) }
}
fn fe_sqr(a: &[u64; 4]) -> [u64; 4] { fe_mul(a, a) }
fn fe_inv(a: &[u64; 4]) -> [u64; 4] {
    let mut result: [u64; 4] = [1, 0, 0, 0];
    let exp_bits: [u64; 4] = [0xFFFFFFFEFFFFFC2D, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF];
    for word in 0..4 { let mut bits = exp_bits[3 - word]; for _ in 0..64 { result = fe_sqr(&result); if bits >> 63 != 0 { result = fe_mul(&result, a); } bits <<= 1; } }
    result
}
fn fe_sqrt(a: &[u64; 4]) -> [u64; 4] {
    let exp: [u64; 4] = [0xFFFFFFFFBFFFFF0C, 0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF, 0x3FFFFFFFFFFFFFFF];
    let mut result: [u64; 4] = [1, 0, 0, 0];
    for word in 0..4 { let mut bits = exp[3 - word]; for _ in 0..64 { result = fe_sqr(&result); if bits >> 63 != 0 { result = fe_mul(&result, a); } bits <<= 1; } }
    result
}
fn fe_mod_n(a: &[u64; 4]) -> [u64; 4] { let (r, borrow) = sub256(a, &N); if borrow == 0 { r } else { *a } }
fn fe_to_be_bytes(a: &[u64; 4]) -> [u8; 32] {
    let b0 = a[0].to_le_bytes(); let b1 = a[1].to_le_bytes(); let b2 = a[2].to_le_bytes(); let b3 = a[3].to_le_bytes();
    let le: [u8; 32] = [b0[0],b0[1],b0[2],b0[3],b0[4],b0[5],b0[6],b0[7],b1[0],b1[1],b1[2],b1[3],b1[4],b1[5],b1[6],b1[7],b2[0],b2[1],b2[2],b2[3],b2[4],b2[5],b2[6],b2[7],b3[0],b3[1],b3[2],b3[3],b3[4],b3[5],b3[6],b3[7]];
    let mut be = [0u8; 32]; for i in 0..32 { be[i] = le[31 - i]; } be
}
fn be_bytes_to_fe(b: &[u8; 32]) -> [u64; 4] {
    let mut rb = [0u8; 32]; for i in 0..32 { rb[i] = b[31 - i]; }
    [u64::from_le_bytes([rb[0],rb[1],rb[2],rb[3],rb[4],rb[5],rb[6],rb[7]]),
     u64::from_le_bytes([rb[8],rb[9],rb[10],rb[11],rb[12],rb[13],rb[14],rb[15]]),
     u64::from_le_bytes([rb[16],rb[17],rb[18],rb[19],rb[20],rb[21],rb[22],rb[23]]),
     u64::from_le_bytes([rb[24],rb[25],rb[26],rb[27],rb[28],rb[29],rb[30],rb[31]])]
}

struct Point { x: [u64; 4], y: [u64; 4], inf: bool }
fn point_double(p: &Point) -> Point {
    if p.inf { return Point { x: [0;4], y: [0;4], inf: true }; }
    let two_y = fe_add(&p.y, &p.y);
    if is_zero(&two_y) { return Point { x: [0;4], y: [0;4], inf: true }; }
    let three = [3u64, 0, 0, 0];
    let x2 = fe_sqr(&p.x);
    let three_x2 = fe_mul(&three, &x2);
    let lambda = fe_mul(&three_x2, &fe_inv(&two_y));
    let lx = fe_sqr(&lambda);
    let two_x = fe_add(&p.x, &p.x);
    let nx = fe_sub(&lx, &two_x);
    let dx = fe_sub(&p.x, &nx);
    let ny = fe_sub(&fe_mul(&lambda, &dx), &p.y);
    Point { x: nx, y: ny, inf: false }
}
fn point_add(a: &Point, b: &Point) -> Point {
    if a.inf { return Point { x: b.x, y: b.y, inf: b.inf }; }
    if b.inf { return Point { x: a.x, y: a.y, inf: a.inf }; }
    if a.x == b.x { if a.y == b.y { return point_double(a); } return Point { x: [0;4], y: [0;4], inf: true }; }
    let dy = fe_sub(&b.y, &a.y); let dx = fe_sub(&b.x, &a.x);
    let lambda = fe_mul(&dy, &fe_inv(&dx));
    let lx = fe_sqr(&lambda);
    let nx = fe_sub(&fe_sub(&lx, &a.x), &b.x);
    let ny = fe_sub(&fe_mul(&lambda, &fe_sub(&a.x, &nx)), &a.y);
    Point { x: nx, y: ny, inf: false }
}
fn scalar_mul(k_bits: &[u64; 4], base: &Point) -> Point {
    let mut result = Point { x: [0;4], y: [0;4], inf: true };
    let mut p = Point { x: base.x, y: base.y, inf: base.inf };
    for word in 0..4 { let mut bits = k_bits[word]; for _ in 0..64 { if bits & 1 != 0 { result = point_add(&result, &p); } p = point_double(&p); bits >>= 1; } }
    result
}

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
fn sha256(data: &[u8], out: &mut [u8; 32]) {
    let mut h = [0x6A09E667u32,0xBB67AE85,0x3C6EF372,0xA54FF53A,0x510E527F,0x9B05688C,0x1F83D9AB,0x5BE0CD19];
    let len = data.len(); let mut buf = [0u8; 64]; let mut off = 0;
    while off + 64 <= len { sha256_block(&mut h, data[off..off+64].try_into().unwrap()); off += 64; }
    let rem = len - off; buf[..rem].copy_from_slice(&data[off..]); buf[rem] = 0x80;
    let bit_len = (len as u64) * 8;
    if rem >= 56 { sha256_block(&mut h, &buf); buf = [0; 64]; }
    buf[56..64].copy_from_slice(&bit_len.to_be_bytes()); sha256_block(&mut h, &buf);
    for i in 0..8 { out[i*4..i*4+4].copy_from_slice(&h[i].to_be_bytes()); }
}
fn sha256_block(h: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 { w[i] = u32::from_be_bytes([block[i*4],block[i*4+1],block[i*4+2],block[i*4+3]]); }
    for i in 16..64 { let s0 = w[i-15].rotate_right(7) ^ w[i-15].rotate_right(18) ^ (w[i-15] >> 3); let s1 = w[i-2].rotate_right(17) ^ w[i-2].rotate_right(19) ^ (w[i-2] >> 10); w[i] = w[i-16].wrapping_add(s0).wrapping_add(w[i-7]).wrapping_add(s1); }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
    for i in 0..64 { let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25); let ch = (e & f) ^ ((!e) & g); let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]); let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22); let maj = (a & b) ^ (a & c) ^ (b & c); let t2 = s0.wrapping_add(maj); hh = g; g = f; f = e; e = d.wrapping_add(t1); d = c; c = b; b = a; a = t1.wrapping_add(t2); }
    h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b); h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e); h[5] = h[5].wrapping_add(f); h[6] = h[6].wrapping_add(g); h[7] = h[7].wrapping_add(hh);
}
fn tagged_hash(tag: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut tag_hash = [0u8; 32]; sha256(tag, &mut tag_hash);
    let mut buf = [0u8; 256]; let total = 64 + msg.len();
    buf[..32].copy_from_slice(&tag_hash); buf[32..64].copy_from_slice(&tag_hash); buf[64..total].copy_from_slice(msg);
    let mut out = [0u8; 32]; sha256(&buf[..total], &mut out); out
}

#[no_mangle]
pub extern "C" fn schnorr_verify_bip340(pk_ptr: i32, sig_ptr: i32, msg_ptr: i32, msg_len: i32) -> i32 {
    let pk: [u8; 32] = unsafe { core::ptr::read(pk_ptr as *const [u8; 32]) };
    let sig: [u8; 64] = unsafe { core::ptr::read(sig_ptr as *const [u8; 64]) };
    let msg: &[u8] = unsafe { core::slice::from_raw_parts(msg_ptr as *const u8, msg_len as usize) };

    let pk_x = be_bytes_to_fe(&pk);
    let r_bytes: [u8; 32] = sig[..32].try_into().unwrap();
    let s_bytes: [u8; 32] = sig[32..].try_into().unwrap();
    let r = be_bytes_to_fe(&r_bytes);
    let s = be_bytes_to_fe(&s_bytes);

    let r_mod_n = fe_mod_n(&r);
    if is_zero(&r_mod_n) { return 0; }
    let s_mod_n = fe_mod_n(&s);
    if is_zero(&s_mod_n) { return 0; }

    let x3 = fe_mul(&fe_mul(&pk_x, &pk_x), &pk_x);
    let y_sq = fe_add(&x3, &[7, 0, 0, 0]);
    let y = fe_sqrt(&y_sq);
    if fe_sqr(&y) != y_sq { return 0; }
    let y_bytes = fe_to_be_bytes(&y);
    let py = if (y_bytes[31] & 1) != 0 { fe_sub(&P, &y) } else { y };
    let p_point = Point { x: pk_x, y: py, inf: false };

    let tag = b"BIP0340/challenge";
    let mut hash_input = [0u8; 256];
    let total = 64 + msg.len();
    if total > 256 { return 0; }
    hash_input[..32].copy_from_slice(&sig[..32]);
    hash_input[32..64].copy_from_slice(&pk);
    hash_input[64..total].copy_from_slice(msg);
    let e_hash = tagged_hash(tag, &hash_input[..total]);
    let e = fe_mod_n(&be_bytes_to_fe(&e_hash));

    let g = Point { x: GX, y: GY, inf: false };
    let sg = scalar_mul(&s_mod_n, &g);
    let ep = scalar_mul(&e, &p_point);
    let neg_ep = Point { x: ep.x, y: fe_sub(&P, &ep.y), inf: ep.inf };
    let r_point = point_add(&sg, &neg_ep);

    if r_point.inf { return 0; }
    if fe_mod_n(&r_point.x) != r_mod_n { return 0; }
    let ry_bytes = fe_to_be_bytes(&r_point.y);
    if (ry_bytes[31] & 1) != 0 { return 0; }

    1
}
