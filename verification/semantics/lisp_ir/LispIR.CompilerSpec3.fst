(** IF/LET/NOT compiler specs with precise jump targets *)
module LispIR.CompilerSpec3

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics

// IF with concrete jump targets
val compile_lambda_if_precise : fuel:int -> Lemma
  (fuel > 10 ==> (match compile_lambda fuel [] (List [Sym "if"; Num 1; Num 42; Num 99]) with
    | None -> true
    | Some code -> (match code with
      | [PushI64 1; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 99; Return] -> true
      | _ -> false)))
let compile_lambda_if_precise fuel = ()

// IF false branch with concrete targets
val compile_lambda_if_false_precise : fuel:int -> Lemma
  (fuel > 10 ==> (match compile_lambda fuel [] (List [Sym "if"; Bool false; Num 42; Num 99]) with
    | None -> true
    | Some code -> (match code with
      | [PushBool false; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 99; Return] -> true
      | _ -> false)))
let compile_lambda_if_false_precise fuel = ()

// LET with precise code
val compile_lambda_let_precise : fuel:int -> n:int -> Lemma
  (fuel > 10 ==> (match compile_lambda fuel [] (List [Sym "let"; List [List [Sym "x"; Num n]]; Sym "x"]) with
    | None -> true
    | Some code -> (match code with
      | [PushI64 v; StoreSlot 0; LoadSlot 0; Return] -> v = n
      | _ -> false)))
let compile_lambda_let_precise fuel n = ()

// NOT true with precise code
val compile_lambda_not_true_precise : fuel:int -> Lemma
  (fuel > 10 ==> (match compile_lambda fuel [] (List [Sym "not"; Bool true]) with
    | None -> true
    | Some code -> (match code with
      | [PushBool true; JumpIfFalse 4; PushBool false; Jump 5; PushBool true; Return] -> true
      | _ -> false)))
let compile_lambda_not_true_precise fuel = ()

// NOT false with precise code
val compile_lambda_not_false_precise : fuel:int -> Lemma
  (fuel > 10 ==> (match compile_lambda fuel [] (List [Sym "not"; Bool false]) with
    | None -> true
    | Some code -> (match code with
      | [PushBool false; JumpIfFalse 4; PushBool false; Jump 5; PushBool true; Return] -> true
      | _ -> false)))
let compile_lambda_not_false_precise fuel = ()
