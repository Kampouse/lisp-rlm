(** Stack Height — fuel experiments for arithmetic *)
module StackFuel

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics

val fresh_vm : list opcode -> vm_state
let fresh_vm code = { stack = []; slots = []; pc = 0; code = code; ok = true }

val stack_is_one : list lisp_val -> Tot bool
let stack_is_one s = match s with | [_] -> true | _ -> false

// Try: compile by hand what compile_chain produces for (+ a b)
// Expected: [PushI64 a; PushI64 b; OpAdd; Return]
val add_manual : a:int -> b:int -> Lemma
  (let code = [PushI64 a; PushI64 b; OpAdd; Return] in
   match eval_steps 1000 (fresh_vm code) with
   | LispIR.Semantics.Ok s' -> stack_is_one s'.stack
   | _ -> false)
let add_manual a b = ()

// Now try through compile_lambda
val add_through_compile : a:int -> b:int -> Lemma
  (match compile_lambda 100 [] (List [Sym "+"; Num a; Num b]) with
   | None -> true
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> stack_is_one s'.stack
     | _ -> false)
let add_through_compile a b = admit ()
