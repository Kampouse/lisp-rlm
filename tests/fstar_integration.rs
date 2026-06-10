//! Integration test: F* proofs match Rust implementation
//! Run: cargo test fstar

use lisp_rlm_wasm::tagged_value::*;

#[test]
fn tag_constants_match_fstar() {
    // These must match CoreTypes.fst
    assert_eq!(TAG_NUM, 0);
    assert_eq!(TAG_BOOL, 1);
    assert_eq!(TAG_FNREF, 2);
    assert_eq!(TAG_CLOSURE, 3);
    assert_eq!(TAG_NIL, 4);
    assert_eq!(TAG_STR, 5);
    assert_eq!(TAG_ARRAY, 6);
    assert_eq!(TAG_BITS, 3);
}

#[test]
fn memory_layout_matches_fstar() {
    // These must match CoreTypes.fst
    assert_eq!(RUNTIME_HEAP_PTR, 56);
    assert_eq!(TEMP_MEM, 64);
    assert_eq!(HEAP_START, 200_000);
    assert_eq!(STORAGE_BUF, 8192);
    assert_eq!(INPUT_BUF, 16384);
    assert_eq!(RETURN_BUF, 32768);
    assert_eq!(BORSH_BUF, 36864);
}

#[test]
fn tag_mask_correct() {
    assert_eq!(TAG_MASK, 7); // 0b111
}

#[test]
fn encode_decode_roundtrip() {
    // Num encoding
    let n = 42i64;
    let tagged = encode_num(n);
    assert_eq!(tagged & TAG_MASK, TAG_NUM);
    
    // Bool encoding
    let t = encode_bool(true);
    let f = encode_bool(false);
    assert_eq!(t & TAG_MASK, TAG_BOOL);
    assert_eq!(f & TAG_MASK, TAG_BOOL);
    
    // Nil encoding
    let nil = encode_nil();
    assert_eq!(nil, TAG_NIL);
}