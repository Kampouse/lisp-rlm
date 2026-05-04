module UnivAdd11
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

val vm_1_1_init : closure_vm
let vm_1_1_init = minsky_add_vm 1 1

val vm_add_1_1 : unit -> Lemma
  (let s1 = closure_eval_op vm_1_1_init in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   let s5 = closure_eval_op s4 in
   let s6 = closure_eval_op s5 in
   let s7 = closure_eval_op s6 in
   let s8 = closure_eval_op s7 in
   match s8 with
   | { ok = true; stack = [Num 2] } -> true
   | _ -> false)
let vm_add_1_1 () =
  let s1 = closure_eval_op vm_1_1_init in
  assert_norm (s1.pc = 1);
  let s2 = closure_eval_op s1 in
  assert_norm (s2.pc = 2);
  let s3 = closure_eval_op s2 in
  assert_norm (s3.pc = 3);
  let s4 = closure_eval_op s3 in
  assert_norm (s4.pc = 4);
  let s5 = closure_eval_op s4 in
  assert_norm (s5.pc = 5);
  let s6 = closure_eval_op s5 in
  assert_norm (s6.pc = 0);
  let s7 = closure_eval_op s6 in
  assert_norm (s7.pc = 6);
  let s8 = closure_eval_op s7 in
  assert_norm (s8.ok = true)
