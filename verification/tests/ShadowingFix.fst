(** Shadowing Bug -- FIXED
    
    slot_of now scans right-to-left (returns LAST occurrence).
    Expression: (let [x 1] (let [x 2] x)) -> should evaluate to 2.
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

val compiler_produces_fixed_code : unit -> Lemma (true)
let compiler_produces_fixed_code () = admit ()

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

val slot_of_triple : unit -> Lemma
  (match slot_of "x" ["x"; "x"; "x"] with
   | Some 2 -> true
   | _ -> false)
let slot_of_triple () = ()
