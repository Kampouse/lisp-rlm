(** Dispatch Routing Lemmas — closure_eval_op routes correctly to each handler
    
    For each opcode, we prove that closure_eval_op produces the correct result
    when given a minimal valid state. This verifies the 54-arm match dispatch
    routes to the right handler for every opcode.
    
    These lemmas close the gap between "handler correct" and "VM correct" —
    we already proved each handler's logic correct; these prove dispatch
    actually invokes that handler.
*)

module DispatchRouting

#set-options "--z3rlimit 50000"

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open LispIR.ClosureVM

// Minimal VM state constructor
val s0 : list opcode -> list lisp_val -> nat -> closure_vm
let s0 code slots nslots = {
  stack = []; slots = slots; pc = 0;
  code = code; ok = true;
  code_table = []; frames = [];
  num_slots = nslots;
  captured = []; closure_envs = [];
}

// State with stack pre-loaded
val sws : list opcode -> list lisp_val -> list lisp_val -> nat -> closure_vm
let sws code stack slots nslots = {
  stack = stack; slots = slots; pc = 0;
  code = code; ok = true;
  code_table = []; frames = [];
  num_slots = nslots;
  captured = []; closure_envs = [];
}

// ============================================================
// Group 1: Stack manipulation (10 opcodes)
// ============================================================

val route_pushi64 : n:int -> Lemma
  (let s' = closure_eval_op (s0 [PushI64 n] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num x :: _ -> x = n | _ -> false))
let route_pushi64 n = ()

val route_pushbool : b:bool -> Lemma
  (let s' = closure_eval_op (s0 [PushBool b] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool x :: _ -> x = b | _ -> false))
let route_pushbool b = ()

val route_pushnil : unit -> Lemma
  (let s' = closure_eval_op (s0 [PushNil] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Nil :: _ -> true | _ -> false))
let route_pushnil () = ()

val route_pushstr : k:string -> Lemma
  (let s' = closure_eval_op (s0 [PushStr k] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Str x :: _ -> x = k | _ -> false))
let route_pushstr k = ()

val route_pushfloat : unit -> Lemma
  (let s' = closure_eval_op (s0 [PushFloat (ff_of_int 0)] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Float _ :: _ -> true | _ -> false))
let route_pushfloat () = ()

val route_dup : n:int -> Lemma
  (let s' = closure_eval_op (sws [Dup] [Num n] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num a :: Num b :: _ -> a = n && b = n | _ -> false))
let route_dup n = ()

val route_pop : n:int -> Lemma
  (let s' = closure_eval_op (sws [Pop] [Num n] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | [] -> true | _ -> false))
let route_pop n = ()

val route_loadslot : n:int -> Lemma
  (let s' = closure_eval_op (sws [LoadSlot 0] [] [Num n] 1) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num x :: _ -> x = n | _ -> false))
let route_loadslot n = ()

val route_storeslot : n:int -> Lemma
  (let s' = closure_eval_op (sws [StoreSlot 0] [Num n] [Num 0] 1) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | [] -> true | _ -> false) &&
   (match s'.slots with | Num x :: _ -> x = n | _ -> false))
let route_storeslot n = ()

val route_makelist : unit -> Lemma
  (let s' = closure_eval_op (sws [MakeList 3] [Num 3; Num 2; Num 1] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | List [Num 1; Num 2; Num 3] :: _ -> true | _ -> false))
let route_makelist () = ()

// ============================================================
// Group 2: Arithmetic (5 opcodes)
// ============================================================

val route_opadd : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [OpAdd] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num r :: _ -> r = a + b | _ -> false))
let route_opadd a b = ()

val route_opsub : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [OpSub] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num r :: _ -> r = a - b | _ -> false))
let route_opsub a b = ()

val route_opmul : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [OpMul] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num r :: _ -> r = Prims.op_Multiply a b | _ -> false))
let route_opmul a b = ()

val route_opdiv : a:int -> b:int -> Lemma
  (not (b = 0) ==>
   (let s' = closure_eval_op (sws [OpDiv] [Num b; Num a] [] 0) in
    s'.ok = true && s'.pc = 1 &&
    (match s'.stack with | Num r :: _ -> r = a / b | _ -> false)))
let route_opdiv a b = ()

val route_opmod : a:int -> b:int -> Lemma
  (not (b = 0) ==>
   (let s' = closure_eval_op (sws [OpMod] [Num b; Num a] [] 0) in
    s'.ok = true && s'.pc = 1 &&
    (match s'.stack with | Num r :: _ -> r = a % b | _ -> false)))
let route_opmod a b = ()

// ============================================================
// Group 3: Comparison (5 opcodes)
// ============================================================

val route_opeq_num : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [OpEq] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (a = b) | _ -> false))
let route_opeq_num a b = ()

val route_opgt : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [OpGt] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (a > b) | _ -> false))
let route_opgt a b = ()

val route_oplt : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [OpLt] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (a < b) | _ -> false))
let route_oplt a b = ()

val route_ople : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [OpLe] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (a <= b) | _ -> false))
let route_ople a b = ()

val route_opge : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [OpGe] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (a >= b) | _ -> false))
let route_opge a b = ()

// ============================================================
// Group 4: Control flow (5 opcodes)
// ============================================================

val route_jump : unit -> Lemma
  (let s' = closure_eval_op (s0 [Jump 42] [] 0) in
   s'.ok = true && s'.pc = 42)
let route_jump () = ()

val route_jumpiffalse_truthy : n:int -> Lemma
  (not (n = 0) ==>
   (let s' = closure_eval_op (sws [JumpIfFalse 99] [Num n] [] 0) in
    s'.ok = true && s'.pc = 1 &&
    (match s'.stack with | [] -> true | _ -> false)))
let route_jumpiffalse_truthy n = ()

val route_jumpiffalse_falsy : unit -> Lemma
  (let s' = closure_eval_op (sws [JumpIfFalse 99] [Bool false] [] 0) in
   s'.ok = true && s'.pc = 99 &&
   (match s'.stack with | [] -> true | _ -> false))
let route_jumpiffalse_falsy () = ()

val route_jumpiftrue_falsy : unit -> Lemma
  (let s' = closure_eval_op (sws [JumpIfTrue 99] [Bool false] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | [] -> true | _ -> false))
let route_jumpiftrue_falsy () = ()

val route_jumpiftrue_truthy : n:int -> Lemma
  (not (n = 0) ==>
   (let s' = closure_eval_op (sws [JumpIfTrue 99] [Num n] [] 0) in
    s'.ok = true && s'.pc = 99 &&
    (match s'.stack with | [] -> true | _ -> false)))
let route_jumpiftrue_truthy n = ()

val route_return_noframe : n:int -> Lemma
  (let s' = closure_eval_op (sws [Return] [Num n] [] 0) in
   s'.ok = true &&
   (match s'.stack with | Num x :: _ -> x = n | _ -> false))
let route_return_noframe n = ()

val route_return_withframe : n:int -> Lemma
  (let s = { (sws [Return] [Num n] [] 0) with
             frames = [{ ret_pc = 42; ret_slots = []; ret_stack = [];
                         ret_code = [Return]; ret_num_slots = 0;
                         ret_captured = [] }] } in
   let s' = closure_eval_op s in
   s'.ok = true && s'.pc = 42 &&
   (match s'.stack with | Num x :: _ -> x = n | _ -> false) &&
   (match s'.frames with | [] -> true | _ -> false))
let route_return_withframe n = ()

// ============================================================
// Group 5: Dict ops (3 opcodes)
// ============================================================

val route_dictget_hit : unit -> Lemma
  (let s' = closure_eval_op (sws [DictGet] [Str "x"; Dict [("x", Num 42)]] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num r :: _ -> r = 42 | _ -> false))
let route_dictget_hit () = ()

val route_dictget_miss : unit -> Lemma
  (let s' = closure_eval_op (sws [DictGet] [Str "z"; Dict [("x", Num 42)]] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Nil :: _ -> true | _ -> false))
let route_dictget_miss () = ()

val route_dictset : unit -> Lemma
  (let s' = closure_eval_op (sws [DictSet] [Num 99; Str "y"; Dict [("x", Num 42)]] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with
    | Dict entries :: _ ->
      (match entries with
       | (ky, Num vy) :: (kx, Num vx) :: _ -> ky = "y" && vy = 99 && kx = "x" && vx = 42
       | _ -> false)
    | _ -> false))
let route_dictset () = ()

val route_dictmutset : unit -> Lemma
  (let s' = closure_eval_op (sws [DictMutSet 0] [Num 99; Str "y"; Dict [("x", Num 42)]]
           [Dict [("x", Num 42)]] 1) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with
    | Dict entries :: _ ->
      (match entries with
       | (ky, Num vy) :: (kx, Num vx) :: _ -> ky = "y" && vy = 99 && kx = "x" && vx = 42
       | _ -> false)
    | _ -> false))
let route_dictmutset () = ()

// ============================================================
// Group 6: Slot+immediate fused ops (10 opcodes)
// ============================================================

val route_slotaddimm : n:int -> imm:int -> Lemma
  (let s' = closure_eval_op (sws [SlotAddImm (0, imm)] [] [Num n] 1) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num r :: _ -> r = n + imm | _ -> false))
let route_slotaddimm n imm = ()

val route_slotsubimm : n:int -> imm:int -> Lemma
  (let s' = closure_eval_op (sws [SlotSubImm (0, imm)] [] [Num n] 1) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num r :: _ -> r = n - imm | _ -> false))
let route_slotsubimm n imm = ()

val route_slotmulimm : n:int -> imm:int -> Lemma
  (let s' = closure_eval_op (sws [SlotMulImm (0, imm)] [] [Num n] 1) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num r :: _ -> r = Prims.op_Multiply n imm | _ -> false))
let route_slotmulimm n imm = ()

val route_slotdivimm : n:int -> imm:int -> Lemma
  (not (imm = 0) ==>
   (let s' = closure_eval_op (sws [SlotDivImm (0, imm)] [] [Num n] 1) in
    s'.ok = true && s'.pc = 1 &&
    (match s'.stack with | Num r :: _ -> r = n / imm | _ -> false)))
let route_slotdivimm n imm = ()

val route_sloteqimm : n:int -> imm:int -> Lemma
  (let s' = closure_eval_op (sws [SlotEqImm (0, imm)] [] [Num n] 1) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (n = imm) | _ -> false))
let route_sloteqimm n imm = ()

val route_slotltimm : n:int -> imm:int -> Lemma
  (let s' = closure_eval_op (sws [SlotLtImm (0, imm)] [] [Num n] 1) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (n < imm) | _ -> false))
let route_slotltimm n imm = ()

val route_slotleimm : n:int -> imm:int -> Lemma
  (let s' = closure_eval_op (sws [SlotLeImm (0, imm)] [] [Num n] 1) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (n <= imm) | _ -> false))
let route_slotleimm n imm = ()

val route_slotgtimm : n:int -> imm:int -> Lemma
  (let s' = closure_eval_op (sws [SlotGtImm (0, imm)] [] [Num n] 1) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (n > imm) | _ -> false))
let route_slotgtimm n imm = ()

val route_slotgeimm : n:int -> imm:int -> Lemma
  (let s' = closure_eval_op (sws [SlotGeImm (0, imm)] [] [Num n] 1) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (n >= imm) | _ -> false))
let route_slotgeimm n imm = ()

// ============================================================
// Group 7: Jump-if-slot-comparison fused ops (5 opcodes)
// ============================================================

val route_jumpifslotlt_true : n:int -> imm:int -> addr:nat -> Lemma
  (n < imm ==>
   (let s' = closure_eval_op (sws [JumpIfSlotLtImm (0, imm, addr)] [] [Num n] 1) in
    s'.ok = true && s'.pc = addr))
let route_jumpifslotlt_true n imm addr = ()

val route_jumpifslotlt_false : n:int -> imm:int -> addr:nat -> Lemma
  (n >= imm ==>
   (let s' = closure_eval_op (sws [JumpIfSlotLtImm (0, imm, addr)] [] [Num n] 1) in
    s'.ok = true && s'.pc = 1))
let route_jumpifslotlt_false n imm addr = ()

val route_jumpifslotle_true : n:int -> imm:int -> addr:nat -> Lemma
  (n <= imm ==>
   (let s' = closure_eval_op (sws [JumpIfSlotLeImm (0, imm, addr)] [] [Num n] 1) in
    s'.ok = true && s'.pc = addr))
let route_jumpifslotle_true n imm addr = ()

val route_jumpifslotgt_true : n:int -> imm:int -> addr:nat -> Lemma
  (n > imm ==>
   (let s' = closure_eval_op (sws [JumpIfSlotGtImm (0, imm, addr)] [] [Num n] 1) in
    s'.ok = true && s'.pc = addr))
let route_jumpifslotgt_true n imm addr = ()

val route_jumpifslotge_true : n:int -> imm:int -> addr:nat -> Lemma
  (n >= imm ==>
   (let s' = closure_eval_op (sws [JumpIfSlotGeImm (0, imm, addr)] [] [Num n] 1) in
    s'.ok = true && s'.pc = addr))
let route_jumpifslotge_true n imm addr = ()

val route_jumpifsloteq_true : n:int -> imm:int -> addr:nat -> Lemma
  (n = imm ==>
   (let s' = closure_eval_op (sws [JumpIfSlotEqImm (0, imm, addr)] [] [Num n] 1) in
    s'.ok = true && s'.pc = addr))
let route_jumpifsloteq_true n imm addr = ()

// ============================================================
// Group 8: TypedBinOp (via TypedBinOp constructor)
// ============================================================

val route_typedbinop_add_i64 : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [TypedBinOp (Add, I64)] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num r :: _ -> r = a + b | _ -> false))
let route_typedbinop_add_i64 a b = ()

val route_typedbinop_sub_i64 : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [TypedBinOp (Sub, I64)] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num r :: _ -> r = a - b | _ -> false))
let route_typedbinop_sub_i64 a b = ()

val route_typedbinop_mul_i64 : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [TypedBinOp (Mul, I64)] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num r :: _ -> r = Prims.op_Multiply a b | _ -> false))
let route_typedbinop_mul_i64 a b = ()

val route_typedbinop_eq_i64 : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [TypedBinOp (Eq, I64)] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (a = b) | _ -> false))
let route_typedbinop_eq_i64 a b = ()

val route_typedbinop_lt_i64 : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [TypedBinOp (Lt, I64)] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (a < b) | _ -> false))
let route_typedbinop_lt_i64 a b = ()

val route_typedbinop_gt_i64 : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [TypedBinOp (Gt, I64)] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (a > b) | _ -> false))
let route_typedbinop_gt_i64 a b = ()

val route_typedbinop_le_i64 : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [TypedBinOp (Le, I64)] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (a <= b) | _ -> false))
let route_typedbinop_le_i64 a b = ()

val route_typedbinop_ge_i64 : a:int -> b:int -> Lemma
  (let s' = closure_eval_op (sws [TypedBinOp (Ge, I64)] [Num b; Num a] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Bool r :: _ -> r = (a >= b) | _ -> false))
let route_typedbinop_ge_i64 a b = ()

// ============================================================
// Group 9: Closure ops (5 opcodes)
// ============================================================

val route_pushclosure : unit -> Lemma
  (let s = { (s0 [PushClosure 0] [] 0) with
             code_table = [{ chunk_code = [PushI64 42; Return];
                            chunk_nslots = 0;
                            chunk_runtime_captures = [] }] } in
   let s' = closure_eval_op s in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num inst_id :: _ -> inst_id = 0 | _ -> false))
let route_pushclosure () = ()

val route_loadcaptured : n:int -> Lemma
  (let s = { (s0 [LoadCaptured 0] [] 0) with captured = [Num n] } in
   let s' = closure_eval_op s in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num x :: _ -> x = n | _ -> false))
let route_loadcaptured n = ()

val route_callcaptured : unit -> Lemma
  (let s = { (sws [CallCaptured (0, 0)] [Num 0] [] 0) with
             code_table = [{ chunk_code = [PushI64 42; Return];
                            chunk_nslots = 0;
                            chunk_runtime_captures = [] }];
             closure_envs = [([], 0)] } in
   let s' = closure_eval_op s in
   s'.ok = true && s'.pc = 0 &&
   (match s'.code with | [PushI64 42; Return] -> true | _ -> false) &&
   (match s'.frames with | f :: _ -> f.ret_pc = 1 | _ -> false))
let route_callcaptured () = ()

val route_callcapturedref : unit -> Lemma
  (let s = { (s0 [CallCapturedRef (0, 0)] [] 0) with
             slots = [Num 0];
             code_table = [{ chunk_code = [PushI64 42; Return];
                            chunk_nslots = 0;
                            chunk_runtime_captures = [] }];
             closure_envs = [([], 0)];
             num_slots = 1 } in
   let s' = closure_eval_op s in
   s'.ok = true && s'.pc = 0 &&
   (match s'.code with | [PushI64 42; Return] -> true | _ -> false) &&
   (match s'.frames with | f :: _ -> f.ret_pc = 1 | _ -> false))
let route_callcapturedref () = ()

val route_callself : unit -> Lemma
  (let s = { (sws [CallSelf 1] [Num 42] [Num 0] 1) with
             code = [CallSelf 1; Return] } in
   let s' = closure_eval_op s in
   s'.ok = true && s'.pc = 0 &&
   (match s'.stack with | [] -> true | _ -> false) &&
   (match s'.frames with | f :: _ -> f.ret_pc = 1 | _ -> false))
let route_callself () = ()

// ============================================================
// Group 10: Fused patterns (4 opcodes)
// ============================================================

val route_storeandloadslot : n:int -> Lemma
  (let s' = closure_eval_op (sws [StoreAndLoadSlot 0] [Num n] [Num 0] 1) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num x :: _ -> x = n | _ -> false))
let route_storeandloadslot n = ()

val route_returnslot : n:int -> Lemma
  (let s = { (s0 [ReturnSlot 0] [] 0) with
             slots = [Num n];
             frames = [{ ret_pc = 42; ret_slots = []; ret_stack = [];
                         ret_code = [Return]; ret_num_slots = 0;
                         ret_captured = [] }];
             num_slots = 1 } in
   let s' = closure_eval_op s in
   s'.ok = true && s'.pc = 42 &&
   (match s'.stack with | Num x :: _ -> x = n | _ -> false) &&
   (match s'.frames with | [] -> true | _ -> false))
let route_returnslot n = ()

val route_getdefaultslot_hit : unit -> Lemma
  (let s = { (s0 [GetDefaultSlot (0, 1, 2, 3)] [] 4) with
             slots = [Dict [("score", Num 5)]; Str "score"; Num 0; Num 0] } in
   let s' = closure_eval_op s in
   s'.ok = true && s'.pc = 1 &&
   (match list_nth s'.slots 3 with
    | Some (Num r) -> r = 5
    | _ -> false))
let route_getdefaultslot_hit () = ()

val route_getdefaultslot_miss : unit -> Lemma
  (let s = { (s0 [GetDefaultSlot (0, 1, 2, 3)] [] 4) with
             slots = [Dict []; Str "missing"; Num 99; Num 0] } in
   let s' = closure_eval_op s in
   s'.ok = true && s'.pc = 1 &&
   (match list_nth s'.slots 3 with
    | Some (Num r) -> r = 99
    | _ -> false))
let route_getdefaultslot_miss () = ()

// ============================================================
// Group 11: Loop/recur ops (2 opcodes)
// ============================================================

val route_recur : unit -> Lemma
  (let s = { (sws [Recur 2] [Num 42] [Num 0; Num 1] 2) with
             code = [Recur 2; Return] } in
   let s' = closure_eval_op s in
   s'.ok = true && s'.pc = 0 &&
   (match s'.stack with | [] -> true | _ -> false))
let route_recur () = ()

val route_recurincaccum : unit -> Lemma
  (let s = { (s0 [RecurIncAccum (0, 1, 2, 10, 99)] [] 3) with
             slots = [Num 3; Num 10; Num 2];
             code = [RecurIncAccum (0, 1, 2, 10, 99); Return] } in
   let s' = closure_eval_op s in
   s'.ok = true && s'.pc = 0)
let route_recurincaccum () = ()

// ============================================================
// Group 12: BuiltinCall + LoadGlobal
// ============================================================

val route_builtincall_length : unit -> Lemma
  (let s' = closure_eval_op (sws [BuiltinCall ("length", 1)]
           [List [Num 1; Num 2; Num 3]] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num 3 :: _ -> true | _ -> false))
let route_builtincall_length () = ()

val route_builtincall_car : unit -> Lemma
  (let s' = closure_eval_op (sws [BuiltinCall ("car", 1)]
           [List [Num 42; Num 99]] [] 0) in
   s'.ok = true && s'.pc = 1 &&
   (match s'.stack with | Num 42 :: _ -> true | _ -> false))
let route_builtincall_car () = ()

val route_builtincall_abs : n:int -> Lemma
  (n < 0 ==>
   (let s' = closure_eval_op (sws [BuiltinCall ("abs", 1)] [Num n] [] 0) in
    s'.ok = true && s'.pc = 1 &&
    (match s'.stack with | Num r :: _ -> r = -n | _ -> false)))
let route_builtincall_abs n = ()

val route_loadglobal : unit -> Lemma
  (let s' = closure_eval_op (s0 [LoadGlobal "x"] [] 0) in
   not s'.ok)
let route_loadglobal () = ()

// ============================================================
// Group 13: Edge cases
// ============================================================

// pc out of bounds → ok=false
val route_out_of_bounds : unit -> Lemma
  (let s' = closure_eval_op { (s0 [] [] 0) with pc = 5 } in
   not s'.ok)
let route_out_of_bounds () = ()

// Div by zero → ok=false
val route_opdiv_zero : a:int -> Lemma
  (let s' = closure_eval_op (sws [OpDiv] [Num 0; Num a] [] 0) in
   not s'.ok)
let route_opdiv_zero a = ()

// Empty stack for Dup → ok=false
val route_dup_empty : unit -> Lemma
  (let s' = closure_eval_op (s0 [Dup] [] 0) in
   not s'.ok)
let route_dup_empty () = ()

// Empty stack for OpAdd → ok=false
val route_opadd_empty : unit -> Lemma
  (let s' = closure_eval_op (s0 [OpAdd] [] 0) in
   not s'.ok)
let route_opadd_empty () = ()
