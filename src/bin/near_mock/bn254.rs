//! Real alt_bn128 (BN254) precompile math for the near-mock — verbatim port
//! of vendor/near-vm-runner/src/logic/alt_bn128.rs (substrate-bn), replacing
//! the fixed-shape tag-blob stubs. Encode/decode must stay byte-identical to
//! production: big-endian Fq elements, LE u128 halves inside U256 (nearcore
//! quirk preserved by decode_u256).
//!
//! Mock ABI note: nearcore's pairing_check returns 1 when the pairing HOLDS
//! (EVM-compatible), 0 otherwise; invalid input is a host error (trap).

use bn::{AffineG1, AffineG2, Fq, Fq2, Fr, Group, Gt, G1, G2};

const BOOL_SIZE: usize = 1;
const SCALAR_SIZE: usize = 256 / 8;
const POINT_SIZE: usize = SCALAR_SIZE * 2;

#[derive(Debug)]
pub(crate) struct InvalidInput {
    pub(crate) msg: String,
}

impl InvalidInput {
    fn new(msg: &str, bad_value: &[u8]) -> InvalidInput {
        let msg = format!("{msg}: {bad_value:X?}");
        InvalidInput { msg }
    }
}

impl std::fmt::Display for InvalidInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

pub(crate) fn split_elements<const ELEMENT_SIZE: usize>(
    data: &[u8],
) -> Result<&[[u8; ELEMENT_SIZE]], InvalidInput> {
    // same semantics as stdx::as_chunks_exact: error on a ragged tail
    if data.len() % ELEMENT_SIZE != 0 {
        return Err(InvalidInput {
            msg: format!(
                "Invalid input length {} for {}-byte elements (input: {:X?})",
                data.len(),
                ELEMENT_SIZE,
                &data[..data.len().min(24)]
            ),
        });
    }
    Ok(unsafe {
        std::slice::from_raw_parts(
            data.as_ptr() as *const [u8; ELEMENT_SIZE],
            data.len() / ELEMENT_SIZE,
        )
    })
}

pub(crate) const G1_MULTIEXP_ELEMENT_SIZE: usize = POINT_SIZE + SCALAR_SIZE;

pub(crate) fn g1_multiexp(
    elements: &[[u8; G1_MULTIEXP_ELEMENT_SIZE]],
) -> Result<[u8; POINT_SIZE], InvalidInput> {
    let elements: Vec<(G1, Fr)> = elements
        .iter()
        .map(|chunk| {
            let (g1, fr) = chunk.split_at(POINT_SIZE);
            let g1 = decode_g1(g1.try_into().unwrap())?;
            let fr = decode_fr(fr.try_into().unwrap())?;
            Ok((g1, fr))
        })
        .collect::<Result<Vec<_>, InvalidInput>>()?;

    let res = G1::multiexp(&elements);

    Ok(encode_g1(res))
}

pub(crate) const G1_SUM_ELEMENT_SIZE: usize = BOOL_SIZE + POINT_SIZE;

pub(crate) fn g1_sum(
    elements: &[[u8; G1_SUM_ELEMENT_SIZE]],
) -> Result<[u8; POINT_SIZE], InvalidInput> {
    let elements: Vec<(bool, G1)> = elements
        .iter()
        .map(|chunk| {
            let sign = decode_bool(&chunk[..BOOL_SIZE].try_into().unwrap())?;
            let g1 = decode_g1(chunk[BOOL_SIZE..].try_into().unwrap())?;
            Ok((sign, g1))
        })
        .collect::<Result<Vec<_>, InvalidInput>>()?;

    let res = elements.iter().fold(
        G1::zero(),
        |acc, &(sign, x)| if sign { acc - x } else { acc + x },
    );

    Ok(encode_g1(res))
}

pub(crate) const PAIRING_CHECK_ELEMENT_SIZE: usize = POINT_SIZE + POINT_SIZE * 2;

pub(crate) fn pairing_check(
    elements: &[[u8; PAIRING_CHECK_ELEMENT_SIZE]],
) -> Result<bool, InvalidInput> {
    let elements: Vec<(G1, G2)> = elements
        .iter()
        .map(|chunk| {
            let (g1, g2) = chunk.split_at(POINT_SIZE);
            let g1 = decode_g1(g1.try_into().unwrap())?;
            let g2 = decode_g2(g2.try_into().unwrap())?;
            Ok((g1, g2))
        })
        .collect::<Result<Vec<_>, InvalidInput>>()?;

    let res = bn::pairing_batch(&elements) == Gt::one();

    Ok(res)
}

fn encode_g1(val: G1) -> [u8; POINT_SIZE] {
    let (x, y) = AffineG1::from_jacobian(val)
        .map(|p| (p.x(), p.y()))
        .unwrap_or_else(|| (Fq::zero(), Fq::zero()));
    let x = encode_fq(x);
    let y = encode_fq(y);
    let mut out = [0u8; POINT_SIZE];
    out[..SCALAR_SIZE].copy_from_slice(&x);
    out[SCALAR_SIZE..].copy_from_slice(&y);
    out
}

fn encode_fq(val: Fq) -> [u8; SCALAR_SIZE] {
    encode_u256(val.into_u256())
}

fn encode_u256(val: bn::arith::U256) -> [u8; SCALAR_SIZE] {
    // nearcore: [lo, hi] u128 halves, each to_le_bytes
    let [lo, hi] = val.0;
    let mut out = [0u8; SCALAR_SIZE];
    out[..16].copy_from_slice(&lo.to_le_bytes());
    out[16..].copy_from_slice(&hi.to_le_bytes());
    out
}

fn decode_g1(raw: &[u8; POINT_SIZE]) -> Result<G1, InvalidInput> {
    let (x, y) = raw.split_at(SCALAR_SIZE);
    let x = decode_fq(x.try_into().unwrap())?;
    let y = decode_fq(y.try_into().unwrap())?;
    if x.is_zero() && y.is_zero() {
        Ok(G1::zero())
    } else {
        AffineG1::new(x, y)
            .map_err(|_err| InvalidInput::new("invalid g1", raw))
            .map(G1::from)
    }
}

fn decode_fq(raw: &[u8; SCALAR_SIZE]) -> Result<Fq, InvalidInput> {
    let val = decode_u256(raw);
    Fq::from_u256(val).map_err(|_| InvalidInput::new("invalid fq", raw))
}

fn decode_g2(raw: &[u8; 2 * POINT_SIZE]) -> Result<G2, InvalidInput> {
    let (x, y) = raw.split_at(POINT_SIZE);
    let x = decode_fq2(x.try_into().unwrap())?;
    let y = decode_fq2(y.try_into().unwrap())?;
    if x.is_zero() && y.is_zero() {
        Ok(G2::zero())
    } else {
        AffineG2::new(x, y)
            .map_err(|_err| InvalidInput::new("invalid g2", raw))
            .map(G2::from)
    }
}

fn decode_fq2(raw: &[u8; 2 * SCALAR_SIZE]) -> Result<Fq2, InvalidInput> {
    let (real, imaginary) = raw.split_at(SCALAR_SIZE);
    let real = decode_fq(real.try_into().unwrap())?;
    let imaginary = decode_fq(imaginary.try_into().unwrap())?;
    Ok(Fq2::new(real, imaginary))
}

fn decode_fr(raw: &[u8; SCALAR_SIZE]) -> Result<Fr, InvalidInput> {
    let val = decode_u256(raw);
    Fr::new(val).ok_or_else(|| InvalidInput::new("invalid fr", raw))
}

fn decode_u256(raw: &[u8; SCALAR_SIZE]) -> bn::arith::U256 {
    let (lo, hi) = raw.split_at(16);
    let lo = u128::from_le_bytes(lo.try_into().unwrap());
    let hi = u128::from_le_bytes(hi.try_into().unwrap());
    bn::arith::U256([lo, hi])
}

fn decode_bool(raw: &[u8; BOOL_SIZE]) -> Result<bool, InvalidInput> {
    match raw {
        [0] => Ok(false),
        [1] => Ok(true),
        _ => Err(InvalidInput::new("invalid bool", raw)),
    }
}
