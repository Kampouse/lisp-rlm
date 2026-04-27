(** nil? VM test on known code *)
module NilQVm

open Lisp.Types
open Lisp.Values
open LispIR.Semantics

val fresh_vm : list opcode -> vm_state
let fresh_vm code = { stack = []; slots = []; pc = 0; code = code; ok = true }

val vm_nilq_nil : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushNil; PushNil; OpEq; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = true | _ -> false)
   | _ -> false)
let vm_nilq_nil () = ()

val vm_nilq_num : n:int -> Lemma
  (match eval_steps 100 (fresh_vm [PushI64 n; PushNil; OpEq; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = false | _ -> false)
   | _ -> false)
let vm_nilq_num n = ()
