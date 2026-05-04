module UnivIterative

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM
open FStar.List.Tot

let n0 : nat = 0
let n1 : nat = 1
let n2 : nat = 2

val accum_code : list opcode
let accum_code = [
  RecurIncAccum (n0, n1, 1, 5, n1);
  ReturnSlot n1;
]

val accum_init : closure_vm
let accum_init =
  let base = make_closure_vm accum_code [] n2 in
  { base with slots = [Num 0; Num 0] }

val accum_sum_0_to_4 : unit -> Lemma
  (let s1 = closure_eval_op accum_init in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   let s5 = closure_eval_op s4 in
   let s6 = closure_eval_op s5 in
   let s7 = closure_eval_op s6 in
   match s7 with
   | { ok = true; stack = [Num 10] } -> true
   | _ -> false)
let accum_sum_0_to_4 () =
  let s1 = closure_eval_op accum_init in
  assert_norm (s1.pc = 0);
  let s2 = closure_eval_op s1 in
  assert_norm (s2.pc = 0);
  let s3 = closure_eval_op s2 in
  assert_norm (s3.pc = 0);
  let s4 = closure_eval_op s3 in
  assert_norm (s4.pc = 0);
  let s5 = closure_eval_op s4 in
  assert_norm (s5.pc = 0);
  let s6 = closure_eval_op s5 in
  assert_norm (s6.pc = 1);
  let s7 = closure_eval_op s6 in
  assert_norm (s7.ok = true)
