(** Dict get/set compiler specs + VM correctness *)
module DictCompilerSpec

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics

val fresh_vm : list opcode -> vm_state
let fresh_vm code = { stack = []; slots = []; pc = 0; code = code; ok = true }

val vm_get_existing : unit -> Lemma
  (match eval_steps 100 {
    stack = [Str "x"; Dict [("x", Num 42)]];
    slots = [];
    pc = 0;
    code = [DictGet; Return];
    ok = true
  } with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Num r :: _ -> r = 42
     | _ -> false)
   | _ -> false)
let vm_get_existing () = ()

val vm_set_new_key : unit -> Lemma
  (match eval_steps 100 {
    stack = [Num 99; Str "y"; Dict [("x", Num 42)]];
    slots = [];
    pc = 0;
    code = [DictSet; Return];
    ok = true
  } with
   | LispIR.Semantics.Ok s' -> (match s'.stack with
     | Dict entries :: _ ->
       (match entries with
        | [(ky, Num vy); (kx, Num vx)] -> ky = "y" && vy = 99 && kx = "x" && vx = 42
        | _ -> false)
     | _ -> false)
   | _ -> false)
let vm_set_new_key () = ()
