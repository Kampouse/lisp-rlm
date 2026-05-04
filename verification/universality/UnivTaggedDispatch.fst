module UnivTagDispatch
open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM
open FStar.List.Tot

let n0 : nat = 0
let n1 : nat = 1

val tagged_dispatch_code : list opcode
let tagged_dispatch_code = [
  LoadSlot 0;
  GetField n0;
  LoadSlot 0;
  GetField n1;
  OpAdd;
  Return;
]

val tagged_prog : lisp_val
let tagged_prog = Tagged ("add", 0, [("0", Num 3); ("1", Num 4)])

val td_init : closure_vm
let td_init =
  let base = make_closure_vm tagged_dispatch_code [] n1 in
  { base with slots = [tagged_prog] }

val vm_tagged_dispatch : unit -> Lemma
  (let s1 = closure_eval_op td_init in
   let s2 = closure_eval_op s1 in
   let s3 = closure_eval_op s2 in
   let s4 = closure_eval_op s3 in
   let s5 = closure_eval_op s4 in
   let s6 = closure_eval_op s5 in
   match s6 with
   | { ok = true; stack = Num 7 :: _ } -> true
   | _ -> false)
let vm_tagged_dispatch () =
  let s1 = closure_eval_op td_init in
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
  assert_norm (s6.ok = true)
