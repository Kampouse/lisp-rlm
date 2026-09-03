//! Wasm `name` custom section: encode, decode, remap.
//!
//! The emitter writes function names after tree-shaking (indices stable per
//! module), the schnorr stitcher remaps them across the lib insertion, and
//! wasm-opt `-g` carries them through its own renumbering. Decoding powers
//! `near-mock symbolicate`: a trap's `<wasm function N>` becomes a function
//! name, and the compile-time `.wasm.map` sidecar (name → source form) turns
//! that into the offending `(define ...)` — locally and for testnet traps
//! alike (download the deployed wasm, decode, look up).
//!
//! Hand-rolled decode (LEB128 + UTF-8): ~30 lines, immune to wasmparser
//! version churn. Encoding uses wasm-encoder's NameSection so the output
//! matches the spec exactly.

use wasm_encoder::{Encode, NameMap, NameSection};

/// Encode a function-name map as a `NameSection` (implements
/// `wasm_encoder::Section`, so it can go straight into `m.section(...)`).
pub fn name_section(function_names: &[(u32, String)]) -> NameSection {
    let mut sec = NameSection::new();
    let mut map = NameMap::new();
    for (idx, name) in function_names {
        map.append(*idx, name);
    }
    sec.functions(&map);
    sec
}

/// Encode a function-name map as a complete `name` custom section payload
/// (section id 0, "name", function subsection 1).
pub fn encode_function_names(function_names: &[(u32, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    name_section(function_names).encode(&mut out);
    out
}

/// Standard section walk: (id, payload) pairs at the top level.
fn sections(data: &[u8]) -> Vec<(u8, &[u8])> {
    let mut out = Vec::new();
    let mut p = 8usize; // skip magic + version
    while p < data.len() {
        let id = data[p];
        p += 1;
        let Some((size, np)) = read_leb_u32(data, p) else { break };
        p = np;
        let end = (p + size as usize).min(data.len());
        out.push((id, &data[p..end]));
        p = end;
    }
    out
}

/// LEB128 u32 → (value, next offset).
fn read_leb_u32(d: &[u8], mut p: usize) -> Option<(u32, usize)> {
    let mut result: u64 = 0;
    let mut shift = 0;
    loop {
        let b = *d.get(p)?;
        p += 1;
        result |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((result as u32, p));
        }
        shift += 7;
        if shift > 35 {
            return None;
        }
    }
}

/// Extract the function-name map from a module's `name` section, if present.
/// Returns None when the section is missing or malformed (symbolication then
/// degrades to raw indices — never errors).
pub fn decode_function_names(wasm: &[u8]) -> Option<Vec<(u32, String)>> {
    for (id, payload) in sections(wasm) {
        if id != 0 {
            continue;
        }
        // custom section: name_len, name, then subsections
        let (nlen, mut p) = read_leb_u32(payload, 0)?;
        if payload.get(p..p + nlen as usize)? != b"name" {
            continue;
        }
        p += nlen as usize;
        while p < payload.len() {
            let sub_id = *payload.get(p)?;
            p += 1;
            let (size, p2) = read_leb_u32(payload, p)?;
            let end = (p2 + size as usize).min(payload.len());
            if sub_id == 1 {
                // function names: count, then (idx, name)*
                let (count, mut q) = read_leb_u32(payload, p2)?;
                let mut out = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let (idx, q2) = read_leb_u32(payload, q)?;
                    let (nlen, q3) = read_leb_u32(payload, q2)?;
                    let name = payload.get(q3..q3 + nlen as usize)?;
                    out.push((idx, String::from_utf8_lossy(name).into_owned()));
                    q = q3 + nlen as usize;
                }
                return Some(out);
            }
            p = end;
        }
    }
    None
}

/// Remap indices through `f`; entries whose mapping is None are dropped
/// (matches wasm-opt deleting the function).
pub fn remap_function_names(
    names: &[(u32, String)],
    mut f: impl FnMut(u32) -> Option<u32>,
) -> Vec<(u32, String)> {
    names
        .iter()
        .filter_map(|(idx, name)| f(*idx).map(|new| (new, name.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let names = vec![
            (0u32, "env.input".to_string()),
            (3u32, "run".to_string()),
            (7u32, "__lambda_2".to_string()),
        ];
        // embed the section in a minimal module, as the emitter does
        let mut m = wasm_encoder::Module::new();
        m.section(&name_section(&names));
        let wasm = m.finish();
        let decoded = decode_function_names(&wasm).expect("own-encoded section must decode");
        assert_eq!(decoded, names);
    }

    #[test]
    fn decode_none_when_absent() {
        // minimal empty module (magic + version only)
        let empty = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        assert!(decode_function_names(&empty).is_none());
    }

    #[test]
    fn remap_drops_unmapped_and_translates() {
        let names = vec![(0u32, "a".into()), (1u32, "gone".into()), (2u32, "c".into())];
        let out = remap_function_names(&names, |i| if i == 1 { None } else { Some(i + 10) });
        assert_eq!(out, vec![(10, "a".into()), (12, "c".into())]);
    }
}
