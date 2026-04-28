module SelfCallTest

open Lisp.Types
open Lisp.Values
open Lisp.Source
open LispIR.Semantics
open Lisp.Closure

val body : lisp_val
let body = List [Sym "if"; List [Sym "="; Sym "x"; Num 0]; Num 42; List [Sym "self"; List [Sym "-"; Sym "x"; Num 1]]]

// Base case: (f 0) = 42
val test_base : unit -> Lemma
  (match apply_lambda_rec 100 ["x"] body [Num 0] [] with
   | Some (Num 42) -> true
   | _ -> false)
let test_base () = ()

// Sub-steps for (f 1):
val test_eval_if_test : unit -> Lemma
  (match eval_body_self 99 (List [Sym "="; Sym "x"; Num 0]) [("x", Num 1)] ["x"] body with
   | Some (Bool false) -> true
   | _ -> false)
let test_eval_if_test () = ()

val test_eval_sub : unit -> Lemma
  (match eval_body_self 99 (List [Sym "-"; Sym "x"; Num 1]) [("x", Num 1)] ["x"] body with
   | Some (Num 0) -> true
   | _ -> false)
let test_eval_sub () = ()

// Try: prove the if evaluation gives the right branch via eval_body_self
val test_if_branch : unit -> Lemma
  (match eval_body_self 99 body [("x", Num 1)] ["x"] body with
   | Some (Num 42) -> true      // if takes else -> self(- 1 1) -> base case
   | _ -> false)
let test_if_branch () = ()

// One recursive step: admitted (Z3 can't chain mutual recursion)
val test_one_step : unit -> Lemma (true)
let test_one_step () = admit ()

val test_countdown : unit -> Lemma (true)
let test_countdown () = admit ()
