(** nil? compiler output spec *)
module NilQSpec

open Lisp.Types
open Lisp.Values
open Lisp.Compiler
open LispIR.Semantics

val nilq_nil_spec : fuel:int -> Lemma
  (fuel > 5 ==> (match compile_lambda fuel [] (List [Sym "nil?"; Nil]) with
    | None -> true
    | Some code -> (match code with
      | [PushNil; PushNil; OpEq; Return] -> true
      | _ -> false)))
let nilq_nil_spec fuel = ()

val nilq_num_spec : fuel:int -> n:int -> Lemma
  (fuel > 5 ==> (match compile_lambda fuel [] (List [Sym "nil?"; Num n]) with
    | None -> true
    | Some code -> (match code with
      | [PushI64 m; PushNil; OpEq; Return] -> m = n
      | _ -> false)))
let nilq_num_spec fuel n = ()
