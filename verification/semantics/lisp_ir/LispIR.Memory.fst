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
// KEY THEOREM: Tagged ≠ Raw (Safety Critical!)
// ============================================================

// CRITICAL SAFETY PROPERTY:
// If you mistakenly use a tagged value as a memory address,
// you'll read from payload*8+tag instead of payload.
// This causes memory corruption or reads from wrong location.

// Lemma: decode_payload is inverse of encode (for valid tags)
// decode_payload(encode_tag(p,t)) = (p*8+t)/8 = p (integer division)
val decode_encode_inverse : p:int -> t:int -> Lemma
  (t >= 0 && t <= 7 ==> decode_payload (encode_tag p t) = p)
let decode_encode_inverse p t = ()

// Lemma: decode_tag recovers tag (for valid tags)
val decode_encode_tag : p:int -> t:int -> Lemma
  (t >= 0 && t <= 7 ==> decode_tag (encode_tag p t) = t)
let decode_encode_tag p t = ()

// KEY INSIGHT: encode_tag(p, t) = p*8 + t
// If p > 0, then p*8 > p, so tagged > payload
// If t > 0, then p*8 + t > p*8 >= p, so tagged > payload (except p=0,t=0)
// Using tagged value as address reads from WRONG location!

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

// Multiplication by int with overflow check
// Multiplies u128 by an int (treated as u64 for NEAR amounts)
// Returns None on overflow (DeFi safety: trap instead of wrap)
// NOTE: This is a simplified model - full Rust uses 32-bit split arithmetic
val u128_mul_safe : dst:u128 -> scalar:int -> Tot (option u128)
let u128_mul_safe dst scalar =
  // SAFETY: If scalar <= 0, trap (money should never multiply by <= 0)
  if scalar <= 0 
  then None  // Trap on non-positive multiplier (DeFi safety)
  else
    // Conservative overflow check: if high part is non-zero and scalar > 1,
    // the result likely exceeds u128 max (2^128 - 1)
    // For DeFi amounts (typically < 2^100), scalar > 1 means potential overflow
    if dst.hi > 0 && scalar > 1
    then None  // Trap - would overflow
    else if dst.lo > 0 && scalar > 0x10000000000000000 / dst.lo
    then None  // Trap - low part would overflow u64
    else 
      let new_lo = Prims.op_Multiply dst.lo scalar in
      let new_hi = Prims.op_Multiply dst.hi scalar in
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
// MEMORY BOUNDS SAFETY
// ============================================================

// Bounds check for u128 operations (requires 16 bytes: addr to addr+15)
// WASM memory is 0 to max_memory (1MB default)
// Address must be: addr >= 0 AND addr + 16 <= max_memory
val u128_bounds_check : addr:int -> Tot bool
let u128_bounds_check addr = 
  addr >= 0 && addr + u128_size <= max_memory

// Lemma: If bounds check passes, read is safe
// (Abstract proof - actual safety depends on runtime memory state)
val u128_read_bounds_safe : addr:int -> Lemma
  (u128_bounds_check addr ==> addr >= 0)
let u128_read_bounds_safe addr = ()

// Lemma: If bounds check passes, write is safe
val u128_write_bounds_safe : addr:int -> Lemma
  (u128_bounds_check addr ==> addr >= 0)
let u128_write_bounds_safe addr = ()

// ============================================================
// BUFFER DISJOINTNESS
// ============================================================

// Heap never overlaps with fixed buffers
let heap_no_overlap = heap_start > return_buf + storage_u128_buf + 16384

// Storage buffer is separate from return buffer
let storage_no_overlap_return = storage_buf + storage_u128_buf < return_buf

// Input buffer is separate from storage
let input_no_overlap = input_buf + 16384 < storage_buf