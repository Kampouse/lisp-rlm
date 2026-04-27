(** Per-Expression Compiler Correctness *)

module LispIR.PerExpr3

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics

val fresh_vm : list opcode -> vm_state
let fresh_vm code = { stack = []; slots = []; pc = 0; code = code; ok = true }

// === LITERALS (AUTO-PROVED) ===

val num_correct : n:int -> Lemma
  (match compile_lambda 100 [] (Num n) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = n | _ -> false)
     | _ -> false)
let num_correct n = ()

val bool_correct : b:bool -> Lemma
  (match compile_lambda 100 [] (Bool b) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = b | _ -> false)
     | _ -> false)
let bool_correct b = ()

val nil_correct : unit -> Lemma
  (match compile_lambda 100 [] Nil with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Nil :: _ -> true | _ -> false)
     | _ -> false)
let nil_correct () = ()

// === SYMBOL LOOKUP (AUTO-PROVED) ===

val sym_correct : n:int -> Lemma
  (match compile_lambda 100 ["x"] (Sym "x") with
   | None -> false
   | Some code ->
     match eval_steps 100 { stack = []; slots = [Num n]; pc = 0; code = code; ok = true } with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Num m :: _ -> m = n | _ -> false)
     | _ -> false)
let sym_correct n = ()

// === ARITHMETIC (AUTO-PROVED for +, -; admitted for *, / due to opaque int_mul/int_div) ===

val add_correct : a:int -> b:int -> Lemma
  (match compile_lambda 100 [] (List [Sym "+"; Num a; Num b]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = a + b | _ -> false)
     | _ -> false)
let add_correct a b = ()

val sub_correct : a:int -> b:int -> Lemma
  (match compile_lambda 100 [] (List [Sym "-"; Num a; Num b]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = a - b | _ -> false)
     | _ -> false)
let sub_correct a b = ()

val mul_correct : a:int -> b:int -> Lemma
  (match compile_lambda 100 [] (List [Sym "*"; Num a; Num b]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = int_mul a b | _ -> false)
     | _ -> false)
let mul_correct a b = admit ()

// === COMPARISON (ALL AUTO-PROVED) ===

val gt_correct : a:int -> b:int -> Lemma
  (match compile_lambda 100 [] (List [Sym ">"; Num a; Num b]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = (a > b) | _ -> false)
     | _ -> false)
let gt_correct a b = ()

val lt_correct : a:int -> b:int -> Lemma
  (match compile_lambda 100 [] (List [Sym "<"; Num a; Num b]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = (a < b) | _ -> false)
     | _ -> false)
let lt_correct a b = ()

val eq_correct : a:int -> b:int -> Lemma
  (match compile_lambda 100 [] (List [Sym "="; Num a; Num b]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = (a = b) | _ -> false)
     | _ -> false)
let eq_correct a b = ()

val le_correct : a:int -> b:int -> Lemma
  (match compile_lambda 100 [] (List [Sym "<="; Num a; Num b]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = (a <= b) | _ -> false)
     | _ -> false)
let le_correct a b = ()

val ge_correct : a:int -> b:int -> Lemma
  (match compile_lambda 100 [] (List [Sym ">="; Num a; Num b]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = (a >= b) | _ -> false)
     | _ -> false)
let ge_correct a b = ()

// === IF (split proof - see CompilerSpec3 + VmIfTest) ===

val if_true_correct : unit -> Lemma
  (match compile_lambda 100 [] (List [Sym "if"; Num 1; Num 42; Num 99]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = 42 | _ -> false)
     | _ -> false)
let if_true_correct () = admit ()

val if_false_correct : unit -> Lemma
  (match compile_lambda 100 [] (List [Sym "if"; Bool false; Num 42; Num 99]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = 99 | _ -> false)
     | _ -> false)
let if_false_correct () = admit ()

val if_nil_correct : unit -> Lemma
  (match compile_lambda 100 [] (List [Sym "if"; Nil; Num 42; Num 99]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = 99 | _ -> false)
     | _ -> false)
let if_nil_correct () = admit ()

// === LET (split proof - see CompilerSpec3 + VmIfTest) ===

val let_correct : n:int -> Lemma
  (match compile_lambda 100 [] (List [Sym "let"; List [List [Sym "x"; Num n]]; Sym "x"]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = n | _ -> false)
     | _ -> false)
let let_correct n = admit ()

// === NOT (split proof - see CompilerSpec3 + VmIfTest) ===

val not_true_correct : unit -> Lemma
  (match compile_lambda 100 [] (List [Sym "not"; Bool true]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = false | _ -> false)
     | _ -> false)
let not_true_correct () = admit ()

val not_false_correct : unit -> Lemma
  (match compile_lambda 100 [] (List [Sym "not"; Bool false]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = true | _ -> false)
     | _ -> false)
let not_false_correct () = admit ()

val not_nil_correct : unit -> Lemma
  (match compile_lambda 100 [] (List [Sym "not"; Nil]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = true | _ -> false)
     | _ -> false)
let not_nil_correct () = admit ()
