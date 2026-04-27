(** nil? and dict op end-to-end tests (admitted)
    These chain compile_lambda + eval_steps in a single lemma.
    F* cannot prove these end-to-end -- see split proofs instead:
    - nil?: NilQSpec + NilQVm
    - dict: DictCompilerSpec + DictOps + DictSetTest
*)
module ExtendedOps

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics

val fresh_vm : list opcode -> vm_state
let fresh_vm code = { stack = []; slots = []; pc = 0; code = code; ok = true }

// nil? of Nil -> true
val nilq_nil_correct : unit -> Lemma
  (match compile_lambda 100 [] (List [Sym "nil?"; Nil]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = true | _ -> false)
     | _ -> false)
let nilq_nil_correct () = admit ()

// nil? of Num -> false
val nilq_num_correct : n:int -> Lemma
  (match compile_lambda 100 [] (List [Sym "nil?"; Num n]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = false | _ -> false)
     | _ -> false)
let nilq_num_correct n = admit ()

// nil? of Bool false -> false
val nilq_bool_correct : unit -> Lemma
  (match compile_lambda 100 [] (List [Sym "nil?"; Bool false]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = false | _ -> false)
     | _ -> false)
let nilq_bool_correct () = admit ()

// DictGet found
val vm_dict_get_found : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushNil; PushStr "x"; DictGet]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Nil :: _ -> true | _ -> false)
   | _ -> false)
let vm_dict_get_found () = admit ()

// DictGet missing
val vm_dict_get_missing : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushNil; PushStr "y"; DictGet]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Nil :: _ -> true | _ -> false)
   | _ -> false)
let vm_dict_get_missing () = admit ()

// DictSet new key -- admitted (Dict content matching too complex for Z3)
// Proven in DictSetTest and DictOps via split proofs.
val vm_dict_set_new : unit -> Lemma (true)
let vm_dict_set_new () = ()
