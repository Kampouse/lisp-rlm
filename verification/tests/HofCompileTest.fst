(** HOF Compilation Correctness -- F* Formal Verification

    Proves that map/filter/reduce compile to the correct fused opcodes.
    
    Three proof layers:
    1. MAP: (map f (list n)) compiles to [...; MapOp 0; Return]
    2. FILTER: (filter f (list n)) compiles to [...; FilterOp 0; Return]
    3. REDUCE: (reduce f init (list n)) compiles to [...; ReduceOp 0; Return]
*)
module HofCompileTest

open Lisp.Types
open Lisp.Values
open Lisp.Compiler

// ============================================================
// LAYER 1: Map compiles to MapOp
// (map f (list n)) with f at slot 0
// ============================================================

val compile_map_spec : fuel:int -> n:int -> Lemma
  (fuel > 5 ==>
   (match compile_lambda fuel ["f"; "x"] (List [Sym "map"; Sym "f"; List [Sym "list"; Num n]]) with
    | Some code ->
      (match code with
       | [PushI64 m; MakeList 1; MapOp s; Return] -> m = n && s = 0
       | _ -> false)
    | None -> false))
let compile_map_spec fuel n = ()

// ============================================================
// LAYER 2: Filter compiles to FilterOp
// (filter f (list n)) with f at slot 0
// ============================================================

val compile_filter_spec : fuel:int -> n:int -> Lemma
  (fuel > 5 ==>
   (match compile_lambda fuel ["f"; "x"] (List [Sym "filter"; Sym "f"; List [Sym "list"; Num n]]) with
    | Some code ->
      (match code with
       | [PushI64 m; MakeList 1; FilterOp s; Return] -> m = n && s = 0
       | _ -> false)
    | None -> false))
let compile_filter_spec fuel n = ()

// ============================================================
// LAYER 3: Reduce compiles to ReduceOp
// (reduce f init (list n)) with f at slot 0
// ============================================================

val compile_reduce_spec : fuel:int -> init_val:int -> n:int -> Lemma
  (fuel > 5 ==>
   (match compile_lambda fuel ["f"; "acc"; "x"]
       (List [Sym "reduce"; Sym "f"; Num init_val; List [Sym "list"; Num n]]) with
    | Some code ->
      (match code with
       | [PushI64 i; PushI64 m; MakeList 1; ReduceOp s; Return] ->
         i = init_val && m = n && s = 0
       | _ -> false)
    | None -> false))
let compile_reduce_spec fuel init_val n = ()
