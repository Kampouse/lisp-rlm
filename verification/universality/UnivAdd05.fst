module UnivAdd05

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

val minsky_add_vm : r1:int -> r2:int -> closure_vm
let minsky_add_vm r1 r2 =
  let base = make_closure_vm minsky_add_code [] n2 in
  { base with slots = [Num r1; Num r2] }

val vm_0_5_init : closure_vm
let vm_0_5_init = minsky_add_vm 0 5

val vm_add_0_5 : unit -> Lemma
  (let s1 = closure_eval_op vm_0_5_init in
   let s2 = closure_eval_op s1 in
   match s2 with
   | { ok = true; stack = [Num 5] } -> true
   | _ -> false)
let vm_add_0_5 () =
  let s1 = closure_eval_op vm_0_5_init in
  assert_norm (s1.pc = 6);
  let s2 = closure_eval_op s1 in
  assert_norm (s2.ok = true)
