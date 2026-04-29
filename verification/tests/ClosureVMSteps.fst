(** Closure VM step-by-step proofs — Frame Stack Model
    
    Uses extracted handlers directly (bypassing 54-arm dispatch)
    with match-based equality for noeq types.
*)
module ClosureVMSteps

#set-options "--z3rlimit 5000"

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM

// Test: make_closure_vm — PushClosure captures runtime values from slots
val s0 : closure_vm
let s0 = make_closure_vm [PushClosure 0; CallCaptured (0, 0); PushNil]
  [{ chunk_code = [PushI64 42; Return]; chunk_nslots = 0; chunk_runtime_captures = [] }] 0

// Step 1: PushClosure 0 → creates closure instance 0, pushes Num 0 onto stack
val step1 : unit -> Lemma
  (match closure_eval_op s0 with
   | { stack = Num r :: _ } -> r = 0
   | _ -> false)
let step1 () = ()

// Direct state construction + PushClosure step (no code_table entry → ok=false)
val push_closure_step : unit -> Lemma
  (let s = { stack = []; slots = []; pc = 0;
             code = [PushClosure 5; PushNil];
             ok = true;
             code_table = [];
             frames = [];
             num_slots = 0;
             captured = [];
             closure_envs = [] } in
   match closure_eval_op s with
   | { ok = false } -> true
   | _ -> false)
let push_closure_step () = ()

// PushClosure with runtime_captures: reads slots[0] at creation time
val push_closure_capture : unit -> Lemma
  (let s = { stack = []; slots = [Num 99]; pc = 0;
             code = [PushClosure 0];
             ok = true;
             code_table = [{ chunk_code = [LoadCaptured 0; Return];
                            chunk_nslots = 1;
                            chunk_runtime_captures = [("x", 0)] }];
             frames = [];
             num_slots = 1;
             captured = [];
             closure_envs = [] } in
   let s1 = closure_eval_op s in
   s1.ok = true &&
   (match s1.stack with | Num inst_id :: _ -> inst_id = 0 | _ -> false) &&
   (match s1.closure_envs with
    | [(caps, 0)] ->
      (match caps with | [Num 99] -> true | _ -> false)
    | _ -> false))
let push_closure_capture () = ()

// CallCaptured: uses extracted handler directly
val call_captured_step : unit -> Lemma
  (let s = { stack = [Num 0]; slots = []; pc = 0;
             code = [CallCaptured (0, 0)];
             ok = true;
             code_table = [{ chunk_code = [PushI64 42; Return];
                            chunk_nslots = 0;
                            chunk_runtime_captures = [] }];
             frames = [];
             num_slots = 0;
             captured = [];
             closure_envs = [([], 0)] } in
   let s1 = callcaptured_handler 0 s 1 in
   s1.pc = 0 && s1.ok = true &&
   (match s1.code with | [PushI64 42; Return] -> true | _ -> false) &&
   (match s1.stack with | [] -> true | _ -> false) &&
   (match s1.frames with
    | [f] ->
      f.ret_pc = 1 &&
      (match f.ret_slots with | [] -> true | _ -> false) &&
      (match f.ret_stack with | [] -> true | _ -> false) &&
      (match f.ret_code with | [CallCaptured (0, 0)] -> true | _ -> false) &&
      f.ret_num_slots = 0 &&
      (match f.ret_captured with | [] -> true | _ -> false)
    | _ -> false))
let call_captured_step () = ()

// Return: uses extracted handler directly
val return_step : unit -> Lemma
  (let s = { stack = [Num 42]; slots = []; pc = 0;
             code = [Return];
             ok = true;
             code_table = [];
             frames = [{ ret_pc = 1; ret_slots = []; ret_stack = [];
                         ret_code = [Return]; ret_num_slots = 0;
                         ret_captured = [] }];
             num_slots = 0;
             captured = [];
             closure_envs = [] } in
   let s1 = return_handler s in
   s1.pc = 1 &&
   s1.ok = true &&
   (match s1.stack with | Num 42 :: _ -> true | _ -> false) &&
   (match s1.frames with | [] -> true | _ -> false) &&
   (match s1.code with | [Return] -> true | _ -> false))
let return_step () = ()

// CallSelf: uses extracted handler directly
val callself_step : unit -> Lemma
  (let s = { stack = [Num 10; Num 20]; slots = [Bool true]; pc = 3;
             code = [CallSelf 2; PushNil];
             ok = true;
             code_table = [];
             frames = [];
             num_slots = 4;
             captured = [];
             closure_envs = [] } in
   let s1 = callself_handler 2 s 4 in
   s1.pc = 0 &&
   s1.ok = true &&
   (match s1.stack with | [] -> true | _ -> false) &&
   (match s1.slots with
    | [Num a; Num b; Nil; Nil] -> a = 20 && b = 10
    | _ -> false) &&
   (match s1.frames with
    | [f] ->
      f.ret_pc = 4 &&
      (match f.ret_slots with | [Bool true] -> true | _ -> false) &&
      (match f.ret_stack with | [] -> true | _ -> false) &&
      (match f.ret_code with | [CallSelf 2; PushNil] -> true | _ -> false) &&
      f.ret_num_slots = 4 &&
      (match f.ret_captured with | [] -> true | _ -> false)
    | _ -> false))
let callself_step () = ()
