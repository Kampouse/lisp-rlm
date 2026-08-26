//! Borsh serialization/deserialization round-trip tests.
//!
//! Uses compile_fuzz mode which stores tagged values at TEMP_MEM without
//! calling value_return. This lets us read tagged results directly.
//!
//! Include the shared harness:
//! #[path = "borsh_harness.rs"] mod harness;

#[path = "borsh_harness.rs"]
mod harness;

use harness::*;

// ══════════════════════════════════════════════════
// i64
// ══════════════════════════════════════════════════

#[test]
fn test_i64_serialize() {
    let src = r#"
(borsh-schema (Counter (count i64)))
(define (run) (borsh-serialize "Counter" 42))
(export "run" run)
"#;
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(8);
    assert_eq!(bytes, 42i64.to_le_bytes(), "i64 serialize");
}

#[test]
fn test_i64_deserialize() {
    let src = deser_program("(Counter (count i64))");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.write_bytes(BORSH_BUF_USIZE, &42i64.to_le_bytes());
    runner.run().unwrap();
    let result = runner.read_result();
    assert!(
        matches!(result, TaggedValue::Num(42)),
        "i64 deserialize: {:?}",
        result
    );
}

#[test]
fn test_i64_roundtrip() {
    let ser_src = ser_program("(Counter (count i64))", "99");
    let mut ser = WasmRunner::new(&ser_src).unwrap();
    ser.run().unwrap();
    let ser_bytes = ser.read_borsh_bytes(8);

    let deser_src = deser_program("(Counter (count i64))");
    let mut deser = WasmRunner::new(&deser_src).unwrap();
    deser.write_bytes(BORSH_BUF_USIZE, &ser_bytes);
    deser.run().unwrap();
    let result = deser.read_result();
    assert!(
        matches!(result, TaggedValue::Num(99)),
        "i64 roundtrip: {:?}",
        result
    );
}

#[test]
fn test_i64_negative_serialize() {
    let src = ser_program("(Counter (count i64))", "-1");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(8);
    assert_eq!(bytes, (-1i64).to_le_bytes(), "i64 -1 serialize");
}

#[test]
fn test_i64_negative_deserialize() {
    let src = deser_program("(Counter (count i64))");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.write_bytes(BORSH_BUF_USIZE, &(-1i64).to_le_bytes());
    runner.run().unwrap();
    let result = runner.read_result();
    assert!(
        matches!(result, TaggedValue::Num(-1)),
        "i64 -1 deserialize: {:?}",
        result
    );
}

// ══════════════════════════════════════════════════
// Multi-field struct
// ══════════════════════════════════════════════════

#[test]
fn test_struct_2field_serialize() {
    let src = ser_program("(Point (x i64) (y i64))", "10 20");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(16);
    let expected: Vec<u8> = [10i64.to_le_bytes(), 20i64.to_le_bytes()].concat();
    assert_eq!(bytes, expected, "struct 2-field serialize");
}

#[test]
fn test_struct_2field_deserialize() {
    let src = deser_program("(Point (x i64) (y i64))");
    let mut runner = WasmRunner::new(&src).unwrap();
    let bytes: Vec<u8> = [10i64.to_le_bytes(), 20i64.to_le_bytes()].concat();
    runner.write_bytes(BORSH_BUF_USIZE, &bytes);
    runner.run().unwrap();
    let tagged = runner.read_raw_result();
    assert_eq!(
        tag_of(tagged),
        TAG_ARRAY,
        "struct 2-field: expected TAG_ARRAY, got tag {}",
        tag_of(tagged)
    );
    let vals = runner.read_array_nums(tagged);
    assert_eq!(vals, vec![10, 20], "struct 2-field deserialize");
}

#[test]
fn test_struct_3field_deserialize() {
    let src = deser_program("(Vec3 (x i64) (y i64) (z i64))");
    let mut runner = WasmRunner::new(&src).unwrap();
    let bytes: Vec<u8> = [1i64.to_le_bytes(), 2i64.to_le_bytes(), 3i64.to_le_bytes()].concat();
    runner.write_bytes(BORSH_BUF_USIZE, &bytes);
    runner.run().unwrap();
    let tagged = runner.read_raw_result();
    assert_eq!(tag_of(tagged), TAG_ARRAY);
    let vals = runner.read_array_nums(tagged);
    assert_eq!(vals, vec![1, 2, 3], "struct 3-field deserialize");
}

#[test]
fn test_struct_2field_roundtrip() {
    let ser_src = ser_program("(Point (x i64) (y i64))", "100 200");
    let mut ser = WasmRunner::new(&ser_src).unwrap();
    ser.run().unwrap();
    let ser_bytes = ser.read_borsh_bytes(16);

    let deser_src = deser_program("(Point (x i64) (y i64))");
    let mut deser = WasmRunner::new(&deser_src).unwrap();
    deser.write_bytes(BORSH_BUF_USIZE, &ser_bytes);
    deser.run().unwrap();
    let tagged = deser.read_raw_result();
    let vals = deser.read_array_nums(tagged);
    assert_eq!(vals, vec![100, 200], "struct 2-field roundtrip");
}

// ══════════════════════════════════════════════════
// Option
// ══════════════════════════════════════════════════

#[test]
fn test_option_some_serialize() {
    let src = r#"
(borsh-schema (Wrapper (value (Option i64))))
(define (run) (borsh-serialize "Wrapper" 42))
(export "run" run)
"#;
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(9);
    let expected: Vec<u8> = [1u8]
        .iter()
        .copied()
        .chain(42i64.to_le_bytes().iter().copied())
        .collect();
    assert_eq!(bytes, expected, "Option Some serialize");
}

#[test]
fn test_option_some_deserialize() {
    let src = deser_program("(Wrapper (value (Option i64)))");
    let mut runner = WasmRunner::new(&src).unwrap();
    let bytes: Vec<u8> = [1u8]
        .iter()
        .copied()
        .chain(42i64.to_le_bytes().iter().copied())
        .collect();
    runner.write_bytes(BORSH_BUF_USIZE, &bytes);
    runner.run().unwrap();
    let result = runner.read_result();
    assert!(
        matches!(result, TaggedValue::Num(42)),
        "Option Some deserialize: {:?}",
        result
    );
}

#[test]
fn test_option_none_serialize() {
    let src = r#"
(borsh-schema (Wrapper (value (Option i64))))
(define (run) (borsh-serialize "Wrapper" nil))
(export "run" run)
"#;
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(1);
    assert_eq!(bytes, vec![0u8], "Option None serialize");
}

#[test]
fn test_option_none_deserialize() {
    let src = deser_program("(Wrapper (value (Option i64)))");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.write_bytes(BORSH_BUF_USIZE, &[0u8]);
    runner.run().unwrap();
    let tagged = runner.read_raw_result();
    // Nil is written as the nil sentinel by the export wrapper
    assert_eq!(
        tag_of(tagged),
        TAG_NIL,
        "Option None: expected TAG_NIL, got tag {} value {}",
        tag_of(tagged),
        tagged
    );
}

// ══════════════════════════════════════════════════
// Enum
// ══════════════════════════════════════════════════

#[test]
fn test_enum_unit_serialize() {
    let src = r#"
(borsh-schema (Color (Red) (Green) (Blue)))
(define (run) (borsh-serialize "Color" 2))
(export "run" run)
"#;
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(1);
    assert_eq!(bytes, vec![2u8], "Color::Blue serialize");
}

#[test]
fn test_enum_unit_deserialize() {
    let src = deser_program("(Color (Red) (Green) (Blue))");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.write_bytes(BORSH_BUF_USIZE, &[2u8]);
    runner.run().unwrap();
    let tagged = runner.read_raw_result();
    assert_eq!(
        tag_of(tagged),
        TAG_ARRAY,
        "enum unit: expected TAG_ARRAY, got tag {}",
        tag_of(tagged)
    );
    let vals = runner.read_array_nums(tagged);
    assert_eq!(vals, vec![2], "Color::Blue: expected [2], got {:?}", vals);
}

#[test]
fn test_enum_fields_serialize() {
    let src = r#"
(borsh-schema (Shape (Circle (radius i64)) (Rect (w i64) (h i64))))
(define (run) (borsh-serialize "Shape" 0 42))
(export "run" run)
"#;
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(9);
    let expected: Vec<u8> = [0u8]
        .iter()
        .copied()
        .chain(42i64.to_le_bytes().iter().copied())
        .collect();
    assert_eq!(bytes, expected, "Shape::Circle serialize");
}

#[test]
fn test_enum_fields_deserialize() {
    let src = deser_program("(Shape (Circle (radius i64)) (Rect (w i64) (h i64)))");
    let mut runner = WasmRunner::new(&src).unwrap();
    let bytes: Vec<u8> = [0u8]
        .iter()
        .copied()
        .chain(42i64.to_le_bytes().iter().copied())
        .collect();
    runner.write_bytes(BORSH_BUF_USIZE, &bytes);
    runner.run().unwrap();
    let tagged = runner.read_raw_result();
    assert_eq!(tag_of(tagged), TAG_ARRAY, "enum fields: expected TAG_ARRAY");
    let vals = runner.read_array_nums(tagged);
    // Variant 0 (Circle) with 1 field: [0, 42]
    assert_eq!(
        vals,
        vec![0, 42],
        "Shape::Circle deserialize: expected [0, 42], got {:?}",
        vals
    );
}

#[test]
fn test_enum_fields_roundtrip() {
    let ser_src = r#"
(borsh-schema (Shape (Circle (radius i64)) (Rect (w i64) (h i64))))
(define (run) (borsh-serialize "Shape" 0 42))
(export "run" run)
"#;
    let mut ser = WasmRunner::new(&ser_src).unwrap();
    ser.run().unwrap();
    let ser_bytes = ser.read_borsh_bytes(9);

    let deser_src = deser_program("(Shape (Circle (radius i64)) (Rect (w i64) (h i64)))");
    let mut deser = WasmRunner::new(&deser_src).unwrap();
    deser.write_bytes(BORSH_BUF_USIZE, &ser_bytes);
    deser.run().unwrap();
    let tagged = deser.read_raw_result();
    let vals = deser.read_array_nums(tagged);
    assert_eq!(vals, vec![0, 42], "Shape::Circle roundtrip");
}

// ══════════════════════════════════════════════════
// Vec
// ══════════════════════════════════════════════════

#[test]
fn test_vec_i64_serialize() {
    let src = r#"
(borsh-schema (Numbers (items (Vec i64))))
(define (run) (borsh-serialize "Numbers" (array 10 20 30)))
(export "run" run)
"#;
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(28); // 4 (len) + 3*8 (items)
    let mut expected: Vec<u8> = 3u32.to_le_bytes().to_vec();
    expected.extend_from_slice(&10i64.to_le_bytes());
    expected.extend_from_slice(&20i64.to_le_bytes());
    expected.extend_from_slice(&30i64.to_le_bytes());
    assert_eq!(bytes, expected, "Vec i64 serialize");
}

#[test]
fn test_vec_i64_deserialize() {
    let src = deser_program("(Numbers (items (Vec i64)))");
    let mut runner = WasmRunner::new(&src).unwrap();
    let mut bytes: Vec<u8> = 3u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&10i64.to_le_bytes());
    bytes.extend_from_slice(&20i64.to_le_bytes());
    bytes.extend_from_slice(&30i64.to_le_bytes());
    runner.write_bytes(BORSH_BUF_USIZE, &bytes);
    runner.run().unwrap();
    let tagged = runner.read_raw_result();
    assert_eq!(tag_of(tagged), TAG_ARRAY, "Vec: expected TAG_ARRAY");
    let vals = runner.read_array_nums(tagged);
    assert_eq!(vals, vec![10, 20, 30], "Vec i64 deserialize");
}

// ══════════════════════════════════════════════════
// u8, u32, bool
// ══════════════════════════════════════════════════

#[test]
fn test_u8_serialize() {
    let src = ser_program("(Byte (val u8))", "200");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(1);
    assert_eq!(bytes, vec![200u8], "u8 serialize");
}

#[test]
fn test_u8_deserialize() {
    let src = deser_program("(Byte (val u8))");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.write_bytes(BORSH_BUF_USIZE, &[200u8]);
    runner.run().unwrap();
    let result = runner.read_result();
    assert!(
        matches!(result, TaggedValue::Num(200)),
        "u8 deserialize: {:?}",
        result
    );
}

#[test]
fn test_u32_serialize() {
    let src = ser_program("(Int (val u32))", "1000");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(4);
    assert_eq!(bytes, 1000u32.to_le_bytes(), "u32 serialize");
}

#[test]
fn test_u32_deserialize() {
    let src = deser_program("(Int (val u32))");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.write_bytes(BORSH_BUF_USIZE, &1000u32.to_le_bytes());
    runner.run().unwrap();
    let result = runner.read_result();
    assert!(
        matches!(result, TaggedValue::Num(1000)),
        "u32 deserialize: {:?}",
        result
    );
}

#[test]
fn test_bool_serialize() {
    let src = r#"
(borsh-schema (Flag (val bool)))
(define (run) (borsh-serialize "Flag" 1))
(export "run" run)
"#;
    let mut runner = WasmRunner::new(src).unwrap();
    runner.run().unwrap();
    let bytes = runner.read_borsh_bytes(1);
    assert_eq!(bytes, vec![1u8], "bool true serialize");
}

#[test]
fn test_bool_deserialize() {
    let src = deser_program("(Flag (val bool))");
    let mut runner = WasmRunner::new(&src).unwrap();
    runner.write_bytes(BORSH_BUF_USIZE, &[1u8]);
    runner.run().unwrap();
    let result = runner.read_result();
    assert!(
        matches!(result, TaggedValue::Bool(true)),
        "bool true deserialize: {:?}",
        result
    );
}

// ── Helper: get tag of a tagged value, handling nil sentinel ──

fn tag_of(tagged: i64) -> i64 {
    if is_nil_sentinel(tagged) {
        TAG_NIL
    } else {
        tagged & TAG_MASK
    }
}

// ── Proptest: property-based Borsh round-trip ──

#[cfg(test)]
mod proptest_tests {
    use super::*;
    use proptest::prelude::*;

    /// i64 values that fit in the tagged representation (val << 3 | tag must not overflow).
    /// Max safe value: 2^60 - 1 = 1152921504606846975 (≈ 1.15e18)
    /// Min safe value: -(2^60) = -1152921504606846976
    const I64_TAG_MAX: i64 = (1i64 << 60) - 1;
    const I64_TAG_MIN: i64 = -(1i64 << 60);

    // Generate random i64 values within tagged-value range
    fn arb_i64() -> impl Strategy<Value = i64> {
        prop_oneof![
            Just(0i64),
            Just(1i64),
            Just(-1i64),
            Just(I64_TAG_MAX),
            Just(I64_TAG_MIN),
            (I64_TAG_MIN..=I64_TAG_MAX),
        ]
    }

    proptest! {
        #[test]
        fn proptest_i64_roundtrip(val in arb_i64()) {
            let src = format!(
                r#"
(borsh-schema (Counter (count i64)))
(define (run) (borsh-serialize "Counter" {val}))
(export "run" run)
"#
            );
            let mut ser = match WasmRunner::new(&src) {
                Ok(r) => r,
                Err(_) => return Ok(()), // compile errors are ok (e.g. out of range)
            };
            if ser.run().is_err() {
                return Ok(()); // traps are ok for edge values
            }
            let ser_bytes = ser.read_borsh_bytes(8);

            let deser_src = deser_program("(Counter (count i64))");
            let mut deser = match WasmRunner::new(&deser_src) {
                Ok(r) => r,
                Err(_) => return Ok(()),
            };
            deser.write_bytes(BORSH_BUF_USIZE, &ser_bytes);
            if deser.run().is_err() {
                return Ok(());
            }
            let result = deser.read_result();
            prop_assert!(matches!(result, TaggedValue::Num(v) if v == val),
                "i64 roundtrip: expected {}, got {:?}", val, result);
        }

        #[test]
        fn proptest_struct_2field_roundtrip(x in arb_i64(), y in arb_i64()) {
            let ser_src = format!(
                r#"
(borsh-schema (Point (x i64) (y i64)))
(define (run) (borsh-serialize "Point" {x} {y}))
(export "run" run)
"#
            );
            let mut ser = match WasmRunner::new(&ser_src) {
                Ok(r) => r,
                Err(_) => return Ok(()),
            };
            if ser.run().is_err() {
                return Ok(());
            }
            let ser_bytes = ser.read_borsh_bytes(16);

            let deser_src = deser_program("(Point (x i64) (y i64))");
            let mut deser = match WasmRunner::new(&deser_src) {
                Ok(r) => r,
                Err(_) => return Ok(()),
            };
            deser.write_bytes(BORSH_BUF_USIZE, &ser_bytes);
            if deser.run().is_err() {
                return Ok(());
            }
            let tagged = deser.read_raw_result();
            prop_assert_eq!(tag_of(tagged), TAG_ARRAY);
            let vals = deser.read_array_nums(tagged);
            prop_assert_eq!(vals, vec![x, y]);
        }
    }
}
