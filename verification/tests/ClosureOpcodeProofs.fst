(** Closure and Remaining Opcode Step Proofs
    
    All opcode handlers now proven via extracted handlers in HandlerProofs.fst.
    Dispatch glue (ok+pc through closure_eval_op) proven here.
    Full handler correctness (slots, frames, stack) proven in HandlerProofs.fst.
*)
module ClosureOpcodeProofs

#set-options "--z3rlimit 500"

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM

// ============================================================
// OpMod — polymorphic modulo
// ============================================================

val step_opmod : a:int -> b:int -> Lemma
  (b > 0 ==>
   (let vm : closure_vm = {
     stack = [Num b; Num a]; slots = []; pc = 0;
     code = [OpMod; Return]; ok = true;
     code_table = []; frames = [];
     num_slots = 0; captured = []; closure_envs = [];
   } in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 1 &&
    (match s1.stack with | Num r :: _ -> r = a % b | _ -> false)))
let step_opmod a b = ()

val step_opmod_zero : a:int -> Lemma
  (let vm : closure_vm = {
    stack = [Num 0; Num a]; slots = []; pc = 0;
    code = [OpMod; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = false)
let step_opmod_zero a = ()

// ============================================================
// LoadCaptured — index into captured values list
// ============================================================

val step_loadcaptured_0 : v:int -> Lemma
  (let vm : closure_vm = {
    stack = []; slots = []; pc = 0;
    code = [LoadCaptured 0; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = [Num v]; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with | Num r :: _ -> r = v | _ -> false))
let step_loadcaptured_0 v = ()

val step_loadcaptured_1 : a:int -> b:int -> Lemma
  (let vm : closure_vm = {
    stack = []; slots = []; pc = 0;
    code = [LoadCaptured 1; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = [Num a; Num b]; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with | Num r :: _ -> r = b | _ -> false))
let step_loadcaptured_1 a b = ()

val step_loadcaptured_str : s:string -> Lemma
  (let vm : closure_vm = {
    stack = []; slots = []; pc = 0;
    code = [LoadCaptured 0; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = [Str s]; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with | Str x :: _ -> x = s | _ -> false))
let step_loadcaptured_str s = ()

// ============================================================
// PushClosure — push closure instance onto stack
// ============================================================

val step_pushclosure_empty : unit -> Lemma
  (let vm : closure_vm = {
    stack = []; slots = []; pc = 0;
    code = [PushClosure 0; Return]; ok = true;
    code_table = [{ chunk_code = [Return]; chunk_nslots = 0; chunk_runtime_captures = [] }];
    frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with | Num inst_id :: _ -> inst_id = 0 | _ -> false))
let step_pushclosure_empty () = ()

val step_pushclosure_with_capture : v:int -> Lemma
  (let vm : closure_vm = {
    stack = []; slots = [Num v]; pc = 0;
    code = [PushClosure 0; Return]; ok = true;
    code_table = [{ chunk_code = [Return]; chunk_nslots = 0;
                    chunk_runtime_captures = [("x", 0)] }];
    frames = [];
    num_slots = 1; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with | Num inst_id :: _ -> inst_id = 0 | _ -> false))
let step_pushclosure_with_capture v = ()

// ============================================================
// CallCapturedRef — dispatch ok+pc through closure_eval_op
// Full handler correctness: callcapturedref_handler_2 in HandlerProofs.fst
// ============================================================

val step_callcapturedref_ok_pc : a:int -> b:int -> Lemma
  (let vm : closure_vm = {
    stack = [Num a; Num b]; slots = [Num 0]; pc = 0;
    code = [CallCapturedRef (0, 2); Return]; ok = true;
    code_table = [{ chunk_code = [Return]; chunk_nslots = 2; chunk_runtime_captures = [] }];
    frames = [];
    num_slots = 1; captured = [];
    closure_envs = [([], 0)];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 0)
let step_callcapturedref_ok_pc a b = ()

// ============================================================
// CallCaptured — dispatch ok+pc through closure_eval_op
// Full handler correctness: callcaptured_handler_2 in HandlerProofs.fst
// ============================================================

val step_callcaptured_ok : a:int -> b:int -> Lemma
  (let vm : closure_vm = {
    stack = [Num b; Num a; Num 0]; slots = []; pc = 0;
    code = [CallCaptured (2, 2); Return]; ok = true;
    code_table = [{ chunk_code = [Return]; chunk_nslots = 2; chunk_runtime_captures = [] }];
    frames = [];
    num_slots = 0; captured = [];
    closure_envs = [([], 0)];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 0)
let step_callcaptured_ok a b = ()

// ============================================================
// GetDefaultSlot — dict get with default fallback
// ============================================================

val step_getdefaultslot_hit : v:int -> Lemma
  (let vm : closure_vm = {
    stack = []; slots = [Dict [("k", Num v)]; Str "k"; Num 0; Num 3]; pc = 0;
    code = [GetDefaultSlot (0, 1, 2, 3); Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 4; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match list_nth s1.slots 3 with
    | Some (Num r) -> r = v
    | _ -> false))
let step_getdefaultslot_hit v = ()

val step_getdefaultslot_miss : d:int -> Lemma
  (let vm : closure_vm = {
    stack = []; slots = [Dict []; Str "missing"; Num d; Num 3]; pc = 0;
    code = [GetDefaultSlot (0, 1, 2, 3); Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 4; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match list_nth s1.slots 3 with
    | Some (Num r) -> r = d
    | _ -> false))
let step_getdefaultslot_miss d = ()

// ============================================================
// DictMutSet — in-place dict mutation
// ============================================================

val step_dictmutset_new : v:int -> Lemma
  (let vm : closure_vm = {
    stack = [Num v; Str "key"]; slots = [Dict []]; pc = 0;
    code = [DictMutSet 0; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 1; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with
    | Dict d :: _ -> (match dict_get "key" d with | Num r -> r = v | _ -> false)
    | _ -> false) &&
   (match list_nth s1.slots 0 with
    | Some (Dict d) -> (match dict_get "key" d with | Num r -> r = v | _ -> false)
    | _ -> false))
let step_dictmutset_new v = ()

val step_dictmutset_overwrite : old:int -> v:int -> Lemma
  (let vm : closure_vm = {
    stack = [Num v; Str "k"]; slots = [Dict [("k", Num old)]]; pc = 0;
    code = [DictMutSet 0; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 1; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with
    | Dict d :: _ -> (match dict_get "k" d with | Num r -> r = v | _ -> false)
    | _ -> false))
let step_dictmutset_overwrite old v = ()

val step_dictmutset_bad_key : unit -> Lemma
  (let vm : closure_vm = {
    stack = [Num 1; Num 2]; slots = [Dict []]; pc = 0;
    code = [DictMutSet 0; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 1; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = false)
let step_dictmutset_bad_key () = ()
