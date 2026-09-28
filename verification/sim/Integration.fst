module Integration

open CoreTypes
open ValueCorrespondence
open Simulation
open HttpStateMachine

(* Combined system state *)
type system_state = { 
  http: http_state; 
  lisp: option lisp_spec_state; 
  wasm: wasm_runtime
}

(* System invariant: both HTTP and lisp-rlm invariants hold *)
let system_invariant (sys: system_state) : Type = 
  http_invariant sys.http /\ 
  runtime_inv sys.wasm /\
  (match sys.lisp with
   | None -> True
   | Some s -> s.fuel = sys.wasm.gas)

(* HTTP request does not corrupt lisp state *)
let http_preserves_lisp (sys1: system_state) (sys2: system_state) : Type =
  sys1.lisp = sys2.lisp /\
  sys2.wasm.gas <= sys1.wasm.gas

(* Lisp execution does not corrupt HTTP state *)
let lisp_preserves_http (sys1: system_state) (sys2: system_state) : Type =
  sys1.http = sys2.http /\
  sys2.wasm.gas <= sys1.wasm.gas

(* Composition: both HTTP and lisp preserve system invariant *)
let composed_preservation (sys1: system_state) (sys2: system_state) : Type =
  system_invariant sys1 /\
  (http_preserves_lisp sys1 sys2 \/ lisp_preserves_http sys1 sys2) ->
  system_invariant sys2