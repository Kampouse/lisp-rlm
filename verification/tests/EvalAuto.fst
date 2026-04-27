(** Test: which eval lemmas can F* auto-prove? *)
module EvalAuto

open Lisp.Types
open Lisp.Values
open Lisp.Source
open LispIR.Semantics

// === LITERALS ===

val test_num : n:int -> Lemma
  (match eval_expr 100 (Num n) [] with
   | Lisp.Source.Ok (Num m) -> m = n
   | _ -> false)
let test_num n = ()

val test_bool : b:bool -> Lemma
  (match eval_expr 100 (Bool b) [] with
   | Lisp.Source.Ok (Bool c) -> c = b
   | _ -> false)
let test_bool b = ()

val test_nil : unit -> Lemma
  (match eval_expr 100 Nil [] with
   | Lisp.Source.Ok Nil -> true
   | _ -> false)
let test_nil () = ()

// === ARITHMETIC ===

val test_add : a:int -> b:int -> Lemma
  (match eval_expr 100 (List [Sym "+"; Num a; Num b]) [] with
   | Lisp.Source.Ok (Num r) -> r = op_int_add a b
   | _ -> false)
let test_add a b = ()

val test_sub : a:int -> b:int -> Lemma
  (match eval_expr 100 (List [Sym "-"; Num a; Num b]) [] with
   | Lisp.Source.Ok (Num r) -> r = op_int_sub a b
   | _ -> false)
let test_sub a b = ()

val test_mul : a:int -> b:int -> Lemma
  (match eval_expr 100 (List [Sym "*"; Num a; Num b]) [] with
   | Lisp.Source.Ok (Num r) -> r = int_mul a b
   | _ -> false)
let test_mul a b = ()

// === COMPARISON ===

val test_gt : a:int -> b:int -> Lemma
  (match eval_expr 100 (List [Sym ">"; Num a; Num b]) [] with
   | Lisp.Source.Ok (Bool r) -> r = op_int_gt a b
   | _ -> false)
let test_gt a b = ()

val test_lt : a:int -> b:int -> Lemma
  (match eval_expr 100 (List [Sym "<"; Num a; Num b]) [] with
   | Lisp.Source.Ok (Bool r) -> r = op_int_lt a b
   | _ -> false)
let test_lt a b = ()

val test_eq : a:int -> b:int -> Lemma
  (match eval_expr 100 (List [Sym "="; Num a; Num b]) [] with
   | Lisp.Source.Ok (Bool r) -> r = (a = b)
   | _ -> false)
let test_eq a b = ()

// === BOOLEAN OPS ===

val test_not_true : unit -> Lemma
  (match eval_expr 100 (List [Sym "not"; Bool true]) [] with
   | Lisp.Source.Ok (Bool r) -> r = false
   | _ -> false)
let test_not_true () = ()

val test_not_false : unit -> Lemma
  (match eval_expr 100 (List [Sym "not"; Bool false]) [] with
   | Lisp.Source.Ok (Bool r) -> r = true
   | _ -> false)
let test_not_false () = ()

val test_nil_q_nil : unit -> Lemma
  (match eval_expr 100 (List [Sym "nil?"; Nil]) [] with
   | Lisp.Source.Ok (Bool r) -> r = true
   | _ -> false)
let test_nil_q_nil () = ()

val test_nil_q_num : n:int -> Lemma
  (match eval_expr 100 (List [Sym "nil?"; Num n]) [] with
   | Lisp.Source.Ok (Bool r) -> r = false
   | _ -> false)
let test_nil_q_num n = ()

// === SYMBOL LOOKUP ===

val test_sym : n:int -> Lemma
  (match eval_expr 100 (Sym "x") [("x", Num n)] with
   | Lisp.Source.Ok (Num m) -> m = n
   | _ -> false)
let test_sym n = ()

// === IF -- source eval side, try auto-prove ===
// NOTE: Num 0 is TRUTHY in Lisp! So (if 0 42 99) = 42, not 99.
// Use Bool false for the falsy case.

val test_if_true : unit -> Lemma
  (match eval_expr 100 (List [Sym "if"; Num 1; Num 42; Num 99]) [] with
   | Lisp.Source.Ok (Num r) -> r = 42
   | _ -> false)
let test_if_true () = ()

val test_if_false : unit -> Lemma
  (match eval_expr 100 (List [Sym "if"; Bool false; Num 42; Num 99]) [] with
   | Lisp.Source.Ok (Num r) -> r = 99
   | _ -> false)
let test_if_false () = ()

// === LET -- source eval side, try auto-prove ===

val test_let : n:int -> Lemma
  (match eval_expr 100 (List [Sym "let"; List [List [Sym "x"; Num n]]; Sym "x"]) [] with
   | Lisp.Source.Ok (Num m) -> m = n
   | _ -> false)
let test_let n = ()
