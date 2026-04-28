(** Shadowing Bug -- FIXED *)
module ShadowingFix

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics

val shadowing_expr : lisp_val
let shadowing_expr = List [
  Sym "let";
  List [List [Sym "x"; Num 1]];
  List [
    Sym "let";
    List [List [Sym "x"; Num 2]];
    Sym "x"
  ]
]

val slot_of_finds_correct : unit -> Lemma
  (match slot_of "x" ["x"; "x"] with | Some 1 -> true | _ -> false)
let slot_of_finds_correct () = ()

val slot_of_non_shadowed : unit -> Lemma
  (match slot_of "x" ["x"; "y"] with | Some 0 -> true | _ -> false)
let slot_of_non_shadowed () = ()

val slot_of_single : unit -> Lemma
  (match slot_of "x" ["x"] with | Some 0 -> true | _ -> false)
let slot_of_single () = ()

val slot_of_triple : unit -> Lemma
  (match slot_of "x" ["x"; "x"; "x"] with | Some 2 -> true | _ -> false)
let slot_of_triple () = ()

val vm_gives_correct_2 : unit -> Lemma
  (match eval_steps 100 { stack = []; slots = []; pc = 0;
    code = [PushI64 1; StoreSlot 0; PushI64 2; StoreSlot 1; LoadSlot 1; Return];
    ok = true } with
   | Ok s' -> (match s'.stack with Num 2 :: _ -> true | _ -> false)
   | _ -> false)
let vm_gives_correct_2 () = ()

val inner_let_spec : fuel:int -> Lemma
  (fuel > 10 ==> (match compile_lambda fuel ["x"]
    (List [Sym "let"; List [List [Sym "x"; Num 2]]; Sym "x"]) with
   | Some [PushI64 2; StoreSlot 1; LoadSlot 1; Return] -> true
   | _ -> false))
let inner_let_spec fuel = ()

// Use assert_norm to force F* to compute compile_lambda on the shadowing expr
val compiler_produces_fixed_code : unit -> Lemma
  (match compile_lambda 100 [] shadowing_expr with
   | Some [PushI64 1; StoreSlot 0; PushI64 2; StoreSlot 1; LoadSlot 1; Return] -> true
   | _ -> false)
let compiler_produces_fixed_code () =
  assert_norm (match compile_lambda 100 [] shadowing_expr with
   | Some [PushI64 1; StoreSlot 0; PushI64 2; StoreSlot 1; LoadSlot 1; Return] -> true
   | _ -> false)
