(** Stack Height Invariant -- F* Proof
    
    Proves that each opcode changes the value stack by a known delta.
    This catches VM bugs where an opcode silently leaves extra values
    or underflows.
*)
module StackHeight

open Lisp.Types
open Lisp.Values
open Lisp.Compiler
open LispIR.Semantics

val stack_len : vm_state -> Tot nat
let stack_len s = list_len_nat s.stack

// --- PushI64: +1 ---
val push_i64_delta : n:int -> s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op (PushI64 n) s with
   | Ok s' -> stack_len s' = stack_len s + 1
   | _ -> true))
let push_i64_delta n s = ()

// --- PushNil: +1 ---
val push_nil_delta : s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op PushNil s with
   | Ok s' -> stack_len s' = stack_len s + 1
   | _ -> true))
let push_nil_delta s = ()

// --- PushBool: +1 ---
val push_bool_delta : b:bool -> s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op (PushBool b) s with
   | Ok s' -> stack_len s' = stack_len s + 1
   | _ -> true))
let push_bool_delta b s = ()

// --- PushStr: +1 ---
val push_str_delta : str:string -> s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op (PushStr str) s with
   | Ok s' -> stack_len s' = stack_len s + 1
   | _ -> true))
let push_str_delta str s = ()

// --- PushFloat: +1 ---
val push_float_delta : f:ffloat -> s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op (PushFloat f) s with
   | Ok s' -> stack_len s' = stack_len s + 1
   | _ -> true))
let push_float_delta f s = ()

// --- Dup: +1 (copies top of stack) ---
val dup_delta : s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op Dup s with
   | Ok s' -> stack_len s' = stack_len s + 1
   | _ -> true))
let dup_delta s = ()

// --- Pop: -1 ---
val pop_delta : s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op Pop s with
   | Ok s' -> stack_len s' = stack_len s - 1
   | _ -> true))
let pop_delta s = ()

// --- StoreSlot: -1 (pops value from stack) ---
val store_slot_delta : idx:int -> s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op (StoreSlot idx) s with
   | Ok s' -> stack_len s' = stack_len s - 1
   | _ -> true))
let store_slot_delta idx s = ()

// --- LoadSlot: +1 (reads slot, pushes to stack) ---
val load_slot_delta : idx:int -> s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op (LoadSlot idx) s with
   | Ok s' -> stack_len s' = stack_len s + 1
   | _ -> true))
let load_slot_delta idx s = ()

// --- OpAdd: -1 (pops 2, pushes 1) ---
val op_add_delta : s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op OpAdd s with
   | Ok s' -> stack_len s' = stack_len s - 1
   | _ -> true))
let op_add_delta s = ()

// --- OpSub: -1 ---
val op_sub_delta : s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op OpSub s with
   | Ok s' -> stack_len s' = stack_len s - 1
   | _ -> true))
let op_sub_delta s = ()

// --- OpGt: -1 ---
val op_gt_delta : s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op OpGt s with
   | Ok s' -> stack_len s' = stack_len s - 1
   | _ -> true))
let op_gt_delta s = ()

// --- OpEq: -1 ---
val op_eq_delta : s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op OpEq s with
   | Ok s' -> stack_len s' = stack_len s - 1
   | _ -> true))
let op_eq_delta s = ()

// --- OpLt: -1 ---
val op_lt_delta : s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op OpLt s with
   | Ok s' -> stack_len s' = stack_len s - 1
   | _ -> true))
let op_lt_delta s = ()

// --- JumpIfFalse: -1 (pops test value) ---
val jump_if_false_delta : addr:nat -> s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op (JumpIfFalse addr) s with
   | Ok s' -> stack_len s' = stack_len s - 1
   | _ -> true))
let jump_if_false_delta addr s = ()

// --- JumpIfTrue: -1 (pops test value) ---
val jump_if_true_delta : addr:nat -> s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op (JumpIfTrue addr) s with
   | Ok s' -> stack_len s' = stack_len s - 1
   | _ -> true))
let jump_if_true_delta addr s = ()

// --- Jump: 0 (no stack change) ---
val jump_delta : addr:nat -> s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op (Jump addr) s with
   | Ok s' -> stack_len s' = stack_len s
   | _ -> true))
let jump_delta addr s = ()

// --- DictGet: -1 (pops key+map, pushes result) ---
val dict_get_delta : s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op DictGet s with
   | Ok s' -> stack_len s' = stack_len s - 1
   | _ -> true))
let dict_get_delta s = ()

// --- SlotGtImm: +1 (pushes Bool) ---
val slot_gt_imm_delta : p:(nat * int) -> s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op (SlotGtImm p) s with
   | Ok s' -> stack_len s' = stack_len s + 1
   | _ -> true))
let slot_gt_imm_delta p s = ()

// --- SlotAddImm: +1 (pushes result) ---
val slot_add_imm_delta : p:(nat * int) -> s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op (SlotAddImm p) s with
   | Ok s' -> stack_len s' = stack_len s + 1
   | _ -> true))
let slot_add_imm_delta p s = ()

// --- Return: stack becomes [retval] (len = 1) ---
val return_delta : s:vm_state -> Lemma
  (requires s.ok)
  (ensures (match eval_op Return s with
   | Ok s' -> stack_len s' = 1
   | _ -> true))
let return_delta s = ()
