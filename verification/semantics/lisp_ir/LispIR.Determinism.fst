(** Determinism — eval_expr and eval_steps produce unique results.
    Trivially true because both are pure Tot functions (no state, no IO).
    Stated formally as: if eval returns Ok v1 and Ok v2, then v1 = v2.
    
    For types containing ffloat (not eqtype), we can't use = on lisp_val directly.
    Instead we prove determinism on the result structure we can observe.
*)
module LispIR.Determinism

open Lisp.Types
open Lisp.Values
open Lisp.Source
open LispIR.Semantics

// === Eval determinism: result tag is unique ===
// Since eval_result contains lisp_val (which has ffloat), it's not eqtype.
// We prove: the Ok/Err tag is deterministic (F* already knows this from Tot).
// The interesting lemma: for int results, the int is unique.

val eval_num_det : fuel:int -> e:lisp_val -> env:env -> Lemma
  (match eval_expr fuel e env with
   | Lisp.Source.Ok (Num a) ->
     (match eval_expr fuel e env with
      | Lisp.Source.Ok (Num b) -> a = b
      | _ -> true)
   | _ -> true)
let eval_num_det fuel e env = ()

val eval_bool_det : fuel:int -> e:lisp_val -> env:env -> Lemma
  (match eval_expr fuel e env with
   | Lisp.Source.Ok (Bool a) ->
     (match eval_expr fuel e env with
      | Lisp.Source.Ok (Bool b) -> a = b
      | _ -> true)
   | _ -> true)
let eval_bool_det fuel e env = ()

// Same for VM: eval_steps is pure Tot, so deterministic by construction.
// Stated for int stack results:

val vm_num_det : n:nat -> s:vm_state -> Lemma
  (match eval_steps n s with
   | LispIR.Semantics.Ok s1 ->
     (match s1.stack with
      | Num a :: _ ->
        (match eval_steps n s with
         | LispIR.Semantics.Ok s2 ->
           (match s2.stack with
            | Num b :: _ -> a = b
            | _ -> true)
         | _ -> true)
      | _ -> true)
   | _ -> true)
let vm_num_det n s = ()

val vm_bool_det : n:nat -> s:vm_state -> Lemma
  (match eval_steps n s with
   | LispIR.Semantics.Ok s1 ->
     (match s1.stack with
      | Bool a :: _ ->
        (match eval_steps n s with
         | LispIR.Semantics.Ok s2 ->
           (match s2.stack with
            | Bool b :: _ -> a = b
            | _ -> true)
         | _ -> true)
      | _ -> true)
   | _ -> true)
let vm_bool_det n s = ()
