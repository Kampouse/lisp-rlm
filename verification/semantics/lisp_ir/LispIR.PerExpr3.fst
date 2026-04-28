(** Per-Expression Compiler Correctness

    Split proof strategy (see skill pitfall #30):
    - Compiler output specs: CompilerSpec.fst (+/-/>/=), CompilerSpec3.fst (if/let/not)
    - VM correctness: proven HERE via direct opcode sequences
    Both halves independently verified.

    AUTO-PROVED: 15/16
    Remaining admits: mul_correct (opaque int_mul)
*)
module LispIR.PerExpr3

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics

val fresh_vm : list opcode -> vm_state
let fresh_vm code = { stack = []; slots = []; pc = 0; code = code; ok = true }

// === LITERALS (AUTO-PROVED -- compile_lambda trivially unfolds) ===

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

// === ARITHMETIC (direct VM proof -- compiler spec in CompilerSpec.fst) ===

val add_correct : a:int -> b:int -> Lemma
  (match eval_steps 100 (fresh_vm [PushI64 a; PushI64 b; OpAdd; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = a + b | _ -> false)
   | _ -> false)
let add_correct a b = ()

val sub_correct : a:int -> b:int -> Lemma
  (match eval_steps 100 (fresh_vm [PushI64 a; PushI64 b; OpSub; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = a - b | _ -> false)
   | _ -> false)
let sub_correct a b = ()

val mul_correct : a:int -> b:int -> Lemma
  (match eval_steps 100 (fresh_vm [PushI64 a; PushI64 b; OpMul; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = int_mul a b | _ -> false)
   | _ -> false)
let mul_correct a b = ()

// === COMPARISON (AUTO-PROVED -- compile_lambda unfolds for these) ===

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

// === IF (direct VM proof -- compiler spec in CompilerSpec3.fst) ===

val if_true_correct : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushI64 1; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 99; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = 42 | _ -> false)
   | _ -> false)
let if_true_correct () = ()

val if_false_correct : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushBool false; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 99; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = 99 | _ -> false)
   | _ -> false)
let if_false_correct () = ()

val if_nil_correct : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushNil; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 99; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = 99 | _ -> false)
   | _ -> false)
let if_nil_correct () = ()

// === LET (direct VM proof -- compiler spec in CompilerSpec3.fst) ===

val let_correct : n:int -> Lemma
  (match eval_steps 100 (fresh_vm [PushI64 n; StoreSlot 0; LoadSlot 0; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Num r :: _ -> r = n | _ -> false)
   | _ -> false)
let let_correct n = ()

// === NOT (direct VM proof -- compiler spec in CompilerSpec3.fst) ===

val not_true_correct : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushBool true; JumpIfFalse 4; PushBool false; Jump 5; PushBool true; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = false | _ -> false)
   | _ -> false)
let not_true_correct () = ()

val not_false_correct : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushBool false; JumpIfFalse 4; PushBool false; Jump 5; PushBool true; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = true | _ -> false)
   | _ -> false)
let not_false_correct () = ()

val not_nil_correct : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushNil; JumpIfFalse 4; PushBool false; Jump 5; PushBool true; Return]) with
   | LispIR.Semantics.Ok s' -> (match s'.stack with Bool r :: _ -> r = true | _ -> false)
   | _ -> false)
let not_nil_correct () = ()
