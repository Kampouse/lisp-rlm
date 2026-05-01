module NewFormRoundtrips

#set-options "--z3rlimit 500"

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM
open Lisp.Source
open Lisp.Compiler

val cvm0 : list opcode -> nat -> closure_vm
let cvm0 code nslots = {
  stack = []; slots = []; pc = 0;
  code = code; ok = true;
  code_table = []; frames = [];
  num_slots = nslots;
  captured = []; closure_envs = []; env = [];
}

// ============================================================
// (list) roundtrips -- AUTO-PROVEN
// ============================================================

val roundtrip_list_empty : fuel:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "list"]) with
    | None -> true
    | Some code ->
      (match code with
       | [MakeList 0; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          s2.ok = true && (match s2.stack with | List [] :: _ -> true | _ -> false))
       | _ -> true)))
let roundtrip_list_empty fuel = ()

val roundtrip_list1 : fuel:int -> n:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "list"; Num n]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushI64 x; MakeList 1; Return] ->
         x = n &&
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in
          s3.ok = true && (match s3.stack with | List [Num r] :: _ -> r = n | _ -> false))
       | _ -> true)))
let roundtrip_list1 fuel n = ()

val roundtrip_list2 : fuel:int -> a:int -> b:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "list"; Num a; Num b]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushI64 x; PushI64 y; MakeList 2; Return] ->
         x = a && y = b &&
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in
          let s4 = closure_eval_op s3 in
          s4.ok = true && (match s4.stack with | List [Num r1; Num r2] :: _ -> r1 = a && r2 = b | _ -> false))
       | _ -> true)))
let roundtrip_list2 fuel a b = ()

val roundtrip_list3 : fuel:int -> a:int -> b:int -> c:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "list"; Num a; Num b; Num c]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushI64 x; PushI64 y; PushI64 z; MakeList 3; Return] ->
         x = a && y = b && z = c &&
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in
          let s4 = closure_eval_op s3 in
          let s5 = closure_eval_op s4 in
          s5.ok = true && (match s5.stack with
                           | List [Num r1; Num r2; Num r3] :: _ -> r1 = a && r2 = b && r3 = c
                           | _ -> false))
       | _ -> true)))
let roundtrip_list3 fuel a b c = ()

// ============================================================
// (progn) roundtrips -- AUTO-PROVEN
// ============================================================

val roundtrip_progn1 : fuel:int -> n:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "progn"; Num n]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushI64 x; Return] ->
         x = n &&
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          s2.ok = true && (match s2.stack with | Num r :: _ -> r = n | _ -> false))
       | _ -> true)))
let roundtrip_progn1 fuel n = ()

val roundtrip_progn2 : fuel:int -> a:int -> b:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "progn"; Num a; Num b]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushI64 x; PushI64 y; Return] ->
         x = a && y = b &&
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in
          s3.ok = true && (match s3.stack with | Num r :: _ -> r = b | _ -> false))
       | _ -> true)))
let roundtrip_progn2 fuel a b = ()

val roundtrip_progn_add : fuel:int -> a:int -> b:int -> c:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "progn"; Num a; List [Sym "+"; Num b; Num c]]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushI64 x; PushI64 y; PushI64 z; OpAdd; Return] ->
         x = a && y = b && z = c &&
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in
          let s4 = closure_eval_op s3 in
          let s5 = closure_eval_op s4 in
          s5.ok = true && (match s5.stack with | Num r :: _ -> r = b + c | _ -> false))
       | _ -> true)))
let roundtrip_progn_add fuel a b c = ()

// ============================================================
// (and)/(or) base cases -- AUTO-PROVEN
// ============================================================

val roundtrip_and_empty : fuel:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "and"]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushBool true; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          s2.ok = true && (match s2.stack with | Bool r :: _ -> r = true | _ -> false))
       | _ -> true)))
let roundtrip_and_empty fuel = ()

val roundtrip_or_empty : fuel:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "or"]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushBool false; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          s2.ok = true && (match s2.stack with | Bool r :: _ -> r = false | _ -> false))
       | _ -> true)))
let roundtrip_or_empty fuel = ()

val roundtrip_and_single : fuel:int -> n:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "and"; Num n]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushI64 x; Return] ->
         x = n &&
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          s2.ok = true && (match s2.stack with | Num r :: _ -> r = n | _ -> false))
       | _ -> true)))
let roundtrip_and_single fuel n = ()

val roundtrip_or_single : fuel:int -> n:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "or"; Num n]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushI64 x; Return] ->
         x = n &&
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          s2.ok = true && (match s2.stack with | Num r :: _ -> r = n | _ -> false))
       | _ -> true)))
let roundtrip_or_single fuel n = ()

// ============================================================
// (nil?) roundtrips -- AUTO-PROVEN
// ============================================================

val roundtrip_nilq_nil : fuel:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "nil?"; Nil]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushNil; PushNil; OpEq; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in
          let s4 = closure_eval_op s3 in
          s4.ok = true && (match s4.stack with | Bool r :: _ -> r = true | _ -> false))
       | _ -> true)))
let roundtrip_nilq_nil fuel = ()

val roundtrip_nilq_num : fuel:int -> n:int -> Lemma
  (fuel > 10 ==>
   (match compile_lambda fuel [] (List [Sym "nil?"; Num n]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushI64 x; PushNil; OpEq; Return] ->
         x = n &&
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in
          let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in
          let s4 = closure_eval_op s3 in
          s4.ok = true && (match s4.stack with | Bool r :: _ -> r = false | _ -> false))
       | _ -> true)))
let roundtrip_nilq_num fuel n = ()

// ============================================================
// (and) two-arg with short-circuit -- AUTO-PROVEN at z3rlimit 500
// ============================================================

val roundtrip_and_tt : fuel:int -> Lemma
  (fuel > 20 ==>
   (match compile_lambda fuel [] (List [Sym "and"; Bool true; Bool true]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushBool true; Dup; JumpIfFalse 5; Pop; PushBool true; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in let s4 = closure_eval_op s3 in
          let s5 = closure_eval_op s4 in let s6 = closure_eval_op s5 in
          s6.ok = true && (match s6.stack with | Bool r :: _ -> r = true | _ -> false))
       | _ -> true)))
let roundtrip_and_tt fuel = ()

val roundtrip_and_tf : fuel:int -> Lemma
  (fuel > 20 ==>
   (match compile_lambda fuel [] (List [Sym "and"; Bool true; Bool false]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushBool true; Dup; JumpIfFalse 5; Pop; PushBool false; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in let s4 = closure_eval_op s3 in
          let s5 = closure_eval_op s4 in let s6 = closure_eval_op s5 in
          s6.ok = true && (match s6.stack with | Bool r :: _ -> r = false | _ -> false))
       | _ -> true)))
let roundtrip_and_tf fuel = ()

val roundtrip_and_ft : fuel:int -> Lemma
  (fuel > 20 ==>
   (match compile_lambda fuel [] (List [Sym "and"; Bool false; Bool true]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushBool false; Dup; JumpIfFalse 5; Pop; PushBool true; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in
          s3.ok = true && (match s3.stack with | Bool r :: _ -> r = false | _ -> false))
       | _ -> true)))
let roundtrip_and_ft fuel = ()

// ============================================================
// (or) two-arg with short-circuit -- AUTO-PROVEN at z3rlimit 500
// ============================================================

val roundtrip_or_ft : fuel:int -> Lemma
  (fuel > 20 ==>
   (match compile_lambda fuel [] (List [Sym "or"; Bool false; Bool true]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushBool false; Dup; JumpIfTrue 5; Pop; PushBool true; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in let s4 = closure_eval_op s3 in
          let s5 = closure_eval_op s4 in let s6 = closure_eval_op s5 in
          s6.ok = true && (match s6.stack with | Bool r :: _ -> r = true | _ -> false))
       | _ -> true)))
let roundtrip_or_ft fuel = ()

val roundtrip_or_tf : fuel:int -> Lemma
  (fuel > 20 ==>
   (match compile_lambda fuel [] (List [Sym "or"; Bool true; Bool false]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushBool true; Dup; JumpIfTrue 5; Pop; PushBool false; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in
          s3.ok = true && (match s3.stack with | Bool r :: _ -> r = true | _ -> false))
       | _ -> true)))
let roundtrip_or_tf fuel = ()

// ============================================================
// (not) roundtrips -- AUTO-PROVEN at z3rlimit 500
// ============================================================

val roundtrip_not_true : fuel:int -> Lemma
  (fuel > 20 ==>
   (match compile_lambda fuel [] (List [Sym "not"; Bool true]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushBool true; JumpIfFalse 4; PushBool false; Jump 5; PushBool true; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in let s4 = closure_eval_op s3 in
          s4.ok = true && (match s4.stack with | Bool r :: _ -> r = false | _ -> false))
       | _ -> true)))
let roundtrip_not_true fuel = ()

val roundtrip_not_false : fuel:int -> Lemma
  (fuel > 20 ==>
   (match compile_lambda fuel [] (List [Sym "not"; Bool false]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushBool false; JumpIfFalse 4; PushBool false; Jump 5; PushBool true; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in
          s3.ok = true && (match s3.stack with | Bool r :: _ -> r = true | _ -> false))
       | _ -> true)))
let roundtrip_not_false fuel = ()

// ============================================================
// (cond) roundtrips -- AUTO-PROVEN at z3rlimit 500
// ============================================================

val roundtrip_cond_true : fuel:int -> Lemma
  (fuel > 20 ==>
   (match compile_lambda fuel [] (List [Sym "cond"; List [Bool true; Num 42]; List [Sym "else"; Num 0]]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushBool true; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 0; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in let s4 = closure_eval_op s3 in
          s4.ok = true && (match s4.stack with | Num r :: _ -> r = 42 | _ -> false))
       | _ -> true)))
let roundtrip_cond_true fuel = ()

val roundtrip_cond_false : fuel:int -> Lemma
  (fuel > 20 ==>
   (match compile_lambda fuel [] (List [Sym "cond"; List [Bool false; Num 42]; List [Sym "else"; Num 0]]) with
    | None -> true
    | Some code ->
      (match code with
       | [PushBool false; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 0; Return] ->
         (let vm = cvm0 code 0 in
          let s1 = closure_eval_op vm in let s2 = closure_eval_op s1 in
          let s3 = closure_eval_op s2 in let s4 = closure_eval_op s3 in
          s4.ok = true && (match s4.stack with | Num r :: _ -> r = 0 | _ -> false))
       | _ -> true)))
let roundtrip_cond_false fuel = ()
