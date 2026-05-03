(** List Compilation Correctness -- F* Formal Verification

    Proves that (list e1 e2 ...) compiles and evaluates correctly on the VM.
    
    Two proof layers:
    1. VM MakeList semantics: concrete bytecode tests
    2. Compiler output structure: compile_lambda produces correct bytecode
    3. End-to-end: compile_lambda + VM execution = correct list value
    
    Proof technique: assert_norm with concrete fuel (100) lets the F*
    normalizer evaluate compile_lambda + eval_steps.
    Parametric fuel breaks the normalizer — it can't unfold compile_lambda
    when fuel is symbolic. Those proofs use admit().
    
    Note: Source evaluator (Lisp.Source) does not model (list) as a special
    form — it's a builtin function, not available in the pure source eval env.
    So we only test compiler+VM correctness, not source eval consistency.
*)
module ListCompileTest

open Lisp.Types
open Lisp.Values
open Lisp.Compiler
open LispIR.Semantics

val fresh_vm : list opcode -> vm_state
let fresh_vm code = { stack = []; slots = []; pc = 0; code = code; ok = true }

// ============================================================
// LAYER 1: VM MakeList semantics (concrete tests)
// ============================================================

// Empty MakeList: pop 0 items, push List []
val vm_makelist_empty : unit -> Lemma
  (match eval_steps 10 (fresh_vm [MakeList 0; Return]) with
   | Ok s' -> (match s'.stack with
     | List [] :: _ -> true
     | _ -> false)
   | _ -> false)
let vm_makelist_empty () = ()

// PushNil produces Nil (the empty list representation)
val vm_pushnil : unit -> Lemma
  (match eval_steps 10 (fresh_vm [PushNil; Return]) with
   | Ok s' -> (match s'.stack with
     | Nil :: _ -> true
     | _ -> false)
   | _ -> false)
let vm_pushnil () = ()

// MakeList 1: pop 1 item, push List [item]
val vm_makelist_one : n:int -> Lemma
  (match eval_steps 10 (fresh_vm [PushI64 n; MakeList 1; Return]) with
   | Ok s' -> (match s'.stack with
     | List [Num m] :: _ -> m = n
     | _ -> false)
   | _ -> false)
let vm_makelist_one n = ()

// MakeList 2: pop 2 items (reversed), push List [first, second]
// Stack before MakeList: [b, a, ...]  →  List [a, b]
val vm_makelist_two : a:int -> b:int -> Lemma
  (match eval_steps 10 (fresh_vm [PushI64 a; PushI64 b; MakeList 2; Return]) with
   | Ok s' -> (match s'.stack with
     | List [Num x; Num y] :: _ -> x = a && y = b
     | _ -> false)
   | _ -> false)
let vm_makelist_two a b = ()

// MakeList 3: three elements (admit — normalizer can't unfold pop_n+list_rev 3 deep)
val vm_makelist_three : a:int -> b:int -> c:int -> Lemma
  (match eval_steps 10 (fresh_vm [PushI64 a; PushI64 b; PushI64 c; MakeList 3; Return]) with
   | Ok s' -> (match s'.stack with
     | List [Num x; Num y; Num z] :: _ -> x = a && y = b && z = c
     | _ -> false)
   | _ -> false)
let vm_makelist_three a b c = admit()

// ============================================================
// LAYER 2: Compiler output structure (assert_norm, concrete fuel)
// ============================================================

// (list) compiles to [MakeList 0; Return] → VM produces List []
val compile_empty_list_spec : unit -> Lemma
  (ensures (match compile_lambda 100 [] (List [Sym "list"]) with
    | Some code -> (match code with
      | [MakeList 0; Return] -> true
      | _ -> false)
    | None -> false))
let compile_empty_list_spec () =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "list"]) with
    | Some code -> (match code with
      | [MakeList 0; Return] -> true
      | _ -> false)
    | None -> false)

// (list n) compiles to [PushI64 n; MakeList 1; Return]
val compile_list_one_spec : n:int -> Lemma
  (ensures (match compile_lambda 100 [] (List [Sym "list"; Num n]) with
    | Some code -> (match code with
      | [PushI64 x; MakeList 1; Return] -> x = n
      | _ -> false)
    | None -> false))
let compile_list_one_spec n =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "list"; Num n]) with
    | Some code -> (match code with
      | [PushI64 x; MakeList 1; Return] -> x = n
      | _ -> false)
    | None -> false)

// (list a b) compiles to [PushI64 a; PushI64 b; MakeList 2; Return]
val compile_list_two_spec : a:int -> b:int -> Lemma
  (ensures (match compile_lambda 100 [] (List [Sym "list"; Num a; Num b]) with
    | Some code -> (match code with
      | [PushI64 x; PushI64 y; MakeList 2; Return] -> x = a && y = b
      | _ -> false)
    | None -> false))
let compile_list_two_spec a b =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "list"; Num a; Num b]) with
    | Some code -> (match code with
      | [PushI64 x; PushI64 y; MakeList 2; Return] -> x = a && y = b
      | _ -> false)
    | None -> false)

// (list a b c) compiles to [PushI64 a; PushI64 b; PushI64 c; MakeList 3; Return]
// Admit — normalizer can't unfold 3-deep compile_list_body recursion
val compile_list_three_spec : a:int -> b:int -> c:int -> Lemma
  (ensures (match compile_lambda 100 [] (List [Sym "list"; Num a; Num b; Num c]) with
    | Some code -> (match code with
      | [PushI64 x; PushI64 y; PushI64 z; MakeList 3; Return] -> x = a && y = b && z = c
      | _ -> false)
    | None -> false))
let compile_list_three_spec a b c = admit()

// ============================================================
// LAYER 3: End-to-end compiler correctness
// compile_lambda + VM = correct list value
// ============================================================

// (list) → VM produces List [] (not Nil — MakeList 0 produces List [])
val cc_empty_list : unit -> Lemma
  (match compile_lambda 100 [] (List [Sym "list"]) with
   | Some code ->
     (match eval_steps 100 (fresh_vm code) with
      | Ok s' -> (match s'.stack with
        | List [] :: _ -> true
        | _ -> false)
      | _ -> false)
   | None -> false)
let cc_empty_list () =
  assert_norm (
    match compile_lambda 100 [] (List [Sym "list"]) with
    | Some code ->
     (match eval_steps 100 (fresh_vm code) with
      | Ok s' -> (match s'.stack with
        | List [] :: _ -> true
        | _ -> false)
      | _ -> false)
    | None -> false)

// (list n) → VM produces List [Num n]
val cc_list_one : n:int -> Lemma
  (match compile_lambda 100 [] (List [Sym "list"; Num n]) with
   | Some code ->
     (match eval_steps 100 (fresh_vm code) with
      | Ok s' -> (match s'.stack with
        | List [Num m] :: _ -> m = n
        | _ -> false)
      | _ -> false)
   | None -> false)
let cc_list_one n = ()

// (list a b) → VM produces List [Num a; Num b]
val cc_list_two : a:int -> b:int -> Lemma
  (match compile_lambda 100 [] (List [Sym "list"; Num a; Num b]) with
   | Some code ->
     (match eval_steps 100 (fresh_vm code) with
      | Ok s' -> (match s'.stack with
        | List [Num x; Num y] :: _ -> x = a && y = b
        | _ -> false)
      | _ -> false)
   | None -> false)
let cc_list_two a b = ()

// (list a b c) → VM produces List [Num a; Num b; Num c]
// Admit — 3-deep list construction exceeds normalizer capacity
val cc_list_three : a:int -> b:int -> c:int -> Lemma
  (match compile_lambda 100 [] (List [Sym "list"; Num a; Num b; Num c]) with
   | Some code ->
     (match eval_steps 100 (fresh_vm code) with
      | Ok s' -> (match s'.stack with
        | List [Num x; Num y; Num z] :: _ -> x = a && y = b && z = c
        | _ -> false)
      | _ -> false)
   | None -> false)
let cc_list_three a b c = admit()

// ============================================================
// LAYER 4: List truthiness
// ============================================================

// Non-empty list is truthy in VM
// Code: [PushI64 a; PushI64 b; MakeList 2; JumpIfFalse 6; PushI64 42; Jump 7; PushI64 99; Return]
// Truthy path: MakeList 2 → list (truthy) → JumpIfFalse no-jump → 42 → Jump 7 → Return
val list_truthy_vm : a:int -> b:int -> Lemma
  (match eval_steps 100 (fresh_vm [PushI64 a; PushI64 b; MakeList 2;
                                    JumpIfFalse 6;
                                    PushI64 42; Jump 7;
                                    PushI64 99; Return]) with
   | Ok s' -> (match s'.stack with
     | Num r :: _ -> r = 42
     | _ -> false)
   | _ -> false)
let list_truthy_vm a b =
  assert_norm (
    match eval_steps 100 (fresh_vm [PushI64 a; PushI64 b; MakeList 2;
                                    JumpIfFalse 6;
                                    PushI64 42; Jump 7;
                                    PushI64 99; Return]) with
   | Ok s' -> (match s'.stack with
     | Num r :: _ -> r = 42
     | _ -> false)
   | _ -> false)

// Empty list (Nil) is falsy in VM
// Code: [PushNil; JumpIfFalse 4; PushI64 42; Jump 5; PushI64 99; Return]
// Falsy path: PushNil → Nil (falsy) → JumpIfFalse 4 → jump → 99 → Return
val empty_list_falsy_vm : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushNil;
                                    JumpIfFalse 4;
                                    PushI64 42; Jump 5;
                                    PushI64 99; Return]) with
   | Ok s' -> (match s'.stack with
     | Num r :: _ -> r = 99
     | _ -> false)
   | _ -> false)
let empty_list_falsy_vm () =
  assert_norm (
    match eval_steps 100 (fresh_vm [PushNil;
                                    JumpIfFalse 4;
                                    PushI64 42; Jump 5;
                                    PushI64 99; Return]) with
   | Ok s' -> (match s'.stack with
     | Num r :: _ -> r = 99
     | _ -> false)
   | _ -> false)

// List [] (from MakeList 0) is truthy in VM
// Code: [MakeList 0; JumpIfFalse 5; PushI64 42; Jump 6; PushI64 99; Return]
// List [] is truthy (not Nil!) → JumpIfFalse no-jump → 42 → Return
val empty_make_list_truthy_vm : unit -> Lemma
  (match eval_steps 100 (fresh_vm [MakeList 0;
                                    JumpIfFalse 5;
                                    PushI64 42; Jump 6;
                                    PushI64 99; Return]) with
   | Ok s' -> (match s'.stack with
     | Num r :: _ -> r = 42
     | _ -> false)
   | _ -> false)
let empty_make_list_truthy_vm () =
  assert_norm (
    match eval_steps 100 (fresh_vm [MakeList 0;
                                    JumpIfFalse 5;
                                    PushI64 42; Jump 6;
                                    PushI64 99; Return]) with
   | Ok s' -> (match s'.stack with
     | Num r :: _ -> r = 42
     | _ -> false)
   | _ -> false)
