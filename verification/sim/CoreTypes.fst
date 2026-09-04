module CoreTypes

(* Tag constants from lisp-rlm tagged_value.rs *)
let tag_num : int = 0
let tag_bool : int = 1
let tag_fnref : int = 2
let tag_closure : int = 3
let tag_nil : int = 4
let tag_str : int = 5
let tag_array : int = 6

let tag_bits : int = 3
let tag_mask : int = 7

(* Memory layout constants from tagged_value.rs *)
let runtime_heap_ptr : int = 56
let temp_mem : int = 64
let heap_start : int = 200000
let storage_buf : int = 8192
let input_buf : int = 16384
let return_buf : int = 32768
let borsh_buf : int = 36864

(* Lisp value ADT *)
type lisp_value = 
  | Num of int 
  | Bool of bool 
  | Nil
  | Str of (int * int)
  | Array of (int * int)
  | FnRef of int
  | Closure of int

(* Spec-level state *)
type lisp_spec_state = {
  stack: list lisp_value;
  fuel: int;
  result: option lisp_value
}

(* WASM runtime state *)
type wasm_runtime = { 
  gas: int; 
  stack_ptr: int; 
  heap_ptr: int; 
  max_memory: int 
}

(* Memory invariant: heap starts at heap_start, grows up *)
let heap_invariant (w: wasm_runtime) : Type = 
  w.heap_ptr >= heap_start /\ w.heap_ptr < w.max_memory

(* Stack invariant: stack above buffers, below heap *)
let stack_invariant (w: wasm_runtime) : Type = 
  w.stack_ptr >= temp_mem /\ w.stack_ptr < heap_start

(* Gas invariant: non-negative *)
let gas_invariant (w: wasm_runtime) : Type = 
  w.gas >= 0

(* Full runtime invariant *)
let runtime_inv (w: wasm_runtime) : Type = 
  heap_invariant w /\ stack_invariant w /\ gas_invariant w

(* Buffer bounds are disjoint *)
let buffers_disjoint : Type = 
  temp_mem < storage_buf /\ 
  storage_buf < input_buf /\ 
  input_buf < return_buf /\ 
  return_buf < borsh_buf /\ 
  borsh_buf < heap_start