module OpcodeProofs

#set-options "--z3rlimit 100"

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM

val cvm : list opcode -> list lisp_val -> list lisp_val -> nat -> closure_vm
let cvm code slots stack nslots = {
  stack = stack; slots = slots; pc = 0;
  code = code; ok = true;
  code_table = []; frames = [];
  num_slots = nslots;
  captured = []; closure_envs = []; env = [];
}

// ============================================================
// EASY OPCODES
// ============================================================

// --- PushFloat: modeled abstractly as Float (ff_of_int 0), still pushes a Float ---
val step_pushfloat : unit -> Lemma
  (let vm = cvm [PushFloat (ff_of_int 0); Return] [] [] 0 in
   let s1 = closure_eval_op vm in
   s1.ok = true && (match s1.stack with | Float _ :: _ -> true | _ -> false))
let step_pushfloat () = ()

// --- JumpIfTrue: truthy value jumps ---
val step_jumpiftrue_jumps : unit -> Lemma
  (let code = [PushBool true; JumpIfTrue 3; PushI64 99; Return] in
   let vm = cvm code [] [] 0 in
   let s1 = closure_eval_op vm in
   let s2 = closure_eval_op s1 in
   s2.ok = true && s2.pc = 3)
let step_jumpiftrue_jumps () = ()

// --- JumpIfTrue: falsy value doesn't jump ---
val step_jumpiftrue_nojump : unit -> Lemma
  (let code = [PushBool false; JumpIfTrue 3; PushI64 99; Return] in
   let vm = cvm code [] [] 0 in
   let s1 = closure_eval_op vm in
   let s2 = closure_eval_op s1 in
   s2.ok = true && s2.pc = 2)
let step_jumpiftrue_nojump () = ()

// --- ReturnSlot: returns slot value ---
val step_returnslot : n:int -> Lemma
  (let vm = cvm [ReturnSlot 0] [Num n] [] 1 in
   let s1 = closure_eval_op vm in
   s1.ok = true && (match s1.stack with | Num r :: _ -> r = n | _ -> false))
let step_returnslot n = ()

// --- DictGet: lookup key in dict ---
val step_dictget_found : unit -> Lemma
  (let d : lisp_val = Dict [("x", Num 42)] in
   let vm = cvm [DictGet; Return] [] [Str "x"; d] 0 in
   let s1 = closure_eval_op vm in
   let s2 = closure_eval_op s1 in
   s2.ok = true && (match s2.stack with | Num r :: _ -> r = 42 | _ -> false))
let step_dictget_found () = ()

val step_dictget_miss : unit -> Lemma
  (let d : lisp_val = Dict [("y", Num 42)] in
   let vm = cvm [DictGet; Return] [] [Str "x"; d] 0 in
   let s1 = closure_eval_op vm in
   let s2 = closure_eval_op s1 in
   s2.ok = true && (match s2.stack with | Nil :: _ -> true | _ -> false))
let step_dictget_miss () = ()

// --- DictSet: set key in dict ---
val step_dictset : n:int -> Lemma
  (let d : lisp_val = Dict [] in
   let vm = cvm [DictSet; Return] [] [Num n; Str "k"; d] 0 in
   let s1 = closure_eval_op vm in
   let s2 = closure_eval_op s1 in
   s2.ok = true && (match s2.stack with
                    | Dict entries :: _ ->
                      (match dict_get "k" entries with | Num r -> r = n | _ -> false)
                    | _ -> false))
let step_dictset n = ()

// --- StoreAndLoadSlot: stores value, leaves on stack ---
val step_storeandloadslot : n:int -> Lemma
  (let vm = cvm [StoreAndLoadSlot 0; Return] [Num 0] [Num n] 1 in
   let s1 = closure_eval_op vm in
   let s2 = closure_eval_op s1 in
   s2.ok = true &&
   (match s2.stack with | Num r :: _ -> r = n | _ -> false) &&
   (match s2.slots with | Num v :: _ -> v = n | _ -> false))
let step_storeandloadslot n = ()

// ============================================================
// FUSED SLOT ARITHMETIC OPS
// ============================================================

val step_slotaddimm : base:int -> imm:int -> Lemma
  (imm >= 0 ==>
   (let vm = cvm [SlotAddImm (0, imm); Return] [Num base] [] 1 in
    let s1 = closure_eval_op vm in
    let s2 = closure_eval_op s1 in
    s2.ok = true && (match s2.stack with | Num r :: _ -> r = base + imm | _ -> false)))
let step_slotaddimm base imm = ()

val step_slotsubimm : base:int -> imm:int -> Lemma
  (imm >= 0 ==>
   (let vm = cvm [SlotSubImm (0, imm); Return] [Num base] [] 1 in
    let s1 = closure_eval_op vm in
    let s2 = closure_eval_op s1 in
    s2.ok = true && (match s2.stack with | Num r :: _ -> r = base - imm | _ -> false)))
let step_slotsubimm base imm = ()

val step_slotmulimm : base:int -> imm:int -> Lemma
  (imm >= 0 ==>
   (let vm = cvm [SlotMulImm (0, imm); Return] [Num base] [] 1 in
    let s1 = closure_eval_op vm in
    let s2 = closure_eval_op s1 in
    s2.ok = true && (match s2.stack with | Num r :: _ -> r = int_mul base imm | _ -> false)))
let step_slotmulimm base imm = ()

val step_slotdivimm : base:int -> imm:int -> Lemma
  (imm > 0 ==>
   (let vm = cvm [SlotDivImm (0, imm); Return] [Num base] [] 1 in
    let s1 = closure_eval_op vm in
    let s2 = closure_eval_op s1 in
    s2.ok = true && (match s2.stack with | Num r :: _ -> r = base / imm | _ -> false)))
let step_slotdivimm base imm = ()

// ============================================================
// FUSED SLOT COMPARISON OPS
// ============================================================

val step_sloteqimm : n:int -> imm:int -> Lemma
  (imm >= 0 ==>
   (let vm = cvm [SlotEqImm (0, imm); Return] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    let s2 = closure_eval_op s1 in
    s2.ok = true && (match s2.stack with | Bool r :: _ -> r = (n = imm) | _ -> false)))
let step_sloteqimm n imm = ()

val step_slotltimm : n:int -> imm:int -> Lemma
  (imm >= 0 ==>
   (let vm = cvm [SlotLtImm (0, imm); Return] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    let s2 = closure_eval_op s1 in
    s2.ok = true && (match s2.stack with | Bool r :: _ -> r = (n < imm) | _ -> false)))
let step_slotltimm n imm = ()

val step_slotleimm : n:int -> imm:int -> Lemma
  (imm >= 0 ==>
   (let vm = cvm [SlotLeImm (0, imm); Return] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    let s2 = closure_eval_op s1 in
    s2.ok = true && (match s2.stack with | Bool r :: _ -> r = (n <= imm) | _ -> false)))
let step_slotleimm n imm = ()

val step_slotgtimm : n:int -> imm:int -> Lemma
  (imm >= 0 ==>
   (let vm = cvm [SlotGtImm (0, imm); Return] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    let s2 = closure_eval_op s1 in
    s2.ok = true && (match s2.stack with | Bool r :: _ -> r = (n > imm) | _ -> false)))
let step_slotgtimm n imm = ()

val step_slotgeimm : n:int -> imm:int -> Lemma
  (imm >= 0 ==>
   (let vm = cvm [SlotGeImm (0, imm); Return] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    let s2 = closure_eval_op s1 in
    s2.ok = true && (match s2.stack with | Bool r :: _ -> r = (n >= imm) | _ -> false)))
let step_slotgeimm n imm = ()

// ============================================================
// FUSED JUMP-IF-SLOT COMPARISON OPS
// ============================================================

val step_jumpifslotlt_yes : n:int -> imm:int -> Lemma
  (n < imm ==>
   (let vm = cvm [JumpIfSlotLtImm (0, imm, 2); PushI64 0; PushI64 1] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 2))
let step_jumpifslotlt_yes n imm = ()

val step_jumpifslotlt_no : n:int -> imm:int -> Lemma
  (n >= imm ==> imm >= 0 ==>
   (let vm = cvm [JumpIfSlotLtImm (0, imm, 2); PushI64 0; PushI64 1] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 1))
let step_jumpifslotlt_no n imm = ()

val step_jumpifslotle_yes : n:int -> imm:int -> Lemma
  (n <= imm ==> imm >= 0 ==>
   (let vm = cvm [JumpIfSlotLeImm (0, imm, 2); PushI64 0; PushI64 1] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 2))
let step_jumpifslotle_yes n imm = ()

val step_jumpifslotle_no : n:int -> imm:int -> Lemma
  (n > imm ==> imm >= 0 ==>
   (let vm = cvm [JumpIfSlotLeImm (0, imm, 2); PushI64 0; PushI64 1] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 1))
let step_jumpifslotle_no n imm = ()

val step_jumpifslotgt_yes : n:int -> imm:int -> Lemma
  (n > imm ==> imm >= 0 ==>
   (let vm = cvm [JumpIfSlotGtImm (0, imm, 2); PushI64 0; PushI64 1] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 2))
let step_jumpifslotgt_yes n imm = ()

val step_jumpifslotgt_no : n:int -> imm:int -> Lemma
  (n <= imm ==> imm >= 0 ==>
   (let vm = cvm [JumpIfSlotGtImm (0, imm, 2); PushI64 0; PushI64 1] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 1))
let step_jumpifslotgt_no n imm = ()

val step_jumpifslotge_yes : n:int -> imm:int -> Lemma
  (n >= imm ==> imm >= 0 ==>
   (let vm = cvm [JumpIfSlotGeImm (0, imm, 2); PushI64 0; PushI64 1] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 2))
let step_jumpifslotge_yes n imm = ()

val step_jumpifslotge_no : n:int -> imm:int -> Lemma
  (n < imm ==> imm >= 0 ==>
   (let vm = cvm [JumpIfSlotGeImm (0, imm, 2); PushI64 0; PushI64 1] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 1))
let step_jumpifslotge_no n imm = ()

val step_jumpifsloteq_yes : n:int -> imm:int -> Lemma
  (n = imm ==> imm >= 0 ==>
   (let vm = cvm [JumpIfSlotEqImm (0, imm, 2); PushI64 0; PushI64 1] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 2))
let step_jumpifsloteq_yes n imm = ()

val step_jumpifsloteq_no : n:int -> imm:int -> Lemma
  (not (n = imm) ==> imm >= 0 ==>
   (let vm = cvm [JumpIfSlotEqImm (0, imm, 2); PushI64 0; PushI64 1] [Num n] [] 1 in
    let s1 = closure_eval_op vm in
    s1.ok = true && s1.pc = 1))
let step_jumpifsloteq_no n imm = ()
