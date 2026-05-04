module UnivConstruct

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM
open FStar.List.Tot

let n0 : nat = 0
let n1 : nat = 1

val construct_code : list opcode
let construct_code = [
  PushI64 42;
  ConstructTag ("result", n1, n0);
  GetField n0;
  Return;
]

val construct_init : closure_vm
let construct_init = make_closure_vm construct_code [] n0

val vm_construct_and_read : unit -> Lemma
  (let s1 = closure_eval_op construct_init in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   match s4 with
   | { ok = true; stack = Num 42 :: _ } -> true
   | _ -> false)
let vm_construct_and_read () =
  let s1 = closure_eval_op construct_init in
  assert_norm (s1.pc = 1);
  let s2 = closure_eval_op s1 in
  assert_norm (s2.pc = 2);
  let s3 = closure_eval_op s2 in
  assert_norm (s3.pc = 3);
  let s4 = closure_eval_op s3 in
  assert_norm (s4.ok = true)
