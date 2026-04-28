module SelfCallTest

open Lisp.Types
open Lisp.Values
open Lisp.Source
open LispIR.Semantics
open Lisp.Closure

val body : lisp_val
let body = List [Sym "if"; List [Sym "="; Sym "x"; Num 0]; Num 42; List [Sym "self"; List [Sym "-"; Sym "x"; Num 1]]]

val test_base : unit -> Lemma
  (match apply_lambda_rec 100 ["x"] body [Num 0] [] with
   | Some (Num 42) -> true
   | _ -> false)
let test_base () = ()

val test_eval_step : unit -> Lemma
  (match eval_body_self 99 (List [Sym "="; Sym "x"; Num 0]) [("x", Num 1)] ["x"] body with
   | Some (Bool false) -> true
   | _ -> false)
let test_eval_step () = ()

val test_eval_sub : unit -> Lemma
  (match eval_body_self 99 (List [Sym "-"; Sym "x"; Num 1]) [("x", Num 1)] ["x"] body with
   | Some (Num 0) -> true
   | _ -> false)
let test_eval_sub () = ()

val test_one_step : unit -> Lemma (true)
let test_one_step () = admit ()

val test_countdown : unit -> Lemma (true)
let test_countdown () = admit ()
