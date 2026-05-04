module UnivTagTest
open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM
open FStar.List.Tot

let n0 : nat = 0
let n1 : nat = 1
let n8 : nat = 8

val tag_test_code : list opcode
let tag_test_code = [
  LoadSlot 0;
  TagTest ("add", n0);
  JumpIfFalse n8;
  GetField n0;
  LoadSlot 0;
  GetField n1;
  OpAdd;
  Return;
]

val tagged_prog : lisp_val
let tagged_prog = Tagged ("add", 0, [("0", Num 3); ("1", Num 4)])

val tt_init : closure_vm
let tt_init =
  let base = make_closure_vm tag_test_code [] n1 in
  { base with slots = [tagged_prog] }

val vm_tag_test_dispatch : unit -> Lemma
  (let s1 = closure_eval_op tt_init in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   let s5 = closure_eval_op s4 in
   let s6 = closure_eval_op s5 in
   let s7 = closure_eval_op s6 in
   let s8 = closure_eval_op s7 in
   match s8 with
   | { ok = true; stack = Num 7 :: _ } -> true
   | _ -> false)
let vm_tag_test_dispatch () =
  let s1 = closure_eval_op tt_init in
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
  assert_norm (s6.pc = 6);
  let s7 = closure_eval_op s6 in
  assert_norm (s7.pc = 7);
  let s8 = closure_eval_op s7 in
  assert_norm (s8.ok = true)
