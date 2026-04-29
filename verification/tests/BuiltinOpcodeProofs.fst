(** Builtin and Global Opcode Step Proofs
    
    All BuiltinCall value specs AUTO-PROVEN after extracting builtin_result.
    LoadGlobal auto-proven (always ok=false).
*)
module BuiltinOpcodeProofs

#set-options "--z3rlimit 500"

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM

// ============================================================
// LoadGlobal — always fails (no env model)
// ============================================================

val step_loadglobal : unit -> Lemma
  (let vm : closure_vm = {
    stack = []; slots = []; pc = 0;
    code = [LoadGlobal "foo"; Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = false)
let step_loadglobal () = ()

// ============================================================
// BuiltinCall — all value specs auto-proven via builtin_result extraction
// ============================================================

// --- abs ---
val step_builtin_abs_pos : a:int -> Lemma
  (a >= 0 ==>
   (let vm : closure_vm = {
     stack = [Num a]; slots = []; pc = 0;
     code = [BuiltinCall ("abs", 1); Return]; ok = true;
     code_table = []; frames = [];
     num_slots = 0; captured = []; closure_envs = [];
   } in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 1 &&
    (match s1.stack with | Num r :: _ -> r = a | _ -> false)))
let step_builtin_abs_pos a = ()

val step_builtin_abs_neg : a:int -> Lemma
  (a < 0 ==>
   (let vm : closure_vm = {
     stack = [Num a]; slots = []; pc = 0;
     code = [BuiltinCall ("abs", 1); Return]; ok = true;
     code_table = []; frames = [];
     num_slots = 0; captured = []; closure_envs = [];
   } in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 1 &&
    (match s1.stack with | Num r :: _ -> r = -a | _ -> false)))
let step_builtin_abs_neg a = ()

// --- length ---
val step_builtin_length3 : unit -> Lemma
  (let vm : closure_vm = {
    stack = [List [Num 1; Num 2; Num 3]]; slots = []; pc = 0;
    code = [BuiltinCall ("length", 1); Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with | Num r :: _ -> r = 3 | _ -> false))
let step_builtin_length3 () = ()

val step_builtin_length_nil : unit -> Lemma
  (let vm : closure_vm = {
    stack = [Nil]; slots = []; pc = 0;
    code = [BuiltinCall ("length", 1); Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with | Num r :: _ -> r = 0 | _ -> false))
let step_builtin_length_nil () = ()

// --- str-concat ---
val step_builtin_strconcat : a:string -> b:string -> Lemma
  (let vm : closure_vm = {
    stack = [Str b; Str a]; slots = []; pc = 0;
    code = [BuiltinCall ("str-concat", 2); Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with | Str r :: _ -> r = a ^ b | _ -> false))
let step_builtin_strconcat a b = ()

// --- min ---
val step_builtin_min_lt : a:int -> b:int -> Lemma
  (a < b ==>
   (let vm : closure_vm = {
     stack = [Num b; Num a]; slots = []; pc = 0;
     code = [BuiltinCall ("min", 2); Return]; ok = true;
     code_table = []; frames = [];
     num_slots = 0; captured = []; closure_envs = [];
   } in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 1 &&
    (match s1.stack with | Num r :: _ -> r = a | _ -> false)))
let step_builtin_min_lt a b = ()

val step_builtin_min_ge : a:int -> b:int -> Lemma
  (not (a < b) ==>
   (let vm : closure_vm = {
     stack = [Num b; Num a]; slots = []; pc = 0;
     code = [BuiltinCall ("min", 2); Return]; ok = true;
     code_table = []; frames = [];
     num_slots = 0; captured = []; closure_envs = [];
   } in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 1 &&
    (match s1.stack with | Num r :: _ -> r = b | _ -> false)))
let step_builtin_min_ge a b = ()

// --- max ---
val step_builtin_max_gt : a:int -> b:int -> Lemma
  (a > b ==>
   (let vm : closure_vm = {
     stack = [Num b; Num a]; slots = []; pc = 0;
     code = [BuiltinCall ("max", 2); Return]; ok = true;
     code_table = []; frames = [];
     num_slots = 0; captured = []; closure_envs = [];
   } in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 1 &&
    (match s1.stack with | Num r :: _ -> r = a | _ -> false)))
let step_builtin_max_gt a b = ()

val step_builtin_max_le : a:int -> b:int -> Lemma
  (not (a > b) ==>
   (let vm : closure_vm = {
     stack = [Num b; Num a]; slots = []; pc = 0;
     code = [BuiltinCall ("max", 2); Return]; ok = true;
     code_table = []; frames = [];
     num_slots = 0; captured = []; closure_envs = [];
   } in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 1 &&
    (match s1.stack with | Num r :: _ -> r = b | _ -> false)))
let step_builtin_max_le a b = ()

// --- car ---
val step_builtin_car : a:int -> b:int -> Lemma
  (let vm : closure_vm = {
    stack = [List [Num a; Num b]]; slots = []; pc = 0;
    code = [BuiltinCall ("car", 1); Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with | Num r :: _ -> r = a | _ -> false))
let step_builtin_car a b = ()

// --- cdr ---
val step_builtin_cdr : a:int -> b:int -> Lemma
  (let vm : closure_vm = {
    stack = [List [Num a; Num b]]; slots = []; pc = 0;
    code = [BuiltinCall ("cdr", 1); Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with | List [Num r] :: _ -> r = b | _ -> false))
let step_builtin_cdr a b = ()

// --- cons ---
val step_builtin_cons : a:int -> b:int -> Lemma
  (let vm : closure_vm = {
    stack = [List [Num b]; Num a]; slots = []; pc = 0;
    code = [BuiltinCall ("cons", 2); Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with
    | List items :: _ -> (match items with | Num r :: Num s :: _ -> r = a && s = b | _ -> false)
    | _ -> false))
let step_builtin_cons a b = ()

// --- list ---
val step_builtin_list : a:int -> b:int -> Lemma
  (let vm : closure_vm = {
    stack = [Num b; Num a]; slots = []; pc = 0;
    code = [BuiltinCall ("list", 2); Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with
    | List items :: _ -> (match items with | Num r :: Num s :: _ -> r = a && s = b | _ -> false)
    | _ -> false))
let step_builtin_list a b = ()

// --- append ---
val step_builtin_append : a:int -> b:int -> Lemma
  (let vm : closure_vm = {
    stack = [List [Num b]; List [Num a]]; slots = []; pc = 0;
    code = [BuiltinCall ("append", 2); Return]; ok = true;
    code_table = []; frames = [];
    num_slots = 0; captured = []; closure_envs = [];
  } in
   let s1 = closure_eval_op vm in
   s1.ok = true && s1.pc = 1 &&
   (match s1.stack with
    | List items :: _ -> (match items with | Num r :: Num s :: _ -> r = a && s = b | _ -> false)
    | _ -> false))
let step_builtin_append a b = ()
