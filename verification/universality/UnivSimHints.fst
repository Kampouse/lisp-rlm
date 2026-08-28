module UnivSimHints

#set-options "--z3rlimit 400"

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM
open FStar.List.Tot

let n0 : nat = 0
let n1 : nat = 1
let n2 : nat = 2
let n6 : nat = 6

val minsky_add_code : list opcode
let minsky_add_code = [
  JumpIfSlotEqImm (n0, 0, n6);
  SlotSubImm (n0, 1);
  StoreAndLoadSlot n0;
  SlotAddImm (n1, 1);
  StoreAndLoadSlot n1;
  Recur n2;
  ReturnSlot n1;
]

val vm_of_minsky : r1:int -> r2:int -> pc:int -> closure_vm
let vm_of_minsky r1 r2 pc =
  let base = make_closure_vm minsky_add_code [] n2 in
  { base with slots = [Num r1; Num r2]; pc = pc }

val sim_step0 : r1:int -> r2:int -> unit -> Lemma
  (r1 > 0 ==>
   (match closure_eval_op (vm_of_minsky r1 r2 0) with
    | { ok = true; pc = 1; slots = [Num r1; Num r2]; stack = [] } -> true
    | _ -> false))
let sim_step0 r1 r2 () = ()

val sim_halt_branch : r2:int -> unit -> Lemma
  (match closure_eval_op (vm_of_minsky 0 r2 0) with
   | { ok = true; pc = 6; slots = [Num 0; Num r2]; stack = [] } -> true
   | _ -> false)
let sim_halt_branch r2 () = ()

val sim_step1 : r1:int -> r2:int -> unit -> Lemma
  (match closure_eval_op (vm_of_minsky r1 r2 1) with
   | { ok = true; pc = 2; slots = [Num r1; Num r2]; stack = [Num v] } -> v = r1 - 1
   | _ -> false)
let sim_step1 r1 r2 () = ()

val sim_step2_at : v:int -> prev:int -> r2:int -> st:closure_vm -> unit -> Lemma
  (requires st.pc == 2 /\ st.ok /\ st.slots == [Num prev; Num r2]
            /\ st.stack == [Num v] /\ st.code == minsky_add_code)
  (ensures ((match closure_eval_op st with
   | { ok = true; pc = 3; slots = [Num s0; Num s1]; stack = [Num stk]; code = c } ->
       s0 = v /\ s1 = r2 /\ stk = v /\ c == minsky_add_code
     | _ -> false)))
let sim_step2_at v prev r2 st _ = ()

val sim_step3_at : r1m:int -> r2:int -> st:closure_vm -> unit -> Lemma
  (requires st.pc == 3 /\ st.ok /\ st.slots == [Num r1m; Num r2]
            /\ st.stack == [Num r1m] /\ st.code == minsky_add_code)
  (ensures ((match closure_eval_op st with
   | { ok = true; pc = 4; slots = [Num r1m; Num r2]; stack = [Num a; Num b] } ->
     a = r2 + 1 /\ b = r1m
   | _ -> false)))
let sim_step3_at r1m r2 st _ = ()

val sim_step4 : r1m:int -> r2p:int -> unit -> Lemma
  (match closure_eval_op { (vm_of_minsky r1m r2p 4) with stack = [Num r2p; Num r1m] } with
   | { ok = true; pc = 5; slots = [Num s0; Num s1] } ->
     s0 = r1m /\ s1 = r2p
   | _ -> false)
let sim_step4 r1m r2p () = ()

val sim_step5 : r1m:int -> r2p:int -> unit -> Lemma
  (match closure_eval_op { (vm_of_minsky r1m r2p 5) with stack = [Num r2p; Num r1m] } with
   | { ok = true; pc = 0; slots = [Num s0; Num s1]; stack = [] } ->
     s0 = r1m /\ s1 = r2p
   | _ -> false)
let sim_step5 r1m r2p () = ()

val sim_return : r2:int -> unit -> Lemma
  (match closure_eval_op (vm_of_minsky 0 r2 6) with
   | { ok = true; stack = [Num v] } -> v = r2
   | _ -> false)
let sim_return r2 () = ()

val sim_two_steps : r1:int -> r2:int -> unit -> Lemma
  (requires r1 > 0)
  (ensures ((let s2 = closure_eval_op (closure_eval_op (vm_of_minsky r1 r2 0)) in
           s2.ok == true /\ s2.pc == 2 /\ s2.slots == [Num r1; Num r2]
           /\ s2.stack == [Num (r1 - 1)] /\ s2.code == minsky_add_code)))
let sim_two_steps r1 r2 () =
  sim_step0 r1 r2 ();
  let s1 = closure_eval_op (vm_of_minsky r1 r2 0) in
  assert (s1.pc == 1 /\ s1.slots == [Num r1; Num r2] /\ s1.stack == []);
  sim_step1 r1 r2 ()

val sim_four_steps : r1:int -> r2:int -> unit -> Lemma
  (requires r1 > 0)
  (ensures ((let s4 = closure_eval_op
              (closure_eval_op
               (closure_eval_op
                (closure_eval_op (vm_of_minsky r1 r2 0)))) in
             match s4 with
             | { ok = true; pc = 4; slots = [Num s0; Num s2]; stack = [Num a; Num b] } ->
               s0 = r1 - 1 /\ s2 = r2 /\ a = r2 + 1 /\ b = r1 - 1
             | _ -> false)))
let sim_four_steps r1 r2 () =
  sim_two_steps r1 r2 ();
  let s2 = closure_eval_op (closure_eval_op (vm_of_minsky r1 r2 0)) in
  sim_step2_at (r1 - 1) r1 r2 s2 ();
  let s3 = closure_eval_op s2 in
  sim_step3_at (r1 - 1) r2 s3 ()


val sim_halt_result : r2:int -> unit -> Lemma
  (match closure_eval_op (closure_eval_op (vm_of_minsky 0 r2 0)) with
   | { ok = true; stack = [Num v] } -> v = r2
   | _ -> false)
let sim_halt_result r2 () =
  sim_halt_branch r2 (); sim_return r2 ()
