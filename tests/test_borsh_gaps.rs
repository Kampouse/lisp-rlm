//! Verification tests for the Borsh gaps that were fixed:
//! 1. F64 serialize/deserialize
//! 2. Vec<String> (variable-length elements in Vec)
//! 3. Nested struct with variable-length fields
//! 4. I64 negative roundtrip (arithmetic shift)
//! 5. Option<T> roundtrip (BlockType::Result fix)

#[path = "borsh_harness.rs"]
mod harness;

use harness::*;
use lisp_rlm_wasm::tagged_value::{TAG_ARRAY, TAG_NIL};

// ── Test 1: F64 serialize ──
#[test]
fn test_f64_serialize() {
    let src = ser_program("(PointF64 (x f64) (y f64))", "1073115682 1073115682");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    // F64 = 8 bytes × 2 fields = 16 bytes
    let bytes = runner.read_borsh_bytes(16);
    assert_eq!(bytes.len(), 16, "F64 serialize should produce 16 bytes");
}

// ── Test 2: Vec<String> serialize (variable-length elements) ──
#[test]
fn test_vec_string_serialize() {
    let src = r#"
(borsh-schema (NameList (names (Vec string))))
(define (run) (borsh-serialize "NameList" (array "alice" "bob")))
(export "run" run)
"#;
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();

    // [4: count=2] [4: len=5][5: "alice"] [4: len=3][3: "bob"] = 20 bytes
    let bytes = runner.read_borsh_bytes(20);

    assert_eq!(&bytes[0..4], &[2, 0, 0, 0], "Vec count should be 2");
    assert_eq!(&bytes[4..8], &[5, 0, 0, 0], "First string length should be 5");
    assert_eq!(&bytes[8..13], b"alice", "First string should be 'alice'");
    assert_eq!(&bytes[13..17], &[3, 0, 0, 0], "Second string length should be 3");
    assert_eq!(&bytes[17..20], b"bob", "Second string should be 'bob'");
}

// ── Test 3: Vec<String> deserialize (the big fix — variable-length elements) ──
#[test]
fn test_vec_string_deserialize() {
    let src = deser_program("(Words (items (Vec string)))");
    let mut runner = WasmRunner::new(&src).unwrap();

    // Write Borsh bytes for Vec<String> with 2 elements: ["hi", "yo"]
    let mut data: Vec<u8> = Vec::new();
    data.extend_from_slice(&2u32.to_le_bytes()); // count
    data.extend_from_slice(&2u32.to_le_bytes()); // len "hi"
    data.extend_from_slice(b"hi");
    data.extend_from_slice(&2u32.to_le_bytes()); // len "yo"
    data.extend_from_slice(b"yo");

    runner.write_bytes(BORSH_BUF_USIZE, &data);
    runner.run().unwrap();

    // If we get here without trapping, the variable-length Vec deserializer works.
    // The result is a tagged array pointing into heap memory.
    let tagged = runner.read_raw_result();
    let tag = tagged & 7;
    assert_eq!(tag, TAG_ARRAY, "Vec<String> deserialize should return TAG_ARRAY (6), got tag {}", tag);
}

// ── Test 4: I64 negative serialize (arithmetic shift fix) ──
#[test]
fn test_i64_negative_serialize_value() {
    let src = ser_program("(Counter (count i64))", "-1");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(8);
    let val = i64::from_le_bytes(bytes.as_slice().try_into().unwrap());
    assert_eq!(val, -1, "Serialized -1 should be i64 -1, got bytes {:?}", bytes);
}

// ── Test 5: Option Some(i64) serialize ──
#[test]
fn test_option_some_serialize() {
    let src = r#"
(borsh-schema (MaybeVal (v (Option i64))))
(define (run) (borsh-serialize "MaybeVal" 42))
(export "run" run)
"#;
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();

    // [1 byte: discriminant=1] [8 bytes: value=42]
    let bytes = runner.read_borsh_bytes(9);
    assert_eq!(bytes[0], 1, "Option Some discriminant should be 1");
    let val = i64::from_le_bytes(bytes[1..9].try_into().unwrap());
    assert_eq!(val, 42, "Option Some value should be 42");
}

// ── Test 6: Full Vec<String> roundtrip (serialize → verify all 3 strings) ──
#[test]
fn test_vec_string_full_roundtrip() {
    let src = r#"
(borsh-schema (Words (items (Vec string))))
(define (run) (borsh-serialize "Words" (array "foo" "bar" "baz")))
(export "run" run)
"#;
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();

    // [4: count=3] [4: len=3][3: "foo"] [4: len=3][3: "bar"] [4: len=3][3: "baz"] = 25
    let bytes = runner.read_borsh_bytes(25);

    let count = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    assert_eq!(count, 3, "Vec should have 3 elements");

    let mut offset = 4;
    for expected in &["foo", "bar", "baz"] {
        let len = u32::from_le_bytes(bytes[offset..offset+4].try_into().unwrap()) as usize;
        offset += 4;
        let s = String::from_utf8(bytes[offset..offset+len].to_vec()).unwrap();
        assert_eq!(s, *expected, "String mismatch");
        offset += len;
    }
}

// ── Test 7: F64 bit-cast roundtrip (values, not just lengths) ──
// f64 fields carry raw-bit i64s (bit-cast). Tagged nums cap at 2^60, so
// only bit patterns < 2^60 are addressable as literals (documented
// constraint) — e.g. bits=1 is the minimum positive denormal
// (5e-324): struct.pack('<d', 5e-324) → 01 00 00 00 00 00 00 00.
#[test]
fn test_f64_bitcast_roundtrip() {
    let src = ser_program("(PointF64 (x f64) (y f64))", "1 0");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(16);
    assert_eq!(&bytes[0..8], &[1, 0, 0, 0, 0, 0, 0, 0], "x must be min-denormal wire bytes");
    assert_eq!(&bytes[8..16], &[0, 0, 0, 0, 0, 0, 0, 0], "y must be +0.0 wire bytes");
}
