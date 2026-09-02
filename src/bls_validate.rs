//! BLS12-381 host-call validation for the mock runner — verbatim port of
//! `nearcore/runtime/near-vm-runner/src/logic/bls12381.rs` (near-vm-runner
//! 0.37.x, blst 0.3.x, V1 = NEP-488 `not_in_group_fix` semantics, which is
//! what current testnet/mainnet run).
//!
//! WHY THIS EXISTS: the mock's BLS stubs previously checked only `len %
//! stride` and never read guest bytes, so wire-encoding bugs that testnet
//! rejects (malformed points, non-canonical field elements, bad sign bytes)
//! sailed through locally. This port makes the mock byte-faithful to the
//! chain: real curve/subgroup validation, nearcore ret codes, nearcore
//! output encodings (sign-free serialized points), and nearcore's
//! trap-vs-ret-code split (`BLS12381InvalidInput` host error on bad total
//! length; ret 1 on malformed points/signs).
//!
//! Native targets only (`cfg(not(target_arch = "wasm32"))`): the mock is a
//! host binary; the wasm32 compiler must never link blst/cc.

/// NEP-488 corner-case fix (accept (0,±2) in sums/decompress) — version 1
/// is stabilized on current protocol; the mock always runs V1 like testnet.
pub const BLS12381_NOT_IN_GROUP_FIX_VERSION: u32 = 1;

const BLS_BOOL_SIZE: usize = 1;
const BLS_SCALAR_SIZE: usize = 32;
const BLS_FP_SIZE: usize = 48;
const BLS_FP2_SIZE: usize = 96;
const BLS_P1_SIZE: usize = 96;
const BLS_P2_SIZE: usize = 192;
const BLS_P1_COMPRESS_SIZE: usize = 48;
const BLS_P2_COMPRESS_SIZE: usize = 96;

/// Host-function selector for `eval` — matches near_mock's import order.
pub mod kind {
    pub const P1_SUM: u8 = 0;
    pub const P2_SUM: u8 = 1;
    pub const G1_MULTIEXP: u8 = 2;
    pub const G2_MULTIEXP: u8 = 3;
    pub const MAP_FP_TO_G1: u8 = 4;
    pub const MAP_FP2_TO_G2: u8 = 5;
    pub const P1_DECOMPRESS: u8 = 6;
    pub const P2_DECOMPRESS: u8 = 7;
}

/// Dispatch one bls12381 host call. `Ok(bytes)` = nearcore ret 0 with the
/// register payload; `Err(())` = nearcore ret 1 (malformed point/sign;
/// register untouched). Bad total length is a HOST ERROR and must be
/// surfaced as a trap by the caller — see `check_error`/`pairing_check`.
pub fn eval(kind: u8, data: &[u8]) -> Result<Option<Vec<u8>>, HostError> {
    match kind {
        kind::P1_SUM => p1_sum(data, BLS12381_NOT_IN_GROUP_FIX_VERSION),
        kind::P2_SUM => p2_sum(data, BLS12381_NOT_IN_GROUP_FIX_VERSION),
        kind::G1_MULTIEXP => g1_multiexp(data, BLS12381_NOT_IN_GROUP_FIX_VERSION),
        kind::G2_MULTIEXP => g2_multiexp(data, BLS12381_NOT_IN_GROUP_FIX_VERSION),
        kind::MAP_FP_TO_G1 => map_fp_to_g1(data, BLS12381_NOT_IN_GROUP_FIX_VERSION),
        kind::MAP_FP2_TO_G2 => map_fp2_to_g2(data, BLS12381_NOT_IN_GROUP_FIX_VERSION),
        kind::P1_DECOMPRESS => p1_decompress(data, BLS12381_NOT_IN_GROUP_FIX_VERSION),
        kind::P2_DECOMPRESS => p2_decompress(data, BLS12381_NOT_IN_GROUP_FIX_VERSION),
        _ => Ok(Some(Vec::new())),
    }
}

/// nearcore's `HostError::BLS12381InvalidInput` — the mock traps on this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    pub msg: String,
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BLS12381InvalidInput: {}", self.msg)
    }
}

macro_rules! bls12381_fn {
    (
        $p_sum:ident,
        $g_multiexp:ident,
        $p_decompress:ident,
        $map_fp_to_g:ident,
        $BLS_P_SIZE:expr,
        $BLS_FP_SIZE:expr,
        $BLS_P_COMPRESS_SIZE:expr,
        $blst_p:ident,
        $blst_p_affine:ident,
        $blst_p_deserialize:ident,
        $blst_p_from_affine:ident,
        $blst_p_cneg:ident,
        $blst_p_add_or_double:ident,
        $blst_p_to_affine:ident,
        $blst_p_affine_serialize:ident,
        $blst_p_in_g:ident,
        $blst_p_mult:ident,
        $read_fp_point:ident,
        $blst_map_to_g:ident,
        $blst_p_uncompress:ident,
        $parse_p:ident,
        $serialize_p:ident
    ) => {
        fn $parse_p(point_data: &[u8], version: u32) -> Option<blst::$blst_p> {
            if point_data[0] & 0x80 != 0 {
                return None;
            }

            let mut pk_aff = blst::$blst_p_affine::default();
            let error_code = unsafe { blst::$blst_p_deserialize(&mut pk_aff, point_data.as_ptr()) };
            let success = error_code == blst::BLST_ERROR::BLST_SUCCESS
                || (version >= BLS12381_NOT_IN_GROUP_FIX_VERSION
                    && error_code == blst::BLST_ERROR::BLST_POINT_NOT_IN_GROUP);
            if !success {
                return None;
            }

            let mut pk = blst::$blst_p::default();
            unsafe {
                blst::$blst_p_from_affine(&mut pk, &pk_aff);
            }
            Some(pk)
        }

        fn $serialize_p(res_pk: &blst::$blst_p) -> Vec<u8> {
            let mut res_affine = blst::$blst_p_affine::default();

            unsafe {
                blst::$blst_p_to_affine(&mut res_affine, res_pk);
            }

            let mut res = [0u8; $BLS_P_SIZE];
            unsafe {
                blst::$blst_p_affine_serialize(res.as_mut_ptr(), &res_affine);
            }

            res.to_vec()
        }

        pub fn $p_sum(data: &[u8], version: u32) -> Result<Option<Vec<u8>>, HostError> {
            const ITEM_SIZE: usize = BLS_BOOL_SIZE + $BLS_P_SIZE;
            check_input_size(data, ITEM_SIZE, stringify!($p_sum))?;

            let mut res_pk = blst::$blst_p::default();

            for item_data in data.chunks_exact(ITEM_SIZE) {
                let (sign_data, point_data) = item_data.split_at(BLS_BOOL_SIZE);
                debug_assert_eq!(point_data.len(), $BLS_P_SIZE);

                let mut pk = match $parse_p(point_data, version) {
                    Some(pk) => pk,
                    None => return Ok(None),
                };

                let sign = sign_data[0];

                if sign == 1 {
                    unsafe {
                        blst::$blst_p_cneg(&mut pk, true);
                    }
                } else if sign != 0 {
                    return Ok(None);
                }

                unsafe {
                    blst::$blst_p_add_or_double(&mut res_pk, &res_pk, &pk);
                }
            }

            Ok(Some($serialize_p(&res_pk)))
        }

        pub fn $g_multiexp(data: &[u8], version: u32) -> Result<Option<Vec<u8>>, HostError> {
            const ITEM_SIZE: usize = $BLS_P_SIZE + BLS_SCALAR_SIZE;
            check_input_size(data, ITEM_SIZE, stringify!($g_multiexp))?;

            let mut res_pk = blst::$blst_p::default();

            for item_data in data.chunks_exact(ITEM_SIZE) {
                let (point_data, scalar_data) = item_data.split_at($BLS_P_SIZE);
                debug_assert_eq!(scalar_data.len(), BLS_SCALAR_SIZE);

                let pk = match $parse_p(point_data, version) {
                    Some(pk) => pk,
                    None => return Ok(None),
                };

                if unsafe { blst::$blst_p_in_g(&pk) } != true {
                    return Ok(None);
                }

                let mut pk_mul = blst::$blst_p::default();
                unsafe {
                    blst::$blst_p_mult(&mut pk_mul, &pk, scalar_data.as_ptr(), BLS_SCALAR_SIZE * 8);
                }

                unsafe {
                    blst::$blst_p_add_or_double(&mut res_pk, &res_pk, &pk_mul);
                }
            }

            Ok(Some($serialize_p(&res_pk)))
        }

        pub fn $p_decompress(data: &[u8], version: u32) -> Result<Option<Vec<u8>>, HostError> {
            const ITEM_SIZE: usize = $BLS_P_COMPRESS_SIZE;
            check_input_size(data, ITEM_SIZE, stringify!($p_decompress))?;
            let elements_count = data.len() / ITEM_SIZE;

            let mut res = Vec::<u8>::with_capacity(elements_count * $BLS_P_SIZE);

            for item_data in data.chunks_exact(ITEM_SIZE) {
                // V1 path only: the mock targets current testnet protocol
                // (`not_in_group_fix` on), so the legacy `min_pk` fallback
                // from nearcore's pre-V1 branch is deliberately omitted.
                let pk_ser = if item_data[0] & 0x80 != 0 {
                    let mut pk = blst::$blst_p_affine::default();
                    let err = unsafe { blst::$blst_p_uncompress(&mut pk, item_data.as_ptr()) };
                    if err != blst::BLST_ERROR::BLST_SUCCESS
                        && err != blst::BLST_ERROR::BLST_POINT_NOT_IN_GROUP
                    {
                        return Ok(None);
                    }

                    let mut ser = [0u8; $BLS_P_SIZE];
                    unsafe {
                        blst::$blst_p_affine_serialize(ser.as_mut_ptr(), &pk);
                    }
                    ser.to_vec()
                } else {
                    return Ok(None);
                };

                res.extend_from_slice(pk_ser.as_slice());
            }

            Ok(Some(res))
        }

        pub fn $map_fp_to_g(data: &[u8], _version: u32) -> Result<Option<Vec<u8>>, HostError> {
            const ITEM_SIZE: usize = $BLS_FP_SIZE;
            check_input_size(data, ITEM_SIZE, stringify!($map_fp_to_g))?;
            let elements_count: usize = data.len() / ITEM_SIZE;

            let mut res_concat: Vec<u8> = Vec::with_capacity($BLS_P_SIZE * elements_count);

            for item_data in data.chunks_exact(ITEM_SIZE) {
                let fp_point = match $read_fp_point(item_data) {
                    Some(fp_point) => fp_point,
                    None => return Ok(None),
                };

                let mut g_point = blst::$blst_p::default();
                unsafe {
                    blst::$blst_map_to_g(&mut g_point, &fp_point, std::ptr::null());
                }

                let mut res = $serialize_p(&g_point);
                res_concat.append(&mut res);
            }

            Ok(Some(res_concat))
        }
    };
}

bls12381_fn!(
    p1_sum,
    g1_multiexp,
    p1_decompress,
    map_fp_to_g1,
    BLS_P1_SIZE,
    BLS_FP_SIZE,
    BLS_P1_COMPRESS_SIZE,
    blst_p1,
    blst_p1_affine,
    blst_p1_deserialize,
    blst_p1_from_affine,
    blst_p1_cneg,
    blst_p1_add_or_double,
    blst_p1_to_affine,
    blst_p1_affine_serialize,
    blst_p1_in_g1,
    blst_p1_mult,
    read_fp_point,
    blst_map_to_g1,
    blst_p1_uncompress,
    parse_p1,
    serialize_p1
);

bls12381_fn!(
    p2_sum,
    g2_multiexp,
    p2_decompress,
    map_fp2_to_g2,
    BLS_P2_SIZE,
    BLS_FP2_SIZE,
    BLS_P2_COMPRESS_SIZE,
    blst_p2,
    blst_p2_affine,
    blst_p2_deserialize,
    blst_p2_from_affine,
    blst_p2_cneg,
    blst_p2_add_or_double,
    blst_p2_to_affine,
    blst_p2_affine_serialize,
    blst_p2_in_g2,
    blst_p2_mult,
    read_fp2_point,
    blst_map_to_g2,
    blst_p2_uncompress,
    parse_p2,
    serialize_p2
);

/// nearcore `pairing_check`: ret 0 = check passed, 1 = malformed input,
/// 2 = well-formed but pairing ≠ 1. `Err` = BLS12381InvalidInput host
/// error (length not a multiple of 288) — caller traps, like testnet.
pub fn pairing_check(data: &[u8]) -> Result<u64, HostError> {
    const ITEM_SIZE: usize = BLS_P1_SIZE + BLS_P2_SIZE;
    check_input_size(data, ITEM_SIZE, "bls12381_pairing_check")?;
    let elements_count = data.len() / ITEM_SIZE;

    let mut blst_g1_list: Vec<blst::blst_p1_affine> =
        vec![blst::blst_p1_affine::default(); elements_count];
    let mut blst_g2_list: Vec<blst::blst_p2_affine> =
        vec![blst::blst_p2_affine::default(); elements_count];

    for (i, item_data) in data.chunks_exact(ITEM_SIZE).enumerate() {
        let (point1_data, point2_data) = item_data.split_at(BLS_P1_SIZE);
        debug_assert_eq!(point2_data.len(), BLS_P2_SIZE);

        if point1_data[0] & 0x80 != 0 {
            return Ok(1);
        }

        let error_code =
            unsafe { blst::blst_p1_deserialize(&mut blst_g1_list[i], point1_data.as_ptr()) };

        if error_code != blst::BLST_ERROR::BLST_SUCCESS {
            return Ok(1);
        }

        let g1_check = unsafe { blst::blst_p1_affine_in_g1(&blst_g1_list[i]) };
        if !g1_check {
            return Ok(1);
        }

        if point2_data[0] & 0x80 != 0 {
            return Ok(1);
        }

        let error_code =
            unsafe { blst::blst_p2_deserialize(&mut blst_g2_list[i], point2_data.as_ptr()) };
        if error_code != blst::BLST_ERROR::BLST_SUCCESS {
            return Ok(1);
        }

        let g2_check = unsafe { blst::blst_p2_affine_in_g2(&blst_g2_list[i]) };
        if !g2_check {
            return Ok(1);
        }
    }

    let mut pairing_fp12 = blst::blst_fp12::default();
    for i in 0..elements_count {
        pairing_fp12 *= blst::blst_fp12::miller_loop(&blst_g2_list[i], &blst_g1_list[i]);
    }
    pairing_fp12 = pairing_fp12.final_exp();

    let pairing_res = unsafe { blst::blst_fp12_is_one(&pairing_fp12) };

    if pairing_res {
        Ok(0)
    } else {
        Ok(2)
    }
}

fn read_fp_point(item_data: &[u8]) -> Option<blst::blst_fp> {
    let mut fp_point = blst::blst_fp::default();
    unsafe {
        blst::blst_fp_from_bendian(&mut fp_point, item_data.as_ptr());
    }

    let mut fp_row: [u8; BLS_FP_SIZE] = [0u8; BLS_FP_SIZE];
    unsafe {
        blst::blst_bendian_from_fp(fp_row.as_mut_ptr(), &fp_point);
    }

    for j in 0..BLS_FP_SIZE {
        if fp_row[j] != item_data[j] {
            return None;
        }
    }

    Some(fp_point)
}

fn read_fp2_point(item_data: &[u8]) -> Option<blst::blst_fp2> {
    let mut c_fp1 = [blst::blst_fp::default(); 2];

    unsafe {
        blst::blst_fp_from_bendian(&mut c_fp1[1], item_data[..BLS_FP_SIZE].as_ptr());
        blst::blst_fp_from_bendian(&mut c_fp1[0], item_data[BLS_FP_SIZE..].as_ptr());
    }

    let mut fp_row: [u8; BLS_FP_SIZE] = [0u8; BLS_FP_SIZE];
    unsafe {
        blst::blst_bendian_from_fp(fp_row.as_mut_ptr(), &c_fp1[0]);
    }

    for j in BLS_FP_SIZE..BLS_FP2_SIZE {
        if fp_row[j - BLS_FP_SIZE] != item_data[j] {
            return None;
        }
    }

    unsafe {
        blst::blst_bendian_from_fp(fp_row.as_mut_ptr(), &c_fp1[1]);
    }

    for j in 0..BLS_FP_SIZE {
        if fp_row[j] != item_data[j] {
            return None;
        }
    }

    Some(blst::blst_fp2 { fp: c_fp1 })
}

fn check_input_size(data: &[u8], item_size: usize, fn_name: &str) -> Result<(), HostError> {
    if data.len() % item_size != 0 {
        return Err(HostError {
            msg: format!(
                "Incorrect input length for {}: {} is not divisible by {}",
                fn_name,
                data.len(),
                item_size
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_total_length_is_host_error_not_ret1() {
        // 96 bytes is not a multiple of 97 (p1_sum item size) → HOST ERROR
        assert!(p1_sum(&[0u8; 96], 1).is_err());
        // 288 % 97 != 0 → HOST ERROR
        assert!(p1_sum(&[0u8; 288], 1).is_err());
        // pairing: 320 % 288 != 0 → HOST ERROR
        assert!(pairing_check(&[0u8; 320]).is_err());
        // empty input is VALID (identity) for every op
        assert_eq!(p1_sum(&[], 1).unwrap().unwrap().len(), 96);
        assert_eq!(pairing_check(&[]).unwrap(), 0);
    }

    #[test]
    fn malformed_sign_byte_is_ret1() {
        // one well-formed-length item with sign byte 2 → ret 1, no trap
        let item = [2u8; 97];
        assert_eq!(p1_sum(&item, 1).unwrap(), None);
        let item = [3u8; 97];
        assert_eq!(p1_sum(&item, 1).unwrap(), None);
    }

    #[test]
    fn all_zero_input_is_malformed_not_crash() {
        // 97 zero bytes: sign 0 ok, but (0,0) is not on the curve → ret 1
        assert_eq!(p1_sum(&[0u8; 97], 1).unwrap(), None);
        // non-canonical Fp element (>= modulus, e.g. all 0xFF) → ret 1
        assert_eq!(map_fp_to_g1(&[0xFFu8; 48], 1).unwrap(), None);
    }
}
