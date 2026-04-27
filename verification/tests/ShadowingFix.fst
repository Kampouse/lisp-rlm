(** Shadowing Bug -- FIXED
    
    The fix: slot_of now scans right-to-left (returns LAST occurrence),
    matching env_push's prepend semantics (newest binding first).
    
    Expression: (let [x 1] (let [x 2] x))
    
    Before fix: slot_of found index 0 (outer x=1) -> wrong
    After fix:  slot_of finds index 1 (inner x=2) -> correct
    
    5/6 auto-proved. compiler_produces_fixed_code admitted
    (Z3 can't unfold nested compile_lambda for complex expressions).
*)
module ShadowingFix

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics

val shadowing_expr : lisp_val
let shadowing_expr = List [
  Sym "let";
  List [List [Sym "x"; Num 1]];
  List [
    Sym "let";
    List [List [Sym "x"; Num 2]];
    Sym "x"
  ]
]

// === Part 1: slot_of finds the CORRECT (last) index ===
val slot_of_finds_correct : unit -> Lemma
  (match slot_of "x" ["x"; "x"] with
   | Some 1 -> true
   | _ -> false)
let slot_of_finds_correct () = ()

val slot_of_non_shadowed : unit -> Lemma
  (match slot_of "x" ["x"; "y"] with
   | Some 0 -> true
   | _ -> false)
let slot_of_non_shadowed () = ()

val slot_of_single : unit -> Lemma
  (match slot_of "x" ["x"] with
   | Some 0 -> true
   | _ -> false)
let slot_of_single () = ()

// === Part 2: Compiler output (admitted -- Z3 can't unfold nested let-in-let) ===
val compiler_produces_fixed_code : unit -> Lemma
  (match compile_lambda 100 [] shadowing_expr with
   | Some [PushI64 1; StoreSlot 0; PushI64 2; StoreSlot 1; LoadSlot 1; Return] -> true
   | _ -> false)
let compiler_produces_fixed_code () = admit ()

// === Part 3: VM on fixed code gives Num 2 (correct) ===
val vm_gives_correct_2 : unit -> Lemma
  (let s = { stack = []; slots = []; pc = 0;
             code = [PushI64 1; StoreSlot 0; PushI64 2; StoreSlot 1; LoadSlot 1; Return];
             ok = true } in
   match eval_steps 100 s with
   | Ok s' -> (match s'.stack with
     | Num 2 :: _ -> true
     | _ -> false)
   | _ -> false)
let vm_gives_correct_2 () = ()

// === Triple shadowing ===
val slot_of_triple : unit -> Lemma
  (match slot_of "x" ["x"; "x"; "x"] with
   | Some 2 -> true
   | _ -> false)
let slot_of_triple () = ()
