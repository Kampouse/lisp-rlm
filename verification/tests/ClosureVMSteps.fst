(** Closure VM step-by-step proofs *)
module ClosureVMSteps

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM

val s0 : closure_vm
let s0 = make_closure_vm [PushClosure 0; CallCaptured (0, 0); PushNil] [[PushI64 42; Return]]

// Step 1: PushClosure 0 → pushes Num 0
val step1 : unit -> Lemma
  (match closure_eval_op s0 with
   | { stack = Num r :: _ } -> r = 0
   | _ -> false)
let step1 () = ()

// Step 2: CallCaptured (0,0) → switches to chunk code
val step2 : unit -> Lemma
  (let s1 = closure_eval_op s0 in  // after PushClosure
   match closure_eval_op s1 with
   | { code = [PushI64 n; Return]; pc = 0; ret_pc = 2 } -> n = 42  // Hmm, F* can't see ret_pc = 2 because it's opaque
   | _ -> false)
// Actually, this is too many computation steps. Let me try a simpler approach.

// Direct state construction + one step
val push_closure_step : unit -> Lemma
  (let s = { stack = []; slots = []; pc = 0;
             code = [PushClosure 5; PushNil];
             ok = true;
             code_table = [];
             ret_pc = 0; ret_code = [] } in
   match closure_eval_op s with
   | { stack = Num r :: []; pc = 1 } -> r = 5
   | _ -> false)
let push_closure_step () = ()

val call_captured_step : unit -> Lemma
  (let s = { stack = [Num 0]; slots = []; pc = 0;
             code = [CallCaptured (0, 0)];
             ok = true;
             code_table = [[PushI64 42; Return]];
             ret_pc = 99; ret_code = [PushNil] } in
   match closure_eval_op s with
   | { code = [PushI64 n; Return]; pc = 0; stack = []; slots = [];
       ret_pc = 1 } -> n = 42
   | _ -> false)
let call_captured_step () = ()

val return_step : unit -> Lemma
  (let s = { stack = [Num 42]; slots = []; pc = 0;
             code = [Return];
             ok = true;
             code_table = [];
             ret_pc = 1; ret_code = [PushI64 99; PushNil] } in
   match closure_eval_op s with
   | { code = [PushI64 n; PushNil]; pc = 1; stack = [Num 42] } -> n = 99
   | _ -> false)
let return_step () = ()
