# F* Memory Model for lisp-rlm

## Current State

The F* verification model (`verification/semantics/lisp_ir/*.fst`) has **NO memory model**. It models:

| Component | F* Model | Rust Implementation |
|-----------|----------|---------------------|
| Stack | `list lisp_val` | `Vec<LispVal>` (VM stack) |
| Slots | `list lisp_val` | `Vec<LispVal>` (lambda locals) |
| Environment | `list (string * lisp_val)` | `im::HashMap<String, LispVal>` |
| **Memory** | ❌ None | WASM linear memory + tagged pointers |
| **Heap** | ❌ None | Bump allocator at address 56 |
| **Buffers** | ❌ None | Fixed addresses (STORAGE_BUF, INPUT_BUF, etc.) |

### Memory Operations F* Cannot Verify

1. **Tagged pointers** — `(ptr << 3) | tag`, `(ptr >> 3)` to untag
2. **u128 load/store** — 16-byte reads/writes at tagged addresses
3. **String allocation** — `emit_runtime_alloc` bump pointer
4. **Array allocation** — Count prefix, then elements
5. **Buffer overflows** — No bounds checks in F*
6. **Memory aliasing** — Same address used for different types

---

## Memory Bugs F* Would Have Caught

### Bug 1: u128 Store Overwrites Adjacent Memory (Fixed in commit `557192c`)

```rust
// BEFORE (broken): stored tagged value, corrupting adjacent u128
v.extend(addr);
v.push(Instruction::I64Store(ma));  // Writes tagged addr, not raw bytes!
```

F* model would prove: `store_u128(addr, lo, hi) → mem[addr] = lo /\ mem[addr+8] = hi`

### Bug 2: Division Overflow (Fixed in commit `6d0c3b5`)

```rust
// div_by_zero: returns 0 in Semantics, sets ok=false in ClosureVM, returns Err in Rust
```

F* model would prove: `div _ 0 → Err "division by zero"`

### Bug 3: SlotAddImm Write-Back Divergence (Still in F* diff)

```
F* Semantics: slots = slots'; stack = r :: s.stack  (writes back)
Rust: stack.push(result)  (no write-back)
```

---

## Proposed F* Memory Model

### 1. Memory Representation

```fstar
module LispIR.Memory

// Memory is a byte array (simplified from WASM's actual page model)
type mem = array byte

// Tagged value = (raw_ptr << 3) | tag
let TAG_NUM = 0
let TAG_STR = 5
let TAG_ARRAY = 6
let TAG_INVALID = 7

// A memory address is either:
// - Raw pointer (untagged)
// - Tagged pointer (requires untag to access)
noeq type tagged_ptr = {
  raw: nat;      // Actual memory address
  tag: nat;      // 0-6 valid, 7 invalid
}

// val tag : ptr:nat -> tag:nat{tag < 7} -> tagged_ptr
let tag ptr t = { raw = ptr; tag = t }

// val untag : tagged_ptr -> nat
let untag p = p.raw

// val tag_validate : tagged_ptr -> Tot (option tagged_ptr)
let tag_validate p =
  if p.tag = TAG_INVALID then None
  else Some p
```

### 2. Buffer Layout

```fstar
// Fixed memory layout (from wasm_emit/mod.rs)
let RUNTIME_HEAP_PTR = 56   // 8-byte slot holding bump allocator
let AMOUNT_MEM = 256        // u128 deposit buffer (256..272)
let INPUT_BUF = 16384      // 16KB input JSON
let RETURN_BUF = 32768      // Return value buffer
let STORAGE_BUF = 8192      // 8-byte storage buffer
let STORAGE_U128_BUF = 8208 // 16-byte u128 storage
let HEAP_START = 200000     // Bump allocator starts here

// Memory range predicate
val mem_in_range : mem -> addr:nat -> len:nat -> Tot bool
let mem_in_range m addr len = addr + len < Array.length m
```

### 3. u128 Operations

```fstar
// u128 is two i64 values (lo, hi) stored at adjacent addresses
noeq type u128 = { lo: int; hi: int }

// Load u128 from memory
val u128_load : mem -> addr:tagged_ptr -> Tot (result (u128, string))
let u128_load m p =
  if tag_validate p = None then Err "invalid tag"
  else if not (mem_in_range m p.raw 16) then Err "out of bounds"
  else
    let lo = bytes_to_i64 (Array.sub m p.raw 8) in
    let hi = bytes_to_i64 (Array.sub m (p.raw + 8) 8) in
    Ok { lo; hi }

// Store u128 to memory (returns new memory)
val u128_store : mem -> addr:tagged_ptr -> v:u128 -> Tot (result mem string)
let u128_store m p v =
  if tag_validate p = None then Err "invalid tag"
  else if not (mem_in_range m p.raw 16) then Err "out of bounds"
  else
    let m' = Array.copy m in
    Array.blit (i64_to_bytes v.lo) 0 m' p.raw 8;
    Array.blit (i64_to_bytes v.hi) 0 m' (p.raw + 8) 8;
    Ok m'

// Key theorem: load after store returns same value
val u128_store_load : m:mem -> p:tagged_ptr -> v:u128 
  -> Tot (Lemma (requires (mem_in_range m p.raw 16 /\ tag_validate p <> None))
                 (ensures (u128_store m p v = Ok m' /\ u128_load m' p = Ok v)))
let u128_store_load m p v = ()
```

### 4. Bump Allocator

```fstar
// Runtime heap pointer at address 56
val runtime_heap_ptr_read : mem -> Tot int
let runtime_heap_ptr_read m = bytes_to_i64 (Array.sub m RUNTIME_HEAP_PTR 8)

val runtime_heap_ptr_write : mem -> new_ptr:nat -> Tot mem
let runtime_heap_ptr_write m new_ptr =
  let m' = Array.copy m in
  Array.blit (i64_to_bytes new_ptr) 0 m' RUNTIME_HEAP_PTR 8;
  m'

// Allocate n bytes: bump and return old pointer
val alloc : mem -> n:nat -> Tot (result (mem * nat) string)
let alloc m n =
  let cur = runtime_heap_ptr_read m in
  let new_ptr = cur + n in
  let limit = 65536 * 16 in  // Default: 16 pages
  if new_ptr >= limit then Err "memory exhausted"
  else
    let m' = runtime_heap_ptr_write m new_ptr in
    Ok (m', cur)

// Key theorem: sequential allocations don't overlap
val alloc_no_overlap : m:mem -> n1:nat -> n2:nat
  -> Tot (Lemma (requires (alloc m n1 = Ok (m1, p1) /\ alloc m1 n2 = Ok (m2, p2)))
                 (ensures (p1 + n1 <= p2)))
let alloc_no_overlap m n1 n2 = ()
```

### 5. String Operations

```fstar
// String in memory: ((len:u31) << 32) | ptr, tagged as STR
noeq type str_repr = {
  ptr: nat;      // Start address
  len: nat;      // Byte length
}

// Allocate string: bump allocator, write bytes, return tagged ptr
val str_alloc : mem -> bytes:array byte -> Tot (result (mem * tagged_ptr) string)
let str_alloc m bytes =
  let len = Array.length bytes in
  let alloc_size = (len + 8 + 7) / 8 * 8 in  // Align to 8 bytes
  match alloc m (8 + alloc_size) with
  | Err msg -> Err msg
  | Ok (m', ptr) ->
     // Write length at ptr
     Array.blit (i64_to_bytes len) 0 m' ptr 8;
     // Write bytes at ptr+8
     Array.blit bytes 0 m' (ptr + 8) len;
     // Return tagged string: (len << 32) | ptr, tagged as STR
     Ok (m', { raw = ptr; tag = TAG_STR })

// Key theorem: two string allocations don't overlap
val str_alloc_no_overlap : m:mem -> s1:array byte -> s2:array byte
  -> Tot (Lemma (requires (str_alloc m s1 = Ok (m1, p1) /\ str_alloc m1 s2 = Ok (m2, p2)))
                 (ensures (p1.raw + 8 + Array.length s1 <= p2.raw)))
let str_alloc_no_overlap m s1 s2 = ()
```

### 6. Tagged Pointer Safety

```fstar
// All memory accesses must untag first
val mem_load_i64 : mem -> tagged_ptr -> Tot (result int string)
let mem_load_i64 m p =
  if tag_validate p = None then Err "invalid tag"
  else if not (mem_in_range m (untag p) 8) then Err "out of bounds"
  else Ok (bytes_to_i64 (Array.sub m p.raw 8))

// Key theorem: tag-untag roundtrip
val tag_untag_inverse : p:nat -> t:nat{t < 7} 
  -> Tot (Lemma (requires True)
                 (ensures (untag (tag p t) = p)))
let tag_untag_inverse p t = ()

// Tag safety: all accesses go through tag_validate
val all_memory_accesses_safe : m:mem -> p:tagged_ptr
  -> Tot (Lemma (requires (mem_load_i64 m p = Ok v))
                 (ensures (tag_validate p <> None /\ mem_in_range m p.raw 8)))
let all_memory_accesses_safe m p = ()
```

---

## Integration with Existing F* Model

### Step 1: Add Memory to VM State

```fstar
// In LispIR.Semantics.fst
type vm_state = {
  // Existing fields
  stack: list lisp_val;
  slots: list lisp_val;
  env: list (string * lisp_val);
  
  // NEW: Memory model
  mem: mem;                    // Linear memory (byte array)
  heap_ptr: nat;               // Bump allocator pointer
}

// NEW: Memory operations in opcode
type mem_op =
  | MemStoreU128 of addr:tagged_ptr * val:u128
  | MemLoadU128 of addr:tagged_ptr
  | MemAlloc of size:nat
  | MemStoreStr of addr:tagged_ptr * bytes:array byte
  | MemLoadStr of addr:tagged_ptr
```

### Step 2: Prove Memory Safety for Each Opcode

```fstar
// Example: u128/add memory safety
val u128_add_safe : mem -> dst:tagged_ptr -> src:tagged_ptr
  -> Tot (Lemma (requires (u128_load m src = Ok sv /\ 
                           u128_load m dst = Ok dv /\ 
                           dv.hi + sv.hi < 2^63))  // No carry overflow
                 (ensures (u128_add m dst src = 
                           Ok (m', { lo = dv.lo + sv.lo; 
                                     hi = dv.hi + sv.hi + carry }))))
let u128_add_safe m dst src = ()
```

### Step 3: Property-Based Testing

```fstar
// Generate random memory states and operations
val prop_u128_roundtrip : unit -> Tot bool
let prop_u128_roundtrip () =
  let m = fresh_mem 65536 in
  let addr = random_tagged_ptr (range HEAP_START 400000) TAG_U128 in
  let v = random_u128 () in
  match u128_store m addr v with
  | Err _ -> true  // Out of bounds is OK
  | Ok m' -> u128_load m' addr = Ok v

// Run 1000 random tests
// F* extracts to OCaml/Haskell for property-based testing
```

---

## Memory Bugs This Would Catch

| Bug Type | F* Model | Rust Code | Status |
|----------|----------|-----------|--------|
| u128 store overwrites adjacent memory | `u128_store` proves bounds | Fixed in `557192c` | ✅ Fixed |
| Division overflow | `div_by_zero` returns Err | Fixed in `6d0c3b5` | ✅ Fixed |
| Heap pointer overflow | `alloc` checks `new_ptr < limit` | `emit_runtime_alloc` line 1005 | ✅ Has check |
| String overlapping allocations | `alloc_no_overlap` theorem | bump allocator is linear | ⚠️ Verify |
| Tag validation skip | `tag_validate` before all accesses | Some paths skip validation | 🔴 Needs audit |
| Load/store at untagged addresses | `untag` required for all memory | Fixed in `557192c` | ✅ Fixed |
| Memory aliasing (STORAGE_BUF overlaps heap) | Buffer addresses in model | `STORAGE_BUF = 8192`, `HEAP_START = 200000` | ✅ Safe |

---

## Recommended Verification Order

1. **Tagged pointer encoding** — Prove `tag_untag_inverse`
2. **u128 operations** — Prove `u128_store_load`, bounds checks
3. **Bump allocator** — Prove `alloc_no_overlap`
4. **String operations** — Prove `str_alloc_no_overlap`
5. **Buffer overlaps** — Prove `STORAGE_BUF` doesn't overlap `HEAP_START`
6. **Memory safety audit** — Find all `I64Load`/`I64Store` sites, verify bounds

This would have caught the `u128/store` bug where tagged addresses were stored instead of raw bytes.