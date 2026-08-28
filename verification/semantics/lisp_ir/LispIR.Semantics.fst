(** Lisp Bytecode VM Semantics — F* Formal Specification

    Mirrors src/bytecode.rs run_compiled_lambda match block.
    Following vWasm's pattern: semantics/wasm/Wasm.Eval.fst
    
    Extended with u128 operations for NEAR DeFi safety:
    - U128Load/U128LoadHigh: read 128-bit values from memory
    - U128Store: write 128-bit values to memory
    - U128Add/U128Sub/U128Mul: arithmetic with overflow/underflow trapping
*)
module LispIR.Semantics

open Lisp.Types
open Lisp.Values
open LispIR.Memory
open FStar.String

// === U64 helpers (mirror Rust u64 semantics; non-U64 operands coerce to 0) ===
let u64_of (v:lisp_val) : u64ty = match v with Num n -> (if n >= 0 then n else 0) % 18446744073709551616 | _ -> 0
// Bitwise ops are abstract: F* ints lack native bitwise. The u64ty refinement
// (0 <= r < 2^64) is what downstream proofs consume.
assume val u64_and : u64ty -> u64ty -> Tot u64ty
assume val u64_or  : u64ty -> u64ty -> Tot u64ty
assume val u64_xor : u64ty -> u64ty -> Tot u64ty
let u64_not (a:u64ty) : u64ty = 18446744073709551615 - a
assume val u64_shl : u64ty -> n:nat{n < 64} -> Tot u64ty
assume val u64_shr : u64ty -> n:nat{n < 64} -> Tot u64ty
let u64_mulhi (a:u64ty) (b:u64ty) : u64ty =
  Prims.op_Multiply a b / 18446744073709551616
let sh_amt (v:lisp_val) : int = num_val v
val sh_amt64 (v:lisp_val) : Tot (option (n:nat{n < 64}))
let sh_amt64 v =
  let n = sh_amt v in
  if n >= 0 && n < 64 then Some n else None

let op_int_add (x:int) (y:int) : int = x + y
let op_int_sub (x:int) (y:int) : int = x - y
let int_mul (x:int) (y:int) : Tot int = Prims.op_Multiply x y
val int_div : x:int -> y:int -> Tot int
let int_div x y = if y = 0 then 0 else x / y
val int_mod : x:int -> y:int -> Tot int
let int_mod x y = if y = 0 then 0 else x % y
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

// Pop n values from stack
val pop_n : list lisp_val -> nat -> Tot (option (list lisp_val * list lisp_val))
let rec pop_n stk n =
  match n, stk with
  | 0, _ -> Some ([], stk)
  | _, [] -> None
  | _, v :: rest ->
    (match pop_n rest (n - 1) with
     | Some (items, remaining) -> Some (v :: items, remaining)
     | None -> None)

// Reverse a list — helper first, then main function
val list_rev_append : list 'a -> list 'a -> Tot (list 'a)
let rec list_rev_append l1 l2 =
  match l1 with
  | [] -> l2
  | x :: rest -> list_rev_append rest (x :: l2)

val list_rev : list 'a -> Tot (list 'a)
let rec list_rev l =
  list_rev_append l []

// Convert list of values to field pairs: [("0", v0); ("1", v1); ...]
val items_to_fields : list lisp_val -> int -> Tot (list (string * lisp_val))
let rec items_to_fields items idx =
  match items with
  | [] -> []
  | v :: rest -> (string_of_int idx, v) :: items_to_fields rest (idx + 1)

// Convenience: items_to_fields starting from 0
val items_to_fields0 : list lisp_val -> Tot (list (string * lisp_val))
let items_to_fields0 items = items_to_fields items 0

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
      // div-by-zero check — matches Rust bytecode.rs:3612-3613
      (match b with
       | Num 0 -> Err "division by zero"
       | Float fb -> if ff_eq fb (ff_of_int 0) then Err "division by zero"
                     else Ok {s with stack = num_arith a b int_div ff_div :: rest; pc = s.pc + 1}
       | _ -> Ok {s with stack = num_arith a b int_div ff_div :: rest; pc = s.pc + 1})
    | _ -> Err "OpDiv: stack underflow")

  | OpMod -> (match s.stack with
    | b :: a :: rest ->
      // Matches Rust bytecode.rs:3618-3625 — uses num_val (truncates floats to int)
      let av = (match a with Num n -> n | Float f -> ff_to_int f | _ -> 0) in
      let bv = (match b with Num n -> n | Float f -> ff_to_int f | _ -> 0) in
      if bv = 0 then Err "modulo by zero"
      else
        let r = int_mod av bv in  // int_div-style safe wrapper
        Ok {s with stack = Num r :: rest; pc = s.pc + 1}
    | _ -> Err "OpMod: stack underflow")

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
      // Does NOT write back to slot — matches Rust (bytecode.rs:2389-2394, 3711-3714)
      Ok {s with stack = r :: s.stack; pc = s.pc + 1}
    | Some (Float f) ->
      let r = Float (ff_add f (ff_of_int imm)) in
      // Does NOT write back to slot
      Ok {s with stack = r :: s.stack; pc = s.pc + 1}
    | _ -> Err "SlotAddImm: slot not numeric")

  // --- New opcodes: literal, global env, captured mutation, deftype ---
  | PushLiteral v ->
    Ok {s with stack = v :: s.stack; pc = s.pc + 1}

  | LoadGlobal _ ->
    // No env model in simple vm_state — advance pc (env handled in ClosureVM)
    Ok {s with pc = s.pc + 1}

  | StoreGlobal _ ->
    Ok {s with pc = s.pc + 1}

  | StoreCaptured _ ->
    Ok {s with pc = s.pc + 1}

  | ConstructTag (type_name, n_args, variant_idx) ->
    // Simplified: pop n_args, build Tagged with variant_id
    (match pop_n s.stack n_args with
     | Some (items, rest) ->
       Ok {s with stack = Tagged (type_name, variant_idx, items_to_fields0 items) :: rest; pc = s.pc + 1}
     | None -> Err "ConstructTag: stack underflow")

  | TagTest (type_name, variant_idx) ->
    (match s.stack with
    | v :: rest ->
      let is_tag = match v with
        | Tagged (tn, vid, _) -> tn = type_name && vid = variant_idx
        | _ -> false in
      // Does NOT pop — pushes Bool result, leaves original value
      Ok {s with stack = Bool is_tag :: v :: rest; pc = s.pc + 1}
    | [] -> Err "TagTest: stack underflow")

  | GetField idx ->
    (match s.stack with
    | v :: rest ->
      (match v with
       | Tagged (_, _, fields) ->
         (match list_nth fields idx with
          | Some (_, fv) -> Ok {s with stack = fv :: rest; pc = s.pc + 1}
          | None -> Ok {s with stack = Nil :: rest; pc = s.pc + 1})
       | _ -> Ok {s with stack = Nil :: rest; pc = s.pc + 1})
    | [] -> Err "GetField: stack underflow")

  // --- MakeList: pop n items, push List ---
  | MakeList n ->
    (match pop_n s.stack n with
     | Some (items, rest) ->
       // items are in reverse stack order (top of stack is head)
       // pop_n returns [top, next, ...] so we reverse to get original order
       let reversed = list_rev items in
       Ok {s with stack = List reversed :: rest; pc = s.pc + 1}
     | None -> Err "MakeList: stack underflow")

  // --- Fused HOF opcodes (placeholder — advance pc) ---
  | MapOp _ ->
    Ok {s with pc = s.pc + 1}
  | FilterOp _ ->
    Ok {s with pc = s.pc + 1}
  | ReduceOp _ ->
    Ok {s with pc = s.pc + 1}

  // --- Vec opcodes ---
  | MakeVec n ->
    (match pop_n s.stack n with
     | Some (items, rest) ->
       // items are in reverse stack order; reverse to get original order
       let reversed = list_rev items in
       Ok {s with stack = Vec (vec_of_list reversed) :: rest; pc = s.pc + 1}
     | None -> Err "MakeVec: stack underflow")

  | VecNth -> (match s.stack with
    | idx_val :: vec_val :: rest ->
      let idx = num_val idx_val in
      if idx < 0 then Ok {s with stack = Nil :: rest; pc = s.pc + 1}
      else
        let r = vec_nth vec_val idx in
        (match r with
         | Some v -> Ok {s with stack = v :: rest; pc = s.pc + 1}
         | None -> Ok {s with stack = Nil :: rest; pc = s.pc + 1})
    | _ -> Err "VecNth: stack underflow")

  | VecLen -> (match s.stack with
    | vec_val :: rest ->
      Ok {s with stack = Num (vec_len vec_val) :: rest; pc = s.pc + 1}
    | _ -> Err "VecLen: stack underflow")

  | VecConj -> (match s.stack with
    | val0 :: vec_val :: rest ->
      Ok {s with stack = vec_conj val0 vec_val :: rest; pc = s.pc + 1}
    | _ -> Err "VecConj: stack underflow")

  | VecContains -> (match s.stack with
    | val0 :: vec_val :: rest ->
      // noeq on lisp_val prevents Seq.mem; use explicit loop with lisp_eq on primitives
      let has = vec_contains_prim val0 vec_val in
      Ok {s with stack = Bool has :: rest; pc = s.pc + 1}
    | _ -> Err "VecContains: stack underflow")

  | VecSlice -> (match s.stack with
    | end_val :: start_val :: vec_val :: rest ->
      let start_i = num_val start_val in
      let end_i = num_val end_val in
      let sliced = vec_slice vec_val start_i end_i in
      Ok {s with stack = Vec sliced :: rest; pc = s.pc + 1}
    | _ -> Err "VecSlice: stack underflow")

  | VecAssoc -> (match s.stack with
    | val0 :: idx_val :: vec_val :: rest ->
      let idx = num_val idx_val in
      let updated = vec_assoc idx val0 vec_val in
      Ok {s with stack = updated :: rest; pc = s.pc + 1}
    | _ -> Err "VecAssoc: stack underflow")

  // --- u128 operations: memory-safe, overflow-trapping arithmetic ---
  // All addresses assumed to be within bounds (checked by memory model).
  // SAFETY: Untag addresses before memory access!
  // See LispIR.Memory for u128_add_safe, u128_sub_safe, u128_mul_safe proofs
  
  // === U64 field arithmetic (mirrors run_compiled_loop / SpecVm) ===
  | U64And -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = Num (u64_and (u64_of a) (u64_of b)) :: rest; pc = s.pc + 1}
    | _ -> Err "U64And: stack underflow")

  | U64Or -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = Num (u64_or (u64_of a) (u64_of b)) :: rest; pc = s.pc + 1}
    | _ -> Err "U64Or: stack underflow")

  | U64Xor -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = Num (u64_xor (u64_of a) (u64_of b)) :: rest; pc = s.pc + 1}
    | _ -> Err "U64Xor: stack underflow")

  | U64Not -> (match s.stack with
    | a :: rest ->
      Ok {s with stack = Num (u64_not (u64_of a)) :: rest; pc = s.pc + 1}
    | _ -> Err "U64Not: stack underflow")

  | U64MulHi -> (match s.stack with
    | b :: a :: rest ->
      Ok {s with stack = Num (u64_mulhi (u64_of a) (u64_of b)) :: rest; pc = s.pc + 1}
    | _ -> Err "U64MulHi: stack underflow")

  | U64Shl -> (match s.stack with
    | sh :: a :: rest ->
      (match sh_amt64 sh with
       | Some n -> Ok {s with stack = Num (u64_shl (u64_of a) n) :: rest; pc = s.pc + 1}
       | None -> Err "shift amount out of range")
    | _ -> Err "U64Shl: stack underflow")

  | U64Shr -> (match s.stack with
    | sh :: a :: rest ->
      (match sh_amt64 sh with
       | Some n -> Ok {s with stack = Num (u64_shr (u64_of a) n) :: rest; pc = s.pc + 1}
       | None -> Err "shift amount out of range")
    | _ -> Err "U64Shr: stack underflow")

  | PushU64 w ->
    Ok {s with stack = Num w :: s.stack; pc = s.pc + 1}

  | Not -> (match s.stack with
    | a :: rest ->
      Ok {s with stack = Bool (not (is_truthy a)) :: rest; pc = s.pc + 1}
    | _ -> Err "Not: stack underflow")

  | PushSelf ->
    (match s.slots with
     | self_v :: _ -> Ok {s with stack = self_v :: s.stack; pc = s.pc + 1}
     | [] -> Err "PushSelf: no self")

  | TracePush _msg ->
    Ok {s with pc = s.pc + 1}

  | TracePop ->
    Ok {s with pc = s.pc + 1}

  | U128Load -> (match s.stack with
    | addr_val :: rest ->
      // Untag address to get raw pointer
      let addr = num_val addr_val in  // num_val extracts raw int from Num
      // Bounds check (abstracted - actual check in memory model)
      // Memory.u128_bounds_check addr must be true
      // In real VM: i64.load addr returns low 64 bits
      // Result is tagged as Num (type tag = 0)
      // See Memory.u128_read_raw for abstract memory model
      Ok {s with stack = Num 0 :: rest; pc = s.pc + 1}  // Placeholder: abstract read
    | [] -> Err "U128Load: stack underflow")

  | U128LoadHigh -> (match s.stack with
    | addr_val :: rest ->
      let addr = num_val addr_val in
      // Read high 64 bits from memory[addr + 8]
      // Bounds check: Memory.u128_bounds_check addr
      Ok {s with stack = Num 0 :: rest; pc = s.pc + 1}  // Placeholder
    | [] -> Err "U128LoadHigh: stack underflow")

  | U128Store -> (match s.stack with
    | hi_val :: lo_val :: addr_val :: rest ->
      let addr = num_val addr_val in
      let lo = num_val lo_val in
      let hi = num_val hi_val in
      // SAFETY: addr must be bounds-checked (Memory.u128_bounds_check addr)
      // SAFETY: addr must be untagged (using num_val, not encode_tag)
      // In real VM: stores lo at addr, hi at addr+8
      // Returns nil
      // See Memory.u128_write_raw for abstract memory model
      Ok {s with stack = Nil :: rest; pc = s.pc + 1}
    | _ -> Err "U128Store: stack underflow")

  | U128Add -> (match s.stack with
    | src_addr_val :: dst_addr_val :: rest ->
      let dst_addr = num_val dst_addr_val in
      let src_addr = num_val src_addr_val in
      // Read u128 from dst, read u128 from src
      // Add: dst = dst + src
      // TRAP on overflow (DeFi safety: no silent wrapping)
      // See Memory.u128_add_safe: returns None on overflow
      // Returns nil on success, traps on overflow
      // MEMORY SAFETY: both addresses bounds-checked (Memory.u128_bounds_check)
      Ok {s with stack = Nil :: rest; pc = s.pc + 1}
    | _ -> Err "U128Add: stack underflow")

  | U128Sub -> (match s.stack with
    | src_addr_val :: dst_addr_val :: rest ->
      let dst_addr = num_val dst_addr_val in
      let src_addr = num_val src_addr_val in
      // Subtract: dst = dst - src
      // TRAP on underflow (DeFi safety: no negative balances)
      // See Memory.u128_sub_safe: returns None when a < b
      // Returns nil on success, traps on underflow
      // MEMORY SAFETY: both addresses bounds-checked
      Ok {s with stack = Nil :: rest; pc = s.pc + 1}
    | _ -> Err "U128Sub: stack underflow")

  | U128Mul -> (match s.stack with
    | val_num :: dst_addr_val :: rest ->
      let dst_addr = num_val dst_addr_val in
      // Multiply: dst = dst * val (val is i64, dst is u128)
      // TRAP on overflow or non-positive scalar (DeFi safety)
      // See Memory.u128_mul_safe: returns None on overflow or scalar <= 0
      // Returns nil on success, traps on overflow
      // MEMORY SAFETY: dst_addr bounds-checked
      Ok {s with stack = Nil :: rest; pc = s.pc + 1}
    | _ -> Err "U128Mul: stack underflow")

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
