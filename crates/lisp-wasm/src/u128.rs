use crate::emit::WasmEmitter;

impl WasmEmitter {
    pub(crate) fn parse_u128(s: &str) -> Result<(i64, i64), String> {
        let mut lo: u64 = 0;
        let mut hi: u64 = 0;
        for ch in s.chars() {
            if ch == '_' { continue; }
            if ch < '0' || ch > '9' { return Err(format!("invalid digit in u128 literal: '{}'", ch)); }
            let digit = ch as u64 - '0' as u64;
            let old_hi = hi as u128;
            let old_lo = lo as u128;
            let new_val = old_hi * (1u128 << 64) + old_lo;
            let new_val = new_val * 10 + digit as u128;
            lo = new_val as u64;
            hi = (new_val >> 64) as u64;
        }
        Ok((lo as i64, hi as i64))
    }
}
