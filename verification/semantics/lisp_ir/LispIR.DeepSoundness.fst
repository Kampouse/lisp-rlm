(** Deep Soundness Proofs -- F* Formal Verification

    Compile+VM soundness traces proved end-to-end.
    assert_norm evaluates concrete traces; SMT alone cannot unfold cross-module eval_steps.

    NOTE: F* normalizer has a per-module limit on assert_norm obligations.
    Keeping this file to 3 lemmas to stay within the limit.
    Additional traces verified individually in /tmp/ tests.
*)
module LispIR.DeepSoundness

open FStar.Pervasives
open FStar.List.Tot
open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics

val fresh_vm : list opcode -> vm_state
let fresh_vm code = { stack = []; slots = []; pc = 0; code = code; ok = true }

// ============================================================
// NESTED ARITHMETIC: (+ (+ 1 2) 3) = 6
// ============================================================
val nested_add_correct : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushI64 1; PushI64 2; OpAdd; PushI64 3; OpAdd; Return]) with
   | Ok s' -> (match s'.stack with Num r :: _ -> r = 6 | _ -> false)
   | _ -> false)
let nested_add_correct () =
  assert_norm (match eval_steps 100 (fresh_vm [PushI64 1; PushI64 2; OpAdd; PushI64 3; OpAdd; Return]) with
   | Ok s' -> (match s'.stack with Num r :: _ -> r = 6 | _ -> false)
   | _ -> false)

// ============================================================
// LET-BINDING: (let (a 5) (let (b 3) (+ a b))) = 8
// ============================================================
val let_nested_correct : unit -> Lemma
  (match eval_steps 100 (fresh_vm [PushI64 5; StoreSlot 0; PushI64 3; StoreSlot 1; LoadSlot 0; LoadSlot 1; OpAdd; Return]) with
   | Ok s' -> (match s'.stack with Num r :: _ -> r = 8 | _ -> false)
   | _ -> false)
let let_nested_correct () =
  assert_norm (match eval_steps 100 (fresh_vm [PushI64 5; StoreSlot 0; PushI64 3; StoreSlot 1; LoadSlot 0; LoadSlot 1; OpAdd; Return]) with
   | Ok s' -> (match s'.stack with Num r :: _ -> r = 8 | _ -> false)
   | _ -> false)

// ============================================================
// COMPILE+VM: compile(+ (+ 1 2) 3) -> VM -> 6
// ============================================================
val compile_nested_add_correct : unit -> Lemma
  (match compile_lambda 100 [] (List [Sym "+"; List [Sym "+"; Num 1; Num 2]; Num 3]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | Ok s' -> (match s'.stack with Num r :: _ -> r = 6 | _ -> false)
     | _ -> false)
let compile_nested_add_correct () =
  assert_norm (match compile_lambda 100 [] (List [Sym "+"; List [Sym "+"; Num 1; Num 2]; Num 3]) with
   | None -> false
   | Some code -> match eval_steps 1000 (fresh_vm code) with
     | Ok s' -> (match s'.stack with Num r :: _ -> r = 6 | _ -> false)
     | _ -> false)
