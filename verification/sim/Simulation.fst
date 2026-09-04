module Simulation

open CoreTypes
open ValueCorrespondence

(* Simulation relation: spec state R WASM state *)
let simulation_rel (s: lisp_spec_state) (w: wasm_runtime) : Type = 
  s.fuel = w.gas /\ 
  stack_correspondence s.stack w.stack_ptr /\ 
  runtime_inv w

(* Step preservation: if simulation holds before, it holds after one step
   This is the MAIN THEOREM to prove for correctness *)
let step_preservation (s1: lisp_spec_state) (w1: wasm_runtime)
                      (s2: lisp_spec_state) (w2: wasm_runtime) : Type =
  simulation_rel s1 w1 ->
  simulation_rel s2 w2

(* Fuel decreases on each step (termination measure) *)
let fuel_decreases (s1: lisp_spec_state) (s2: lisp_spec_state) : Type =
  s2.fuel < s1.fuel \/ s1.fuel = 0

(* Heap grows monotonically *)
let heap_grows (w1: wasm_runtime) (w2: wasm_runtime) : Type =
  w2.heap_ptr >= w1.heap_ptr

(* Stack pointer decreases (stack grows down) *)
let stack_grows_down (w1: wasm_runtime) (w2: wasm_runtime) : Type =
  w2.stack_ptr <= w1.stack_ptr

(* Combined preservation: all invariants hold after step *)
let full_preservation (s1: lisp_spec_state) (w1: wasm_runtime)
                      (s2: lisp_spec_state) (w2: wasm_runtime) : Type =
  simulation_rel s1 w1 /\ 
  fuel_decreases s1 s2 /\
  heap_grows w1 w2 /\
  stack_grows_down w1 w2 ->
  simulation_rel s2 w2