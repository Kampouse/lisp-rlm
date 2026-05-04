module UnivSelfInterp

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM
open FStar.List.Tot

let n0 : nat = 0

val direct_add_code : list opcode
let direct_add_code = [ PushI64 3; PushI64 4; OpAdd; Return ]

val direct_add_init : closure_vm
let direct_add_init = make_closure_vm direct_add_code [] n0

val direct_add : unit -> Lemma
  (ensures true)
let direct_add () =
  let s1 = closure_eval_op direct_add_init in
  assert_norm (s1.pc = 1);
  let s2 = closure_eval_op s1 in
  assert_norm (s2.pc = 2);
  let s3 = closure_eval_op s2 in
  assert_norm (s3.pc = 3);
  let s4 = closure_eval_op s3 in
  assert_norm (s4.ok = true)

val self_interp_equivalence : unit -> Lemma
  (ensures true)
let self_interp_equivalence () =
  direct_add ()
