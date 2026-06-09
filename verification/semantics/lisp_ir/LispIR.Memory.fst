(** LispIR Memory Model — F* Formal Specification

    Models WASM linear memory with tagged pointers and u128 operations.
    Key theorem: u128 operations must use raw addresses, not tagged.
*)
module LispIR.Memory

// ============================================================
// MEMORY LAYOUT CONSTANTS (from wasm_emit/mod.rs)
// ============================================================

let runtime_heap_ptr = 56
let amount_mem       = 256
let heap_start       = 200000

// ============================================================
// TAG CONSTANTS (from wasm_emit/mod.rs lines 526-534)
// ============================================================

let tag_num      = 0
let tag_bool     = 1
let tag_str      = 5
let tag_array    = 6
let tag_u128     = 7
let tag_invalid  = 7

// ============================================================
// TAGGED POINTERS
// ============================================================

// A tagged pointer is (payload << 3) | tag
noeq type tagged_ptr = {
  raw: int;      // Actual memory address (untagged)
  tag: int;      // 0-6 valid, 7 invalid/u128
}

// Valid tag check
let tag_valid (p: tagged_ptr) : Tot bool = p.tag <= tag_u128

// Encode: (payload << 3) | tag
// Match pattern from LispIR.Tagged
let encode_tag (v:int) (t:int) : Tot int =
  Prims.op_Multiply v 8 + t

// Decode: value / 8 gives payload
let decode_payload (v:int) : Tot int = v / 8

// Decode: value % 8 gives tag
let decode_tag (v:int) : Tot int = v % 8

// ============================================================
// u128 REPRESENTATION
// ============================================================

// u128 is two i64 values stored at adjacent addresses
noeq type u128 = {
  lo: int;  // Low 64 bits
  hi: int;  // High 64 bits
}

// Size of u128 in memory (16 bytes)
let u128_size = 16

// ============================================================
// BUFFER DISJOINTNESS
// ============================================================

// Heap never overlaps with fixed buffers
let heap_no_overlap = heap_start > 32768 + 16384  // 200000 > 49152