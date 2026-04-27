(** Compiler Output Specification Lemmas
    
    Test what F* can auto-prove about the compiler's output structure.
    The compiler type has ffloat (non-eqtype), so we can't use = on it.
    But we CAN match the result and prove properties about specific fields.
*)
module LispIR.CompilerSpec

open Lisp.Types
open Lisp.Values
open Lisp.Source
open Lisp.Compiler
open LispIR.Semantics

// === Can F* prove compile produces Some? ===

val compile_add_succeeds : fuel:int -> a:int -> b:int -> Lemma
  (fuel > 5 ==> (match compile fuel (List [Sym "+"; Num a; Num b]) (init_compiler []) with
    | None -> false
    | Some _ -> true))
let compile_add_succeeds fuel a b = ()

// === Can F* prove slot_map is preserved? ===

val compile_add_preserves_slots : fuel:int -> a:int -> b:int -> Lemma
  (fuel > 5 ==> (match compile fuel (List [Sym "+"; Num a; Num b]) (init_compiler []) with
    | None -> true
    | Some c -> c.slot_map = []))
let compile_add_preserves_slots fuel a b = ()

// === Can F* prove code length? ===

val compile_add_code_len : fuel:int -> a:int -> b:int -> Lemma
  (fuel > 5 ==> (match compile fuel (List [Sym "+"; Num a; Num b]) (init_compiler []) with
    | None -> true
    | Some c -> list_len_int c.code = 3))
let compile_add_code_len fuel a b = ()

// === Per-opcode inspection: can we match individual opcodes? ===
// The code list should be [PushI64 a; PushI64 b; OpAdd]
// Check first opcode

val compile_add_first_op : fuel:int -> a:int -> b:int -> Lemma
  (fuel > 5 ==> (match compile fuel (List [Sym "+"; Num a; Num b]) (init_compiler []) with
    | None -> true
    | Some c -> (match c.code with
      | PushI64 n :: _ -> n = a
      | _ -> false)))
let compile_add_first_op fuel a b = ()

val compile_add_second_op : fuel:int -> a:int -> b:int -> Lemma
  (fuel > 5 ==> (match compile fuel (List [Sym "+"; Num a; Num b]) (init_compiler []) with
    | None -> true
    | Some c -> (match c.code with
      | _ :: PushI64 n :: _ -> n = b
      | _ -> false)))
let compile_add_second_op fuel a b = ()

val compile_add_third_op : fuel:int -> a:int -> b:int -> Lemma
  (fuel > 5 ==> (match compile fuel (List [Sym "+"; Num a; Num b]) (init_compiler []) with
    | None -> true
    | Some c -> (match c.code with
      | _ :: _ :: OpAdd :: _ -> true
      | _ -> false)))
let compile_add_third_op fuel a b = ()

// === Full chain: all three opcodes correct ===

val compile_add_full_spec : fuel:int -> a:int -> b:int -> Lemma
  (fuel > 5 ==> (match compile fuel (List [Sym "+"; Num a; Num b]) (init_compiler []) with
    | None -> true
    | Some c -> (match c.code with
      | [PushI64 x; PushI64 y; OpAdd] -> x = a && y = b
      | _ -> false)))
let compile_add_full_spec fuel a b = ()

// === compile_lambda wraps with Return ===

val compile_lambda_add_spec : fuel:int -> a:int -> b:int -> Lemma
  (fuel > 5 ==> (match compile_lambda fuel [] (List [Sym "+"; Num a; Num b]) with
    | None -> true
    | Some code -> (match code with
      | [PushI64 x; PushI64 y; OpAdd; Return] -> x = a && y = b
      | _ -> false)))
let compile_lambda_add_spec fuel a b = ()

// === Comparison specs ===

val compile_lambda_gt_spec : fuel:int -> a:int -> b:int -> Lemma
  (fuel > 5 ==> (match compile_lambda fuel [] (List [Sym ">"; Num a; Num b]) with
    | None -> true
    | Some code -> (match code with
      | [PushI64 x; PushI64 y; OpGt; Return] -> x = a && y = b
      | _ -> false)))
let compile_lambda_gt_spec fuel a b = ()

val compile_lambda_eq_spec : fuel:int -> a:int -> b:int -> Lemma
  (fuel > 5 ==> (match compile_lambda fuel [] (List [Sym "="; Num a; Num b]) with
    | None -> true
    | Some code -> (match code with
      | [PushI64 x; PushI64 y; OpEq; Return] -> x = a && y = b
      | _ -> false)))
let compile_lambda_eq_spec fuel a b = ()
