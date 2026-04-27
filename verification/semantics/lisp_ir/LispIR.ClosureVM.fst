(** Closures in the VM — bytecode-level model
    
    Multi-chunk VM: code table maps indices to code sequences.
    PushClosure n pushes a reference to code chunk n.
    CallCaptured argc pops closure + args, switches to that chunk.
    Return pops back to caller's continuation.
    One level of call/return — sufficient for non-nested closure calls.
*)
module LispIR.ClosureVM

open Lisp.Types
open Lisp.Values
open LispIR.Semantics

noeq type closure_vm = {
  stack   : list lisp_val;
  slots   : list lisp_val;
  pc      : int;
  code    : list opcode;
  ok      : bool;
  code_table : list (list opcode);
  ret_pc  : int;
  ret_code : list opcode;
}

val make_closure_vm : list opcode -> list (list opcode) -> closure_vm
let make_closure_vm main_code table = {
  stack = [];
  slots = [];
  pc = 0;
  code = main_code;
  ok = true;
  code_table = table;
  ret_pc = 0;
  ret_code = [];
}

// Top-level pop_and_bind for reuse in tests and closure_eval_op
val pop_and_bind : n:nat -> list lisp_val -> list lisp_val -> Tot (list lisp_val * list lisp_val) (decreases n)
let rec pop_and_bind n stk sl =
  if n = 0 then (stk, sl) else
  match stk with
  | v :: rest -> pop_and_bind (n - 1) rest (v :: sl)
  | [] -> (stk, sl)

// Single step
val closure_eval_op : closure_vm -> closure_vm
let closure_eval_op s =
  match list_nth s.code s.pc with
  | None -> { s with ok = false }
  | Some op ->
    let pc = s.pc + 1 in
    match op with
    | PushI64 n -> { s with stack = Num n :: s.stack; pc = pc }
    | PushBool b -> { s with stack = Bool b :: s.stack; pc = pc }
    | PushNil -> { s with stack = Nil :: s.stack; pc = pc }
    | PushStr str -> { s with stack = Str str :: s.stack; pc = pc }
    | OpAdd ->
      (match s.stack with
       | b :: a :: rest ->
         { s with stack = num_arith a b op_int_add ff_add :: rest; pc = pc }
       | _ -> { s with ok = false })
    | OpSub ->
      (match s.stack with
       | b :: a :: rest ->
         { s with stack = num_arith a b op_int_sub ff_sub :: rest; pc = pc }
       | _ -> { s with ok = false })
    | OpEq ->
      (match s.stack with
       | b :: a :: rest ->
         { s with stack = Bool (lisp_eq a b) :: rest; pc = pc }
       | _ -> { s with ok = false })
    | OpGt ->
      (match s.stack with
       | b :: a :: rest ->
         { s with stack = Bool (num_cmp a b ff_gt op_int_gt) :: rest; pc = pc }
       | _ -> { s with ok = false })
    | OpLt ->
      (match s.stack with
       | b :: a :: rest ->
         { s with stack = Bool (num_cmp a b ff_lt op_int_lt) :: rest; pc = pc }
       | _ -> { s with ok = false })
    | LoadSlot idx ->
      (match list_nth s.slots idx with
       | Some v -> { s with stack = v :: s.stack; pc = pc }
       | None -> { s with ok = false })
    | StoreSlot idx ->
      (match s.stack with
       | v :: rest ->
         (match list_set_or_extend s.slots idx v with
          | Some slots' -> { s with slots = slots'; stack = rest; pc = pc }
          | None -> { s with ok = false })
       | _ -> { s with ok = false })
    | JumpIfFalse addr ->
      (match s.stack with
       | v :: rest ->
         if is_truthy v then { s with stack = rest; pc = pc }
         else { s with stack = rest; pc = addr }
       | _ -> { s with ok = false })
    | Jump addr -> { s with pc = addr }

    // Closure ops
    | PushClosure chunk_idx ->
      { s with stack = Num chunk_idx :: s.stack; pc = pc }

    | CallCaptured (argc, _nlocals) ->
      if argc <> 0 then { s with ok = false } else
      (match s.stack with
       | closure_ref :: rest ->
         (match closure_ref with
          | Num chunk_idx ->
            (match list_nth s.code_table chunk_idx with
             | Some chunk_code ->
               { s with
                 code = chunk_code;
                 pc = 0;
                 stack = rest;
                 slots = [];
                 ret_pc = pc;
                 ret_code = s.code;
               }
             | None -> { s with ok = false })
          | _ -> { s with ok = false })
       | _ -> { s with ok = false })

    | Return ->
      { s with code = s.ret_code; pc = s.ret_pc }

    // Self-call: pop args, bind to slots, restart at PC=0
    | CallSelf argc ->
      let (new_stack, new_slots) = pop_and_bind argc s.stack [] in
      { s with pc = 0; stack = new_stack; slots = new_slots }

    | _ -> { s with pc = pc }

// Multi-step — just step until fuel runs out or error
val closure_eval_steps : int -> closure_vm -> closure_vm
let rec closure_eval_steps fuel s =
  if fuel <= 0 then s else
  if not s.ok then s else
  closure_eval_steps (fuel - 1) (closure_eval_op s)
