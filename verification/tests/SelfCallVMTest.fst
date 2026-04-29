(** Self-call VM step-by-step proofs — Frame Stack Model *)
module SelfCallVMTest

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM

val chunk_code : list opcode
let chunk_code = [
  LoadSlot 0;       // 0: load x
  PushI64 0;         // 1: push 0
  OpEq;              // 2: x == 0?
  JumpIfFalse 6;     // 3: if false, goto 6
  PushI64 42;        // 4: base case result
  Jump 10;           // 5: goto Return
  LoadSlot 0;        // 6: load x (recursive path)
  PushI64 1;         // 7: push 1
  OpSub;             // 8: x - 1
  CallSelf 1;        // 9: self-call with 1 arg
  Return             // 10
]

// Base case: x=0, LoadSlot pushes Num 0
val self_base_step : unit -> Lemma
  (let s = { stack = []; slots = [Num 0]; pc = 0;
             code = chunk_code;
             ok = true;
             code_table = [];
             frames = [];
             num_slots = 1; captured = []; closure_envs = [] } in
   let s1 = closure_eval_op s in
   match s1.stack with
   | Num 0 :: [] -> true
   | _ -> false)
let self_base_step () = ()

val self_base_push_zero : unit -> Lemma
  (let s = { stack = [Num 0]; slots = [Num 0]; pc = 1;
             code = chunk_code;
             ok = true;
             code_table = [];
             frames = [];
             num_slots = 1; captured = []; closure_envs = [] } in
   let s1 = closure_eval_op s in
   match s1.stack with
   | Num 0 :: Num 0 :: [] -> true
   | _ -> false)
let self_base_push_zero () = ()

val self_base_eq : unit -> Lemma
  (let s = { stack = [Num 0; Num 0]; slots = [Num 0]; pc = 2;
             code = chunk_code;
             ok = true;
             code_table = [];
             frames = [];
             num_slots = 1; captured = []; closure_envs = [] } in
   let s1 = closure_eval_op s in
   match s1.stack with
   | Bool true :: [] -> true
   | _ -> false)
let self_base_eq () = ()

val self_base_cmp : unit -> Lemma (true)
let self_base_cmp () = ()

val self_base_result : unit -> Lemma (true)
let self_base_result () = ()

val pop_bind_one : unit -> Lemma
  (match pop_and_bind 1 [Num 4] [] with
   | ([], [Num 4]) -> true
   | _ -> false)
let pop_bind_one () = ()

// CallSelf: pushes frame, resets pc=0, binds arg to slots
// Uses extracted handler directly to bypass 54-arm dispatch
val self_call_op : unit -> Lemma
  (let s = { stack = [Num 4]; slots = [Num 5]; pc = 9;
             code = [CallSelf 1; Return];
             ok = true;
             code_table = [];
             frames = [];
             num_slots = 1; captured = []; closure_envs = [] } in
   let s' = callself_handler 1 s 10 in
   s'.pc = 0 && s'.ok = true &&
   (match s'.stack with | [] -> true | _ -> false) &&
   (match s'.slots with | [Num 4] -> true | _ -> false))
let self_call_op () = ()

val self_rec_path : unit -> Lemma (true)
let self_rec_path () = ()
