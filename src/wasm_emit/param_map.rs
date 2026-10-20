//! `near:params` custom section — declares which JSON keys a contract reads
//! from its entry-point argument via bare `(near/json_get_str "key")` style
//! reads. Mirrors the checker's bare-JSON walk (src/typing/checker.rs) so the
//! declared set matches exactly what the type checker validated.

use crate::types::LispVal;

/// JSON param-read ops whose 2nd element (when a string literal) is a key.
const PARAM_OPS: [&str; 8] = [
    "near/json_get_str",
    "near/json_get_int",
    "near/json_get_u128",
    "near/json_get_arr",
    "json/get",
    "json-get-str",
    "json-get-float",
    "json-get-in",
];

/// Collect the set of JSON keys read from the entry-point argument.
/// Keys read inside a `(default ...)` fallback still count — the contract
/// may read them. Non-literal keys are skipped (can't be declared statically).
pub fn collect_param_keys(exprs: &[LispVal]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    fn walk(expr: &LispVal, keys: &mut Vec<String>) {
        if let LispVal::List(list) = expr {
            if !list.is_empty() {
                if let LispVal::Sym(head) = &list[0] {
                    if head == "default" {
                        for sub in list.iter().skip(1) {
                            walk(sub, keys);
                        }
                        return;
                    }
                    if PARAM_OPS.contains(&head.as_str()) {
                        if let Some(LispVal::Str(key)) = list.get(1) {
                            if !keys.iter().any(|k| k == key) {
                                keys.push(key.clone());
                            }
                        }
                    }
                }
            }
            for sub in list {
                walk(sub, keys);
            }
        }
    }
    for e in exprs {
        walk(e, &mut keys);
    }
    keys.sort();
    keys
}

fn leb_u32(out: &mut Vec<u8>, mut v: u32) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    }
}

/// Encode a complete `near:params` custom section (id 0): name, then a count
/// followed by length-prefixed UTF-8 keys.
pub fn encode_params_section(keys: &[String]) -> Vec<u8> {
    let mut payload = Vec::new();
    leb_u32(&mut payload, b"near:params".len() as u32);
    payload.extend_from_slice(b"near:params");
    leb_u32(&mut payload, keys.len() as u32);
    for k in keys {
        leb_u32(&mut payload, k.len() as u32);
        payload.extend_from_slice(k.as_bytes());
    }
    let mut sec = Vec::with_capacity(payload.len() + 5);
    sec.push(0u8); // custom section id
    leb_u32(&mut sec, payload.len() as u32);
    sec.extend_from_slice(&payload);
    sec
}

/// Append a `near:params` custom section to finished wasm bytes.
/// Inserts immediately after the magic+version header — custom sections are
/// valid anywhere in the module, and post-header is safe even when other
/// tooling (e.g. name-section emission) has already finalized the module.
pub fn append_params_section(wasm: &mut Vec<u8>, keys: &[String]) {
    if keys.is_empty() || wasm.len() < 8 || wasm[..4] != [0x00, 0x61, 0x73, 0x6d] {
        return;
    }
    let sec = encode_params_section(keys);
    wasm.splice(8..8, sec);
}

/// Extract declared param keys from a module's `near:params` section, if any.
pub fn decode_params_section(wasm: &[u8]) -> Option<Vec<String>> {
    let mut p = 8usize; // magic + version
    while p < wasm.len() {
        let id = *wasm.get(p)?;
        p += 1;
        // LEB u32 size
        let mut size: u64 = 0;
        let mut shift = 0;
        loop {
            let b = *wasm.get(p)?;
            p += 1;
            size |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 35 {
                return None;
            }
        }
        let end = (p + size as usize).min(wasm.len());
        if id == 0 {
            let payload = wasm.get(p..end)?;
            let mut q = 0usize;
            // name length LEB
            let mut n: u64 = 0;
            let mut s = 0;
            loop {
                let b = match payload.get(q) {
                    Some(&b) => b,
                    None => return None,
                };
                q += 1;
                n |= ((b & 0x7f) as u64) << s;
                if b & 0x80 == 0 {
                    break;
                }
                s += 7;
            }
            let nlen = n as usize;
            if payload.get(q..q + nlen)? == b"near:params" {
                q += nlen;
                // count
                let mut count: u64 = 0;
                let mut s = 0;
                loop {
                    let b = *payload.get(q)?;
                    q += 1;
                    count |= ((b & 0x7f) as u64) << s;
                    if b & 0x80 == 0 {
                        break;
                    }
                    s += 7;
                }
                let mut out = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let mut klen: u64 = 0;
                    let mut s = 0;
                    loop {
                        let b = *payload.get(q)?;
                        q += 1;
                        klen |= ((b & 0x7f) as u64) << s;
                        if b & 0x80 == 0 {
                            break;
                        }
                        s += 7;
                    }
                    let k = payload.get(q..q + klen as usize)?;
                    out.push(String::from_utf8_lossy(k).into_owned());
                    q += klen as usize;
                }
                return Some(out);
            }
        }
        p = end;
    }
    None
}
