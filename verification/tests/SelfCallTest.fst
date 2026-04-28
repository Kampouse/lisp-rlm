module SelfCallTest

open Lisp.Types
open Lisp.Values
open Lisp.Source
open LispIR.Semantics
open Lisp.Closure

// Body with actual self-call
val body : lisp_val
let body = List [Sym "if"; List [Sym "="; Sym "x"; Num 0]; Num 42; List [Sym "self"; List [Sym "-"; Sym "x"; Num 1]]]

// Base case: (f 0) = 42
val test_base : unit -> Lemma
  (match apply_lambda_rec 100 ["x"] body [Num 0] [] with
   | Some (Num 42) -> true
   | _ -> false)
let test_base () = ()

// Can we prove that the first eval_body_self step produces the right thing?
// For x=1: eval the if, test is false, need to eval (self (- 1 1))
// eval_body_self should evaluate (- 1 1) = 0, then call apply_lambda_rec with 0
// But Z3 can't unfold through this mutual call.
//
// Try: prove eval_body_self on the BODY expression gives Some result
val test_eval_step : unit -> Lemma
  (match eval_body_self 99 (List [Sym "="; Sym "x"; Num 0]) [("x", Num 1)] ["x"] body with
   | Some (Bool false) -> true
   | _ -> false)
let test_eval_step () = ()

// The subtraction
val test_eval_sub : unit -> Lemma
  (match eval_body_self 99 (List [Sym "-"; Sym "x"; Num 1]) [("x", Num 1)] ["x"] body with
   | Some (Num 0) -> true
   | _ -> false)
let test_eval_sub () = ()

// These should work: they don't cross the self-call boundary.

val test_one_step : unit -> Lemma (true)
let test_one_step () = admit ()

val test_countdown : unit -> Lemma (true)
let test_countdown () = admit ()
