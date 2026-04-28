(** Lisp Bytecode VM Semantics — F* Formal Specification

    Mirrors src/bytecode.rs run_compiled_lambda match block.
    Following vWasm's pattern: semantics/wasm/Wasm.Eval.fst
*)
module LispIR.Semantics

open Lisp.Types
open Lisp.Values

let op_int_add (x:int) (y:int) : int = x + y
let op_int_sub (x:int) (y:int) : int = x - y
let int_mul (x:int) (y:int) : Tot int = Prims.op_Multiply x y
assume val int_div : int -> int -> Tot int
let op_int_lt  (x:int) (y:int) : bool = x < y
let op_int_le  (x:int) (y:int) : bool = x <= y
let op_int_gt  (x:int) (y:int) : bool = x > y
let op_int_ge  (x:int) (y:int) : bool = x >= y

noeq type vm_result =
  | Ok of vm_state
  | Err of string

// === Helpers (int-indexed) ===

val list_nth : list 'a -> int -> Tot (option 'a)
let rec list_nth lst n =
  if n < 0 then None else
  match lst, n with
  | [], _ -> None
  | x :: _, 0 -> Some x
  | _ :: rest, n -> list_nth rest (n - 1)

val list_update : list 'a -> int -> 'a -> Tot (option (list 'a))
let rec list_update lst idx v2 =
  if idx < 0 then None else
  match lst, idx with
  | [], _ -> None
  | x :: rest, 0 -> Some (v2 :: rest)
  | x :: rest, n -> (match list_update rest (n - 1) v2 with
                       | Some lst -> Some (x :: lst)
                       | None -> None)

// Extend-or-update: if idx = len(lst), append; if idx < len, update; else None
val list_set_or_extend : list 'a -> int -> 'a -> Tot (option (list 'a))
let rec list_set_or_extend lst idx v2 =
  if idx < 0 then None else
  match lst, idx with
  | [], 0 -> Some [v2]
  | [], _ -> None
  | x :: rest, 0 -> Some (v2 :: rest)
  | x :: rest, n -> (match list_set_or_extend rest (n - 1) v2 with
                       | Some lst -> Some (x :: lst)
                       | None -> None)

val val_of_dict : lisp_val -> Tot (list (string * lisp_val))
let val_of_dict v =
  match v with
  | Dict entries -> entries
  | _ -> []

// === Core: evaluate one opcode ===
val eval_op : opcode -> vm_state -> Tot vm_result
let eval_op op s =
  if not s.ok then Ok s else
  match op with

  | PushI64 n ->
    Ok {s with stack = Num n :: s.stack; pc = s.pc + 1}

  | PushFloat f ->
    Ok {s with stack = Float f :: s.stack; pc = s.pc + 1}

  | PushBool b ->
    Ok {s with stack = Bool b :: s.stack; pc = s.pc + 1}

  | PushNil ->
    Ok {s with stack = Nil :: s.stack; pc = s.pc + 1}

  | PushStr str ->
    Ok {s with stack = Str str :: s.stack; pc = s.pc + 1}

  | Dup -> (match s.stack with
    | v :: rest -> Ok {s with stack = v :: v :: rest; pc = s.pc + 1}
    | [] -> Err "Dup: stack underflow")

  | Pop -> (match s.stack with
    | _ :: rest -> Ok {s with stack = rest; pc = s.pc + 1}
    | [] -> Err "Pop: stack underflow")

  | LoadSlot idx -> (match list_nth s.slots idx with
    | Some v -> Ok {s with stack = v :: s.stack; pc = s.pc + 1}
    | None -> Err "LoadSlot: index out of bounds")

  | StoreSlot idx -> (match s.stack with
    | v :: rest -> (match list_set_or_extend s.slots idx v with
      | Some slots' -> Ok {s with stack = rest; slots = slots'; pc = s.pc + 1}
      | None -> Err "StoreSlot: index out of bounds")
    | [] -> Err "StoreSlot: stack underflow")

  | OpAdd -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = num_arith a b (op_int_add) ff_add :: rest; pc = s.pc + 1}
    | _ -> Err "OpAdd: stack underflow")

  | OpSub -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = num_arith a b (op_int_sub) ff_sub :: rest; pc = s.pc + 1}
    | _ -> Err "OpSub: stack underflow")

  | OpMul -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = num_arith a b int_mul ff_mul :: rest; pc = s.pc + 1}
    | _ -> Err "OpMul: stack underflow")

  | OpDiv -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = num_arith a b int_div ff_div :: rest; pc = s.pc + 1}
    | _ -> Err "OpDiv: stack underflow")

  | OpEq -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = Bool (lisp_eq a b) :: rest; pc = s.pc + 1}
    | _ -> Err "OpEq: stack underflow")

  | OpLt -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = Bool (num_cmp a b ff_lt (op_int_lt)) :: rest; pc = s.pc + 1}
    | _ -> Err "OpLt: stack underflow")

  | OpLe -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = Bool (num_cmp a b ff_le (op_int_le)) :: rest; pc = s.pc + 1}
    | _ -> Err "OpLe: stack underflow")

  | OpGt -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = Bool (num_cmp a b ff_gt (op_int_gt)) :: rest; pc = s.pc + 1}
    | _ -> Err "OpGt: stack underflow")

  | OpGe -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = Bool (num_cmp a b ff_ge (op_int_ge)) :: rest; pc = s.pc + 1}
    | _ -> Err "OpGe: stack underflow")

  | Jump addr ->
    Ok {s with pc = addr}

  | JumpIfTrue addr -> (match s.stack with
    | v :: rest ->
      if is_truthy v
      then Ok {s with stack = rest; pc = addr}
      else Ok {s with stack = rest; pc = s.pc + 1}
    | [] -> Err "JumpIfTrue: stack underflow")

  | JumpIfFalse addr -> (match s.stack with
    | v :: rest ->
      if not (is_truthy v)
      then Ok {s with stack = rest; pc = addr}
      else Ok {s with stack = rest; pc = s.pc + 1}
    | [] -> Err "JumpIfFalse: stack underflow")

  | Return -> (match s.stack with
    | v :: _ -> Ok {s with stack = [v]; ok = false}
    | [] -> Err "Return: stack underflow")

  | DictGet -> (match s.stack with
    | key :: map :: rest -> (match key with
      | Str k -> Ok {s with stack = dict_get k (val_of_dict map) :: rest; pc = s.pc + 1}
      | _ -> Ok {s with stack = Nil :: rest; pc = s.pc + 1})
    | _ -> Err "DictGet: stack underflow")

  | DictSet -> (match s.stack with
    | v :: key :: map :: rest -> (match key with
      | Str k -> Ok {s with stack = Dict (dict_set k v (val_of_dict map)) :: rest; pc = s.pc + 1}
      | _ -> Err "DictSet: key not a string")
    | _ -> Err "DictSet: stack underflow")

  | SlotGtImm (idx, imm) -> (match list_nth s.slots idx with
    | Some (Num n) ->
      Ok {s with stack = Bool (n > imm) :: s.stack; pc = s.pc + 1}
    | Some (Float f) ->
      Ok {s with stack = Bool (ff_gt f (ff_of_int imm)) :: s.stack; pc = s.pc + 1}
    | _ -> Err "SlotGtImm: slot not numeric or out of bounds")

  | SlotLtImm (idx, imm) -> (match list_nth s.slots idx with
    | Some (Num n) ->
      Ok {s with stack = Bool (n < imm) :: s.stack; pc = s.pc + 1}
    | Some (Float f) ->
      Ok {s with stack = Bool (ff_lt f (ff_of_int imm)) :: s.stack; pc = s.pc + 1}
    | _ -> Err "SlotLtImm: slot not numeric or out of bounds")

  | SlotLeImm (idx, imm) -> (match list_nth s.slots idx with
    | Some (Num n) ->
      Ok {s with stack = Bool (n <= imm) :: s.stack; pc = s.pc + 1}
    | Some (Float f) ->
      Ok {s with stack = Bool (ff_le f (ff_of_int imm)) :: s.stack; pc = s.pc + 1}
    | _ -> Err "SlotLeImm: slot not numeric or out of bounds")

  | SlotGeImm (idx, imm) -> (match list_nth s.slots idx with
    | Some (Num n) ->
      Ok {s with stack = Bool (n >= imm) :: s.stack; pc = s.pc + 1}
    | Some (Float f) ->
      Ok {s with stack = Bool (ff_ge f (ff_of_int imm)) :: s.stack; pc = s.pc + 1}
    | _ -> Err "SlotGeImm: slot not numeric or out of bounds")

  | SlotAddImm (idx, imm) -> (match list_nth s.slots idx with
    | Some (Num n) ->
      let r = Num (n + imm) in
      (match list_update s.slots idx r with
       | Some slots' -> Ok {s with slots = slots'; stack = r :: s.stack; pc = s.pc + 1}
       | None -> Err "SlotAddImm: update failed")
    | Some (Float f) ->
      let r = Float (ff_add f (ff_of_int imm)) in
      (match list_update s.slots idx r with
       | Some slots' -> Ok {s with slots = slots'; stack = r :: s.stack; pc = s.pc + 1}
       | None -> Err "SlotAddImm: update failed")
    | _ -> Err "SlotAddImm: slot not numeric")

  // --- Default: advance PC for all remaining ops ---
  | _ -> Ok {s with pc = s.pc + 1}

// === Multi-step evaluation ===
val eval_steps : n:nat -> vm_state -> Tot vm_result
let rec eval_steps n s =
  match n with
  | 0 -> Ok s
  | _ -> (match list_nth s.code s.pc with
    | None -> Ok s
    | Some op -> (match eval_op op s with
      | Err msg -> Err msg
      | Ok s' -> if not s'.ok then Ok s' else eval_steps (n - 1) s'))
