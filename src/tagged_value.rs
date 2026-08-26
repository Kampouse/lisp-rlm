//! Tagged value runtime contract for WASM memory.
//!
//! Defines the wire format shared between the WASM emitter and any host-side
//! (or test-side) code that needs to interpret tagged i64 values stored in
//! linear memory. The format is `(payload << 3) | tag` where the bottom 3 bits
//! discriminate the value type.

// ── Tag constants ──

pub const TAG_NUM: i64 = 0;
pub const TAG_BOOL: i64 = 1;
pub const TAG_FNREF: i64 = 2;
pub const TAG_CLOSURE: i64 = 3;
pub const TAG_NIL: i64 = 4;
pub const TAG_STR: i64 = 5;
pub const TAG_ARRAY: i64 = 6;

pub const TAG_BITS: i64 = 3;
pub const TAG_MASK: i64 = (1 << TAG_BITS) - 1; // 0b111 = 7

// ── Memory layout constants ──

pub const TEMP_MEM: i64 = 64;
pub const RUNTIME_HEAP_PTR: i64 = 56; // 8-byte slot holding bump-allocator pointer
pub const HEAP_START: i64 = 200_000;
pub const STORAGE_BUF: i64 = 8192;
pub const STORAGE_U128_BUF: i64 = 8208;
pub const INPUT_BUF: i64 = 16384;
pub const RETURN_BUF: i64 = 32768;
pub const BORSH_BUF: i64 = 36864;

/// Special sentinel written by the export wrapper when the result is nil.
/// Uses `0xFE`/`0xFF` byte patterns that cannot appear in valid untagged i64
/// values (which always have the lower 3 bits as a valid tag 0-6).
pub const NIL_SENTINEL: i64 = 0x7FFE_FEFF_FEFF_FEFE_i64;

// ── Tag enum ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Num,
    Bool,
    FnRef,
    Closure,
    Nil,
    Str,
    Array,
}

impl Tag {
    pub fn from_i64(tag: i64) -> Option<Self> {
        match tag {
            TAG_NUM => Some(Tag::Num),
            TAG_BOOL => Some(Tag::Bool),
            TAG_FNREF => Some(Tag::FnRef),
            TAG_CLOSURE => Some(Tag::Closure),
            TAG_NIL => Some(Tag::Nil),
            TAG_STR => Some(Tag::Str),
            TAG_ARRAY => Some(Tag::Array),
            _ => None,
        }
    }

    pub fn as_i64(self) -> i64 {
        match self {
            Tag::Num => TAG_NUM,
            Tag::Bool => TAG_BOOL,
            Tag::FnRef => TAG_FNREF,
            Tag::Closure => TAG_CLOSURE,
            Tag::Nil => TAG_NIL,
            Tag::Str => TAG_STR,
            Tag::Array => TAG_ARRAY,
        }
    }
}

// ── TaggedValue enum ──

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaggedValue {
    /// Integer value (61-bit signed, payload stored directly).
    Num(i64),
    /// Boolean value (payload = 0 for false, 1 for true).
    Bool(bool),
    /// Nil sentinel.
    Nil,
    /// String: pointer into WASM memory and byte length.
    /// Encoded in WASM as `(ptr | (len << 32)) << TAG_BITS | TAG_STR`.
    Str { ptr: i64, len: i64 },
    /// Array: pointer to runtime array layout in WASM memory and element count.
    /// Runtime layout at `ptr`: `[count: i64, elem0_tagged: i64, elem1_tagged: i64, ...]`
    Array { ptr: i64, count: i64 },
    /// Function reference (local function index).
    FnRef(u32),
    /// Closure (heap pointer to closure environment).
    Closure(i64),
}

// ── Decode ──

/// Decode a tagged i64 value read from WASM memory.
///
/// For `Str` and `Array` variants, the payload is a byte offset into `memory`;
/// we read additional data from there. For all other variants the full
/// information is embedded in the tagged word.
pub fn decode(memory: &[u8], tagged: i64) -> TaggedValue {
    let tag_val = tagged & TAG_MASK;
    let payload = tagged >> TAG_BITS; // arithmetic shift preserves sign for Num

    match Tag::from_i64(tag_val) {
        Some(Tag::Num) => TaggedValue::Num(payload),
        Some(Tag::Bool) => TaggedValue::Bool(payload != 0),
        Some(Tag::Nil) => TaggedValue::Nil,
        Some(Tag::FnRef) => TaggedValue::FnRef(payload as u32),
        Some(Tag::Closure) => TaggedValue::Closure(payload),
        Some(Tag::Str) => {
            // payload = heap_off | (len << 32)
            let ptr = payload & 0xFFFF_FFFF; // lower 32 bits = offset
            let len = (payload >> 32) & 0xFFFF_FFFF; // upper 32 bits = length
            TaggedValue::Str { ptr, len }
        }
        Some(Tag::Array) => {
            // payload is the byte offset in WASM memory where the array lives.
            // Layout: [count: i64, elem0_tagged: i64, elem1_tagged: i64, ...]
            let ptr = payload;
            let count = read_i64_le(memory, ptr as usize);
            TaggedValue::Array { ptr, count }
        }
        None => {
            // Unknown tag — treat as raw Num as a fallback (shouldn't happen
            // with well-formed WASM output).
            TaggedValue::Num(payload)
        }
    }
}

// ── Encode ──

/// Encode a numeric value: `(val << TAG_BITS) | TAG_NUM`.
pub fn encode_num(val: i64) -> i64 {
    (val << TAG_BITS) | TAG_NUM
}

/// Encode a boolean value: `(0 or 1 << TAG_BITS) | TAG_BOOL`.
pub fn encode_bool(val: bool) -> i64 {
    let payload = if val { 1i64 } else { 0 };
    (payload << TAG_BITS) | TAG_BOOL
}

/// Encode nil as the TAG_NIL constant (tag only, no payload).
pub fn encode_nil() -> i64 {
    TAG_NIL
}

/// Check whether a tagged i64 is the special nil sentinel written by the
/// export wrapper (as opposed to a normal tagged-nil value).
pub fn is_nil_sentinel(val: i64) -> bool {
    val == NIL_SENTINEL
}

// ── Helpers ──

fn read_i64_le(mem: &[u8], offset: usize) -> i64 {
    let end = offset + 8;
    if end > mem.len() {
        return 0; // out of bounds — caller should check memory size
    }
    let bytes: [u8; 8] = mem[offset..end].try_into().unwrap();
    i64::from_le_bytes(bytes)
}

#[allow(dead_code)]
fn write_i64_le(mem: &mut [u8], offset: usize, val: i64) {
    let end = offset + 8;
    if end <= mem.len() {
        mem[offset..end].copy_from_slice(&val.to_le_bytes());
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_roundtrip() {
        for &tag_val in &[
            TAG_NUM,
            TAG_BOOL,
            TAG_FNREF,
            TAG_CLOSURE,
            TAG_NIL,
            TAG_STR,
            TAG_ARRAY,
        ] {
            let tag = Tag::from_i64(tag_val).unwrap_or_else(|| panic!("unknown tag {}", tag_val));
            assert_eq!(tag.as_i64(), tag_val);
        }
    }

    #[test]
    fn tag_from_invalid_returns_none() {
        assert!(Tag::from_i64(7).is_none());
        assert!(Tag::from_i64(-1).is_none());
        assert!(Tag::from_i64(99).is_none());
    }

    #[test]
    fn encode_decode_num() {
        for &n in &[0i64, 1, -1, 42, i64::MAX >> 3, i64::MIN >> 3] {
            let tagged = encode_num(n);
            // Tag bits must be 0
            assert_eq!(tagged & TAG_MASK, TAG_NUM);
            let decoded = decode(&[], tagged);
            assert_eq!(decoded, TaggedValue::Num(n));
        }
    }

    #[test]
    fn encode_decode_bool() {
        let t = encode_bool(true);
        let f = encode_bool(false);
        assert_eq!(t & TAG_MASK, TAG_BOOL);
        assert_eq!(f & TAG_MASK, TAG_BOOL);
        assert_eq!(decode(&[], t), TaggedValue::Bool(true));
        assert_eq!(decode(&[], f), TaggedValue::Bool(false));
    }

    #[test]
    fn encode_nil_constant() {
        let nil = encode_nil();
        assert_eq!(nil, TAG_NIL);
        assert_eq!(nil & TAG_MASK, TAG_NIL);
        match decode(&[], nil) {
            TaggedValue::Nil => {}
            other => panic!("expected Nil, got {:?}", other),
        }
    }

    #[test]
    fn nil_sentinel_detection() {
        assert!(is_nil_sentinel(NIL_SENTINEL));
        // Different from actual tagged nil
        assert!(!is_nil_sentinel(TAG_NIL));
        assert!(!is_nil_sentinel(0));
    }

    #[test]
    fn encode_decode_fnref() {
        for &idx in &[0u32, 1, 42, 255] {
            let tagged = (idx as i64) << TAG_BITS | TAG_FNREF;
            match decode(&[], tagged) {
                TaggedValue::FnRef(i) => assert_eq!(i, idx),
                other => panic!("expected FnRef({}), got {:?}", idx, other),
            }
        }
    }

    #[test]
    fn encode_decode_closure() {
        let heap_ptr: i64 = 4096;
        let tagged = (heap_ptr << TAG_BITS) | TAG_CLOSURE;
        match decode(&[], tagged) {
            TaggedValue::Closure(p) => assert_eq!(p, heap_ptr),
            other => panic!("expected Closure({}), got {:?}", heap_ptr, other),
        }
    }

    #[test]
    fn decode_str_tagged() {
        // payload = ptr | (len << 32), where ptr=0x1000, len=5
        let ptr: i64 = 0x1000;
        let len: i64 = 5;
        let payload = ptr | (len << 32);
        let tagged = (payload << TAG_BITS) | TAG_STR;
        match decode(&[], tagged) {
            TaggedValue::Str { ptr: p, len: l } => {
                assert_eq!(p, ptr);
                assert_eq!(l, len);
            }
            other => panic!("expected Str, got {:?}", other),
        }
    }

    #[test]
    fn decode_array_from_memory() {
        // Build a fake WASM memory with: [count=2, elem0=encode_num(10), elem1=encode_bool(true)]
        let count: i64 = 2;
        let e0 = encode_num(10);
        let e1 = encode_bool(true);
        let mut mem = vec![0u8; 4096 + 3 * 8]; // offset 4096, 3 i64s
        let base = 4096usize;
        write_i64_le(&mut mem, base, count);
        write_i64_le(&mut mem, base + 8, e0);
        write_i64_le(&mut mem, base + 16, e1);

        let tagged = (4096i64 << TAG_BITS) | TAG_ARRAY;
        match decode(&mem, tagged) {
            TaggedValue::Array { ptr, count: c } => {
                assert_eq!(ptr, 4096);
                assert_eq!(c, 2);
            }
            other => panic!("expected Array, got {:?}", other),
        }

        // Decode elements
        let decoded_elem0 = decode(&mem, read_i64_le(&mem, base + 8));
        let decoded_elem1 = decode(&mem, read_i64_le(&mem, base + 16));
        assert_eq!(decoded_elem0, TaggedValue::Num(10));
        assert_eq!(decoded_elem1, TaggedValue::Bool(true));
    }

    #[test]
    fn memory_layout_constants_sanity() {
        // Buffers should not overlap — check relative ordering in memory
        assert!(RUNTIME_HEAP_PTR < TEMP_MEM);
        assert!(TEMP_MEM < STORAGE_BUF);
        assert!(STORAGE_BUF < INPUT_BUF);
        assert!(INPUT_BUF < RETURN_BUF);
        assert!(RETURN_BUF < BORSH_BUF);
        // HEAP_START must be above all fixed buffers and data segments
        assert!(BORSH_BUF < HEAP_START);
    }

    #[test]
    fn nil_sentinel_differs_from_tagged_nil() {
        // The nil sentinel is a specific constant written by the export wrapper.
        // It must not equal the standard tagged-nil word (TAG_NIL = 4).
        assert_ne!(NIL_SENTINEL, TAG_NIL);
    }
}
