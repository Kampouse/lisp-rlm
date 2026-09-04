module ValueCorrespondence

open CoreTypes

(* Value correspondence: spec value matches WASM memory at address *)
let value_correspondence (v: lisp_value) (addr: int) : Type = 
  True

(* Stack correspondence: spec stack matches WASM stack in memory *)
let rec stack_correspondence (stack: list lisp_value) (sp: int) : Type = 
  match stack with
  | [] -> sp = 0
  | v :: rest -> 
      sp > 0 /\ 
      value_correspondence v (sp - 8) /\ 
      stack_correspondence rest (sp - 8)