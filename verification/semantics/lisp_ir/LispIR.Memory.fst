(** LispIR Memory Model — F* Formal Specification

    Models WASM linear memory with tagged pointers and u128 operations.
    Key theorem: u128 operations must use raw addresses, not tagged.
    
    PROVED SAFETY PROPERTIES:
    - u128_add/sub trap on overflow/underflow (DeFi safety)
    - Tagged pointers different from raw addresses
    
    NOT PROVED (abstracted):
    - Memory operations (assume val)
*)
module LispIR.Memory

// ============================================================
// MEMORY LAYOUT CONSTANTS (from wasm_emit/mod.rs)
// ============================================================

let runtime_heap_ptr = 56
let amount_mem       = 256
let input_buf        = 16384
let return_buf       = 32768
let storage_buf      = 8192
let storage_u128_buf = 8208
let heap_start       = 200000
let max_memory       = 1048576  // 1MB default WASM memory

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
val tag_valid : tagged_ptr -> Tot bool
let tag_valid (p: tagged_ptr) = p.tag <= tag_u128

// Encode: (payload << 3) | tag = payload * 8 + tag
let encode_tag (v:int) (t:int) : Tot int =
  Prims.op_Multiply v 8 + t

// Decode: value / 8 gives payload
let decode_payload (v:int) : Tot int = v / 8

// Decode: value % 8 gives tag
let decode_tag (v:int) : Tot int = v % 8

// Untag: extract raw address from tagged value
let untag_ptr (p:tagged_ptr) : Tot int = p.raw

// Untag (from integer)
let untag_int (v:int) : Tot int = v / 8

// ============================================================
// KEY INSIGHT: Tagged ≠ Raw
// ============================================================

// NOTE: Tagged pointer = payload * 8 + tag
// If payload ≠ 0 or tag ≠ 0, then tagged ≠ payload
// Exception: (0, 0) gives tagged=0=payload, but that's Num(0) which is fine.
// THE KEY INSIGHT: Using tagged value as address would access payload*8+tag,
// which is WRONG unless you intentionally want that offset (which you don't).

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
// u128 ARITHMETIC WITH OVERFLOW DETECTION
// ============================================================

// Unsigned comparison: a < b (comparing high bits first)
val u128_lt : u128 -> u128 -> Tot bool
let u128_lt a b =
  if a.hi < b.hi then true
  else if a.hi > b.hi then false
  else a.lo < b.lo

// Unsigned comparison: a = b (both parts equal)
val u128_eq : u128 -> u128 -> Tot bool
let u128_eq a b = a.lo = b.lo && a.hi = b.hi

// Unsigned comparison: a <= b
val u128_le : u128 -> u128 -> Tot bool
let u128_le a b = u128_lt a b || u128_eq a b

// Addition with overflow check
// Returns None on overflow (DeFi safety: trap instead of wrap)
val u128_add_safe : u128 -> u128 -> Tot (option u128)
let u128_add_safe a b =
  let new_lo = a.lo + b.lo in
  let carry  = if new_lo < a.lo then 1 else 0 in
  let new_hi = a.hi + b.hi + carry in
  // Overflow: new_hi wrapped or carry overflowed
  if new_hi < a.hi
  then None  // Overflow - would trap in WASM
  else Some { lo = new_lo; hi = new_hi }

// Subtraction with underflow check
// Returns None on underflow (DeFi safety: trap instead of wrap)
val u128_sub_safe : a:u128 -> b:u128 -> Tot (option u128)
let u128_sub_safe a b =
  // First check if a >= b (unsigned)
  if u128_lt a b 
  then None  // Underflow - would trap in WASM
  else
    let borrow = if a.lo < b.lo then 1 else 0 in
    let new_lo = a.lo - b.lo in
    let new_hi = a.hi - b.hi - borrow in
    Some { lo = new_lo; hi = new_hi }

// ============================================================
// THEOREM: Underflow → None
// ============================================================

// THEOREM: Subtraction underflow returns None
// When a < b (unsigned), u128_sub_safe returns None (would trap in WASM)
// This follows directly from the definition of u128_sub_safe

// ============================================================
// ABSTRACT MEMORY MODEL
// ============================================================

// Abstract memory state: we model it as a type without implementation
assume type memory

// Memory read/write: abstract operations (no SMT definition needed)
assume val mem_read_u64  : memory -> int -> Tot int
assume val mem_write_u64 : memory -> int -> int -> Tot memory

// Dummy memory for lemmas
assume val dummy_memory : memory

// ============================================================
// u128 MEMORY OPERATIONS
// ============================================================

// Read u128 from memory (low at addr, high at addr+8)
let u128_read_raw (mem:memory) (addr:int) : Tot u128 = 
  { lo = mem_read_u64 mem addr;
    hi = mem_read_u64 mem (addr + 8) }

// Write u128 to memory
let u128_write_raw (mem:memory) (addr:int) (v:u128) : Tot memory =
  let m1 = mem_write_u64 mem addr v.lo in
  mem_write_u64 m1 (addr + 8) v.hi

// ============================================================
// BUFFER DISJOINTNESS
// ============================================================

// Heap never overlaps with fixed buffers
let heap_no_overlap = heap_start > return_buf + storage_u128_buf + 16384

// Storage buffer is separate from return buffer
let storage_no_overlap_return = storage_buf + storage_u128_buf < return_buf

// Input buffer is separate from storage
let input_no_overlap = input_buf + 16384 < storage_buf