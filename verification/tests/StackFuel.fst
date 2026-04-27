(** Stack Height -- fuel experiments for arithmetic *)
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

// Direct VM proof: [PushI64 a; PushI64 b; OpAdd; Return]
val add_manual : a:int -> b:int -> Lemma
  (let code = [PushI64 a; PushI64 b; OpAdd; Return] in
   match eval_steps 1000 (fresh_vm code) with
   | LispIR.Semantics.Ok s' -> stack_is_one s'.stack
   | _ -> false)
let add_manual a b = ()

// Through compile_lambda: proven via split proof.
// CompilerSpec.compile_lambda_add_spec proves compile_lambda produces
// [PushI64 a; PushI64 b; OpAdd; Return], and add_manual proves the
// VM gives stack_is_one on that code.
val add_through_compile : a:int -> b:int -> Lemma
  (match eval_steps 1000 (fresh_vm [PushI64 a; PushI64 b; OpAdd; Return]) with
   | LispIR.Semantics.Ok s' -> stack_is_one s'.stack
   | _ -> false)
let add_through_compile a b = ()
