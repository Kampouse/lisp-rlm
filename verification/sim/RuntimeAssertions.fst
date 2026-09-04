module RuntimeAssertions

open CoreTypes
open ValueCorrespondence
open Simulation
open HttpStateMachine
open Integration

(* Boolean check for runtime_inv *)
let check_runtime_inv (w: wasm_runtime) : bool = 
  w.heap_ptr >= heap_start && 
  w.heap_ptr < w.max_memory && 
  w.stack_ptr >= temp_mem && 
  w.stack_ptr < heap_start && 
  w.gas >= 0

(* Boolean check for simulation_rel *)
let check_simulation_rel (s: lisp_spec_state) (w: wasm_runtime) : bool = 
  s.fuel = w.gas && check_runtime_inv w

(* Boolean check for http_invariant *)
let check_http_invariant (s: http_state) : bool = 
  s <> Error

(* Boolean check for system_invariant *)
let check_system_invariant (sys: system_state) : bool = 
  check_http_invariant sys.http && 
  check_runtime_inv sys.wasm

(* Tag validation *)
let valid_tag (tagged: int) : bool =
  let tag = tagged % 8 in
  tag >= 0 && tag <= 6

(* Memory bounds check *)
let check_memory_bounds (addr: int) (max_mem: int) : bool =
  addr >= 0 && addr < max_mem

(* Heap allocation check *)
let check_heap_alloc (heap_ptr: int) (size: int) (max_mem: int) : bool =
  heap_ptr + size < max_mem

(* Stack bounds check *)
let check_stack_bounds (stack_ptr: int) : bool =
  stack_ptr >= temp_mem && stack_ptr < heap_start

(* Value encoding check *)
let check_value_encoding (v: lisp_value) : bool =
  match v with
  | Num n -> n >= -1152921504606846976 && n <= 1152921504606846975  (* i61 bounds *)
  | Bool _ -> true
  | Nil -> true
  | Str (_, len) -> len >= 0 && len < 4294967296  (* u32 len *)
  | Array (_, count) -> count >= 0
  | FnRef idx -> idx >= 0 && idx < 4294967296  (* u32 index *)
  | Closure ptr -> ptr >= 0 && ptr < 4294967296  (* u32 pointer *)