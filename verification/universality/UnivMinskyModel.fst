module UnivMinskyModel

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM
open FStar.List.Tot

let n0 : nat = 0
let n1 : nat = 1
let n2 : nat = 2
let n6 : nat = 6

type minsky_state = {
  r1     : int;
  r2     : int;
  m_pc   : int;
  halted : bool;
}

val minsky_add_step : minsky_state -> minsky_state
let minsky_add_step s =
  if s.halted then s
  else match s.m_pc with
  | 0 -> if s.r1 = 0 then { s with m_pc = 3; halted = true }
         else { s with m_pc = 1 }
  | 1 -> { s with r1 = s.r1 - 1; m_pc = 2 }
  | 2 -> { s with r2 = s.r2 + 1; m_pc = 0 }
  | _ -> { s with halted = true }

val minsky_add_run : fuel:int -> minsky_state -> minsky_state
let rec minsky_add_run fuel s =
  if fuel <= 0 then s
  else if s.halted then s
  else minsky_add_run (fuel - 1) (minsky_add_step s)

val ms_3_4 : minsky_state
let ms_3_4 = { r1 = 3; r2 = 4; m_pc = 0; halted = false }

val ms_0_5 : minsky_state
let ms_0_5 = { r1 = 0; r2 = 5; m_pc = 0; halted = false }

val ms_1_1 : minsky_state
let ms_1_1 = { r1 = 1; r2 = 1; m_pc = 0; halted = false }

val minsky_add_3_4 : unit -> Lemma
  (ensures
    (minsky_add_run 100 ms_3_4).halted &&
    (minsky_add_run 100 ms_3_4).r1 = 0 &&
    (minsky_add_run 100 ms_3_4).r2 = 7)
let minsky_add_3_4 () =
  assert_norm (minsky_add_run 100 ms_3_4 = { r1 = 0; r2 = 7; m_pc = 3; halted = true })

val minsky_add_0_5 : unit -> Lemma
  (ensures
    (minsky_add_run 100 ms_0_5).halted &&
    (minsky_add_run 100 ms_0_5).r1 = 0 &&
    (minsky_add_run 100 ms_0_5).r2 = 5)
let minsky_add_0_5 () =
  assert_norm (minsky_add_run 100 ms_0_5 = { r1 = 0; r2 = 5; m_pc = 3; halted = true })

val minsky_add_1_1 : unit -> Lemma
  (ensures
    (minsky_add_run 100 ms_1_1).halted &&
    (minsky_add_run 100 ms_1_1).r1 = 0 &&
    (minsky_add_run 100 ms_1_1).r2 = 2)
let minsky_add_1_1 () =
  assert_norm (minsky_add_run 100 ms_1_1 = { r1 = 0; r2 = 2; m_pc = 3; halted = true })

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

val vm_of_minsky : r1:int -> r2:int -> pc:int -> closure_vm
let vm_of_minsky r1 r2 pc =
  let base = make_closure_vm minsky_add_code [] n2 in
  { base with slots = [Num r1; Num r2]; pc = pc }
