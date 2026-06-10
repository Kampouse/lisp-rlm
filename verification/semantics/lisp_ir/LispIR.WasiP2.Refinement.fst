(** WasiP2 Refinement: F* Constants Match Rust Implementation

    This module documents that F* model constants match the Rust implementation.
    These are AXIOMS - we verify manually by cross-checking source code.
    
    REFINEMENT OBLIGATIONS:
    1. Verify storage_buf_addr matches Rust STORAGE_BUF
    2. Verify input_buf_addr matches Rust INPUT_BUF
    3. Verify return_buf_addr matches Rust RETURN_BUF
    4. Verify heap_start_addr matches Rust HEAP_START
    5. Verify buffer sizes match Rust constants
    
    RUST SOURCE LOCATIONS:
    - src/wasm_emit/mod.rs — memory layout constants
    - src/wasm_emit/call_u128.rs — u128 memory operations
    - src/bin/near_mock.rs — mock implementations
    
    VERIFICATION METHOD:
    $ grep -r "const.*BUF\|const.*HEAP" src/wasm_emit/mod.rs
    $ grep -r "STORAGE_BUF\|INPUT_BUF\|RETURN_BUF\|HEAP_START" src/
    
    Last verified: 2026-06-09 (commit 63d42ef)
*)

module LispIR.WasiP2.Refinement

open LispIR.WasiP2

// ============================================================
// REFINEMENT AXIOMS (verified manually)
// ============================================================

// RUST CONSTANTS (from src/wasm_emit/mod.rs)
// These are the ACTUAL values from Rust - verified manually
// If Rust changes these, this file MUST be updated
let rust_storage_buf : int = 8192
let rust_input_buf : int = 16384
let rust_return_buf : int = 32768
let rust_heap_start : int = 200000
let rust_amount_mem : int = 256

// Buffer sizes (derived from layout)
let rust_storage_buf_size : int = 8192
let rust_input_buf_size : int = 8192
let rust_return_buf_size : int = 16384

// ============================================================
// REFINEMENT PROOFS: F* constants match Rust
// ============================================================

// THEOREM: F* storage_buf_addr equals Rust STORAGE_BUF
// PROOF: Both are 8192
val refinement_storage : unit -> Lemma
  (ensures (storage_buf_addr = rust_storage_buf))
let refinement_storage () = ()

// THEOREM: F* input_buf_addr equals Rust INPUT_BUF
// PROOF: Both are 16384
val refinement_input : unit -> Lemma
  (ensures (input_buf_addr = rust_input_buf))
let refinement_input () = ()

// THEOREM: F* return_buf_addr equals Rust RETURN_BUF
// PROOF: Both are 32768
val refinement_return : unit -> Lemma
  (ensures (return_buf_addr = rust_return_buf))
let refinement_return () = ()

// THEOREM: F* heap_start_addr equals Rust HEAP_START
// PROOF: Both are 200000
val refinement_heap : unit -> Lemma
  (ensures (heap_start_addr = rust_heap_start))
let refinement_heap () = ()

// THEOREM: All F* constants match Rust
// PROOF: All values are equal (Z3 can prove arithmetic)
val refinement_all : unit -> Lemma
  (ensures (
    storage_buf_addr = rust_storage_buf /\
    input_buf_addr = rust_input_buf /\
    return_buf_addr = rust_return_buf /\
    heap_start_addr = rust_heap_start
  ))
let refinement_all () = ()

// THEOREM: Buffer sizes match
// PROOF: All values are equal
val refinement_sizes : unit -> Lemma
  (ensures (
    storage_buf_size = rust_storage_buf_size /\
    input_buf_size = rust_input_buf_size /\
    return_buf_size = rust_return_buf_size
  ))
let refinement_sizes () = ()

// ============================================================
// SAFETY PROPERTIES DERIVED FROM REFINEMENT
// ============================================================

// THEOREM: Rust buffer layout is safe (no overlaps)
// PROOF: Follows from refinement + buffer_ordering
val rust_buffer_safety : unit -> Lemma
  (ensures (
    rust_storage_buf + rust_storage_buf_size <= rust_input_buf /\
    rust_input_buf + rust_input_buf_size <= rust_return_buf /\
    rust_return_buf + rust_return_buf_size <= rust_heap_start
  ))
let rust_buffer_safety () = 
  refinement_all ();
  refinement_sizes ()

// ============================================================
// VERIFICATION CHECKLIST
// ============================================================

// RUN THIS TO VERIFY:
// $ cd /path/to/lisp-rlm
// $ grep -E "STORAGE_BUF|INPUT_BUF|RETURN_BUF|HEAP_START|AMOUNT_MEM" src/wasm_emit/mod.rs
// 
// EXPECTED OUTPUT:
// const STORAGE_BUF: usize = 8192;
// const INPUT_BUF: usize = 16384;
// const RETURN_BUF: usize = 32768;
// const HEAP_START: usize = 200000;
// const AMOUNT_MEM: usize = 256;
//
// IF VALUES DIFFER, UPDATE THIS FILE AND WasiP2.fst CONSTANTS

// ============================================================
// TESTS
// ============================================================

// TEST: Refinement constants are positive
val test_refinement_positive : unit -> Lemma
  (ensures (rust_storage_buf > 0 /\ rust_input_buf > 0 /\ 
            rust_return_buf > 0 /\ rust_heap_start > 0))
let test_refinement_positive () = ()

// TEST: Refinement ordering matches F*
val test_refinement_ordering : unit -> Lemma
  (ensures (rust_storage_buf < rust_input_buf /\ 
            rust_input_buf < rust_return_buf /\
            rust_return_buf < rust_heap_start))
let test_refinement_ordering () = ()

// TEST: All refinements pass
val test_refinement_complete : unit -> Lemma
  (ensures (True))
let test_refinement_complete () =
  refinement_storage ();
  refinement_input ();
  refinement_return ();
  refinement_heap ();
  refinement_all ();
  refinement_sizes ();
  rust_buffer_safety ()