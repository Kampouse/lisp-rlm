module UnivAdd34
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

val vm_3_4_init : closure_vm
let vm_3_4_init = minsky_add_vm 3 4

val vm_add_3_4 : unit -> Lemma
  (let s01 = closure_eval_op vm_3_4_init in
   let s02 = closure_eval_op s01 in
   let s03 = closure_eval_op s02 in
   let s04 = closure_eval_op s03 in
   let s05 = closure_eval_op s04 in
   let s06 = closure_eval_op s05 in
   let s07 = closure_eval_op s06 in
   let s08 = closure_eval_op s07 in
   let s09 = closure_eval_op s08 in
   let s10 = closure_eval_op s09 in
   let s11 = closure_eval_op s10 in
   let s12 = closure_eval_op s11 in
   let s13 = closure_eval_op s12 in
   let s14 = closure_eval_op s13 in
   let s15 = closure_eval_op s14 in
   let s16 = closure_eval_op s15 in
   let s17 = closure_eval_op s16 in
   let s18 = closure_eval_op s17 in
   let s19 = closure_eval_op s18 in
   let s20 = closure_eval_op s19 in
   match s20 with
   | { ok = true; stack = [Num 7] } -> true
   | _ -> false)
let vm_add_3_4 () =
  let s01 = closure_eval_op vm_3_4_init in
  assert_norm (s01.pc = 1);
  let s02 = closure_eval_op s01 in
  assert_norm (s02.pc = 2);
  let s03 = closure_eval_op s02 in
  assert_norm (s03.pc = 3);
  let s04 = closure_eval_op s03 in
  assert_norm (s04.pc = 4);
  let s05 = closure_eval_op s04 in
  assert_norm (s05.pc = 5);
  let s06 = closure_eval_op s05 in
  assert_norm (s06.pc = 0);
  let s07 = closure_eval_op s06 in
  assert_norm (s07.pc = 1);
  let s08 = closure_eval_op s07 in
  assert_norm (s08.pc = 2);
  let s09 = closure_eval_op s08 in
  assert_norm (s09.pc = 3);
  let s10 = closure_eval_op s09 in
  assert_norm (s10.pc = 4);
  let s11 = closure_eval_op s10 in
  assert_norm (s11.pc = 5);
  let s12 = closure_eval_op s11 in
  assert_norm (s12.pc = 0);
  let s13 = closure_eval_op s12 in
  assert_norm (s13.pc = 1);
  let s14 = closure_eval_op s13 in
  assert_norm (s14.pc = 2);
  let s15 = closure_eval_op s14 in
  assert_norm (s15.pc = 3);
  let s16 = closure_eval_op s15 in
  assert_norm (s16.pc = 4);
  let s17 = closure_eval_op s16 in
  assert_norm (s17.pc = 5);
  let s18 = closure_eval_op s17 in
  assert_norm (s18.pc = 0);
  let s19 = closure_eval_op s18 in
  assert_norm (s19.pc = 6);
  let s20 = closure_eval_op s19 in
  assert_norm (s20.ok = true)
