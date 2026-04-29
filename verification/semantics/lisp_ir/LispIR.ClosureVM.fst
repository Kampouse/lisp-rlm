(** Closures in the VM — Frame Stack Model
    
    Replaces single ret_pc/ret_code with a list of saved frames.
    Mirrors the Rust VM's Vec<Frame> for iterative CallSelf.
    
    CallSelf: push current frame (pc+1, slots, stack), fresh slots/stack, pc=0
    Return: pop frame, push return value to caller's stack, restore pc/slots/stack
    CallCaptured: push frame, switch to code_table chunk, fresh slots
    PushClosure: push chunk index as Num
    
    Frame stack depth is bounded by fuel in closure_eval_steps.
*)
module LispIR.ClosureVM

open Lisp.Types
open Lisp.Values
open LispIR.Semantics
open FStar.List.Tot

// === Chunk: code + metadata for a compiled lambda ===
noeq type chunk = {
  chunk_code : list opcode;
  chunk_nslots : nat;
  // Runtime captures: (name, parent_slot_index) — PushClosure reads s.slots[parent_slot_index]
  chunk_runtime_captures : list (string * nat);
}

// === Frame: saved caller state ===
noeq type frame = {
  ret_pc        : int;
  ret_slots     : list lisp_val;
  ret_stack     : list lisp_val;
  ret_code      : list opcode;
  ret_num_slots : nat;
  ret_captured  : list lisp_val;
}

noeq type closure_vm = {
  stack      : list lisp_val;
  slots      : list lisp_val;
  pc         : int;
  code       : list opcode;
  ok         : bool;
  code_table : list chunk;
  frames     : list frame;
  // Total slot count for the current function (for CallSelf fresh slot alloc)
  num_slots  : nat;
  // Current function's captured values (Populated by CallCaptured/CallCapturedRef)
  captured   : list lisp_val;
  // Per-instance closure storage: (captured_values, chunk_index)
  closure_envs : list (list lisp_val * nat);
}

val make_closure_vm : list opcode -> list chunk -> nat -> closure_vm
let make_closure_vm main_code table nslots = {
  stack = [];
  slots = [];
  pc = 0;
  code = main_code;
  ok = true;
  code_table = table;
  frames = [];
  num_slots = nslots;
  captured = [];
  closure_envs = [];
}

// Pop n values from stack, return (remaining_stack, popped_in_order)
val pop_and_bind : n:nat -> list lisp_val -> list lisp_val -> Tot (list lisp_val * list lisp_val) (decreases n)
let rec pop_and_bind n stk acc =
  if n = 0 then (stk, acc) else
  match stk with
  | v :: rest -> pop_and_bind (n - 1) rest (v :: acc)
  | [] -> (stk, acc)

// Build fresh slot array of given size, filling first n_args from args list
// Non-recursive for n<=3 (covers all current proof cases) to help Z3
val fill_slots : n:nat -> list lisp_val -> list lisp_val
let rec fill_slots n args =
  match n with
  | 0 -> []
  | 1 -> (match args with | a :: _ -> [a] | [] -> [Nil])
  | 2 -> (match args with
          | a :: b :: _ -> [a; b]
          | a :: [] -> [a; Nil]
          | [] -> [Nil; Nil])
  | 3 -> (match args with
          | a :: b :: c :: _ -> [a; b; c]
          | a :: b :: [] -> [a; b; Nil]
          | a :: [] -> [a; Nil; Nil]
          | [] -> [Nil; Nil; Nil])
  | _ -> (match args with
          | a :: rest -> a :: fill_slots (n - 1) rest
          | [] -> Nil :: fill_slots (n - 1) [])

// Pad a list to length n with Nil
val pad_slots : n:nat -> list lisp_val -> list lisp_val
let rec pad_slots n sl =
  match n with
  | 0 -> []
  | _ -> (match sl with
          | v :: rest -> v :: pad_slots (n - 1) rest
          | [] -> Nil :: pad_slots (n - 1) [])

// Build captured values list from runtime_captures: read s.slots[idx] for each (name, idx)
val build_captured : list (string * nat) -> list lisp_val -> list lisp_val
let rec build_captured rtcaps slots =
  match rtcaps with
  | [] -> []
  | (_, idx) :: rest ->
    let v = match list_nth slots idx with Some x -> x | None -> Nil in
    v :: build_captured rest slots

// Extracted handler for CallSelf — helps Z3 unfold fill_slots
val callself_handler : argc:nat -> s:closure_vm -> pc:nat -> closure_vm
let callself_handler argc s pc =
  let (new_stack, args) = pop_and_bind argc s.stack [] in
  let caller_frame : frame = {
    ret_pc = pc;
    ret_slots = s.slots;
    ret_stack = new_stack;
    ret_code = s.code;
    ret_num_slots = s.num_slots;
    ret_captured = s.captured;
  } in
  { s with
    pc = 0;
    stack = [];
    slots = fill_slots s.num_slots args;
    frames = caller_frame :: s.frames;
  }

// Extracted handler for Recur
val recur_handler : nslots_arg:nat -> s:closure_vm -> closure_vm
let recur_handler nslots_arg s =
  let (remaining, args) = pop_and_bind nslots_arg s.stack [] in
  { s with pc = 0; slots = fill_slots nslots_arg args; stack = [] }

// Extracted handler for RecurIncAccum
val recurincaccum_handler : counter:nat -> accum:nat -> step:int -> limit:int -> exit_addr:nat -> s:closure_vm -> closure_vm
let recurincaccum_handler counter accum step limit exit_addr s =
  (match list_nth s.slots counter with
   | Some (Num cv) ->
     if cv >= limit then { s with pc = exit_addr }
     else
       (match list_nth s.slots accum with
        | Some (Num av) ->
          let new_accum = av + cv in
          let new_counter = cv + step in
          let slots1 = (match list_set_or_extend s.slots accum (Num new_accum) with
                        | Some sl -> sl | None -> s.slots) in
          let slots2 = (match list_set_or_extend slots1 counter (Num new_counter) with
                        | Some sl -> sl | None -> slots1) in
          { s with slots = slots2; pc = 0 }
        | _ -> { s with ok = false })
   | _ -> { s with ok = false })

// Extracted handler for CallCaptured
val callcaptured_handler : argc:nat -> s:closure_vm -> pc:nat -> closure_vm
let callcaptured_handler argc s pc =
  let (remaining, args) = pop_and_bind argc s.stack [] in
  (match remaining with
   | closure_ref :: rest ->
     (match closure_ref with
      | Num inst_id ->
        (match list_nth s.closure_envs inst_id with
         | Some (caps, chunk_idx) ->
           (match list_nth s.code_table chunk_idx with
            | Some ch ->
              let caller_frame : frame = {
                ret_pc = pc;
                ret_slots = s.slots;
                ret_stack = rest;
                ret_code = s.code;
                ret_num_slots = s.num_slots;
                ret_captured = s.captured;
              } in
              { s with
                code = ch.chunk_code;
                pc = 0;
                stack = [];
                slots = pad_slots ch.chunk_nslots args;
                num_slots = ch.chunk_nslots;
                captured = caps;
                frames = caller_frame :: s.frames;
              }
            | None -> { s with ok = false })
         | None -> { s with ok = false })
      | _ -> { s with ok = false })
   | _ -> { s with ok = false })

// Extracted handler for CallCapturedRef
val callcapturedref_handler : idx:nat -> argc:nat -> s:closure_vm -> pc:nat -> closure_vm
let callcapturedref_handler idx argc s pc =
  (match list_nth s.slots idx with
   | Some (Num inst_id) ->
     let (remaining, args) = pop_and_bind argc s.stack [] in
     (match list_nth s.closure_envs inst_id with
      | Some (caps, chunk_idx) ->
        (match list_nth s.code_table chunk_idx with
         | Some ch ->
           let caller_frame : frame = {
             ret_pc = pc;
             ret_slots = s.slots;
             ret_stack = remaining;
             ret_code = s.code;
             ret_num_slots = s.num_slots;
             ret_captured = s.captured;
           } in
           { s with
             code = ch.chunk_code;
             pc = 0;
             stack = [];
             slots = pad_slots ch.chunk_nslots args;
             num_slots = ch.chunk_nslots;
             captured = caps;
             frames = caller_frame :: s.frames;
           }
         | None -> { s with ok = false })
      | None -> { s with ok = false })
   | _ -> { s with ok = false })

// Extracted handler for Return
val return_handler : s:closure_vm -> closure_vm
let return_handler s =
  (match s.stack with
   | retval :: _ ->
     (match s.frames with
      | caller :: rest ->
        { s with
          pc = caller.ret_pc;
          slots = caller.ret_slots;
          stack = retval :: caller.ret_stack;
          code = caller.ret_code;
          num_slots = caller.ret_num_slots;
          captured = caller.ret_captured;
          frames = rest;
        }
      | [] -> s)
   | [] -> s)

// Typed arithmetic: individual result functions per binop (Z3 can unfold each)
val typed_add_i64 : a:int -> b:int -> lisp_val
let typed_add_i64 a b = Num (a + b)

val typed_sub_i64 : a:int -> b:int -> lisp_val
let typed_sub_i64 a b = Num (a - b)

val typed_mul_i64 : a:int -> b:int -> lisp_val
let typed_mul_i64 a b = Num (int_mul a b)

val typed_eq_i64 : a:int -> b:int -> lisp_val
let typed_eq_i64 a b = Bool (a = b)

val typed_lt_i64 : a:int -> b:int -> lisp_val
let typed_lt_i64 a b = Bool (a < b)

val typed_le_i64 : a:int -> b:int -> lisp_val
let typed_le_i64 a b = Bool (a <= b)

val typed_gt_i64 : a:int -> b:int -> lisp_val
let typed_gt_i64 a b = Bool (a > b)

val typed_ge_i64 : a:int -> b:int -> lisp_val
let typed_ge_i64 a b = Bool (a >= b)

val typed_mod_i64 : a:int -> b:int -> lisp_val
let typed_mod_i64 a b = if b = 0 then Num 0 else Num (a % b)

val typed_add_f64 : a:int -> b:int -> lisp_val
let typed_add_f64 a b = Num (a + b)

val typed_sub_f64 : a:int -> b:int -> lisp_val
let typed_sub_f64 a b = Num (a - b)

// Dispatch: maps (binop, ty) to the right result function
val typedbinop_result : binop -> ty -> lisp_val -> lisp_val -> Tot lisp_val
let typedbinop_result binop ty a b =
  match ty with
  | I64 ->
    (match a, b with
     | Num na, Num nb ->
       (match binop with
        | Add -> typed_add_i64 na nb
        | Sub -> typed_sub_i64 na nb
        | Mul -> typed_mul_i64 na nb
        | Div -> Num na  // simplified
        | Eq -> typed_eq_i64 na nb
        | Lt -> typed_lt_i64 na nb
        | Le -> typed_le_i64 na nb
        | Gt -> typed_gt_i64 na nb
        | Ge -> typed_ge_i64 na nb
        | Mod -> typed_mod_i64 na nb)
     | _ -> Nil)
  | F64 ->
    (match a, b with
     | Num na, Num nb ->
       (match binop with
        | Add -> typed_add_f64 na nb
        | Sub -> typed_sub_f64 na nb
        | _ -> Num na)  // simplified
     | _ -> Nil)

// Pure builtin computation — extracted for Z3-friendliness
val builtin_result : name:string -> args:list lisp_val -> lisp_val
let builtin_result name args =
  (match name with
   | "length" ->
     (match args with
      | [List items] -> Num (List.length items)
      | [Nil] -> Num 0
      | _ -> Num 0)
   | "append" ->
     (match args with
      | [List a; List b] -> List (a @ b)
      | [List a; Nil] -> List a
      | [Nil; List b] -> List b
      | _ -> List [])
   | "car" ->
     (match args with
      | [List (h :: _)] -> h
      | _ -> Nil)
   | "cdr" ->
     (match args with
      | [List (_ :: t)] -> List t
      | _ -> Nil)
   | "cons" ->
     (match args with
      | [v; List lst] -> List (v :: lst)
      | [v; Nil] -> List [v]
      | _ -> Nil)
   | "list" -> List args
   | "str-concat" ->
     (match args with
      | [Str a; Str b] -> Str (a ^ b)
      | _ -> Str "")
   | "abs" ->
     (match args with
      | [Num n] -> Num (if n < 0 then -n else n)
      | _ -> Num 0)
   | "min" ->
     (match args with
      | [Num a; Num b] -> Num (if a < b then a else b)
      | _ -> Num 0)
   | "max" ->
     (match args with
      | [Num a; Num b] -> Num (if a > b then a else b)
      | _ -> Num 0)
   | _ -> Nil)

// Single step
val closure_eval_op : closure_vm -> closure_vm
let closure_eval_op s =
  match list_nth s.code s.pc with
  | None -> { s with ok = false }
  | Some op ->
    let pc = s.pc + 1 in
    match op with
    | CallSelf argc -> callself_handler argc s pc

    // Return: pop frame, restore caller state
    | Return -> return_handler s

    // Self-call: push current frame, fresh slots/stack, bind args, pc=0
    | Recur nslots -> recur_handler nslots s

    // RecurIncAccum: peephole-optimized loop counter increment
    | RecurIncAccum (counter, accum, step, limit, exit_addr) ->
      recurincaccum_handler counter accum step limit exit_addr s

    // Remaining slot-immediate fused ops
    | TypedBinOp (binop, ty) ->
      (match s.stack with
       | b :: a :: rest ->
         (match ty with
          | I64 ->
            (match a, b with
             | Num na, Num nb ->
               (match binop with
                | Add -> { s with stack = typed_add_i64 na nb :: rest; pc = pc }
                | Sub -> { s with stack = typed_sub_i64 na nb :: rest; pc = pc }
                | Mul -> { s with stack = typed_mul_i64 na nb :: rest; pc = pc }
                | Div -> { s with stack = Num na :: rest; pc = pc }
                | Eq -> { s with stack = typed_eq_i64 na nb :: rest; pc = pc }
                | Lt -> { s with stack = typed_lt_i64 na nb :: rest; pc = pc }
                | Le -> { s with stack = typed_le_i64 na nb :: rest; pc = pc }
                | Gt -> { s with stack = typed_gt_i64 na nb :: rest; pc = pc }
                | Ge -> { s with stack = typed_ge_i64 na nb :: rest; pc = pc }
                | Mod -> { s with stack = typed_mod_i64 na nb :: rest; pc = pc })
             | _ -> { s with ok = false })
          | F64 ->
            (match a, b with
             | Num na, Num nb ->
               (match binop with
                | Add -> { s with stack = typed_add_f64 na nb :: rest; pc = pc }
                | Sub -> { s with stack = typed_sub_f64 na nb :: rest; pc = pc }
                | _ -> { s with stack = Num na :: rest; pc = pc })
             | _ -> { s with ok = false }))
       | _ -> { s with ok = false })

    // Division and modulo
    | PushI64 n -> { s with stack = Num n :: s.stack; pc = pc }
    | PushBool b -> { s with stack = Bool b :: s.stack; pc = pc }
    | PushNil -> { s with stack = Nil :: s.stack; pc = pc }
    | JumpIfFalse addr ->
      (match s.stack with
       | v :: rest ->
         if is_truthy v then { s with stack = rest; pc = pc }
         else { s with stack = rest; pc = addr }
       | _ -> { s with ok = false })
    | Jump addr -> { s with pc = addr }

    // Dict ops
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
    | OpMul ->
      (match s.stack with
       | b :: a :: rest ->
         { s with stack = num_arith a b int_mul ff_mul :: rest; pc = pc }
       | _ -> { s with ok = false })
    | PushFloat _ -> { s with stack = Float (ff_of_int 0) :: s.stack; pc = pc }
    | PushStr str -> { s with stack = Str str :: s.stack; pc = pc }
    | Dup ->
      (match s.stack with
       | v :: rest -> { s with stack = v :: v :: rest; pc = pc }
       | [] -> { s with ok = false })
    | Pop ->
      (match s.stack with
       | _ :: rest -> { s with stack = rest; pc = pc }
       | [] -> { s with ok = false })
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
    | OpLe ->
      (match s.stack with
       | b :: a :: rest ->
         { s with stack = Bool (num_cmp a b ff_le op_int_le) :: rest; pc = pc }
       | _ -> { s with ok = false })
    | OpGe ->
      (match s.stack with
       | b :: a :: rest ->
         { s with stack = Bool (num_cmp a b ff_ge op_int_ge) :: rest; pc = pc }
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
    | JumpIfTrue addr ->
      (match s.stack with
       | v :: rest ->
         if is_truthy v then { s with stack = rest; pc = addr }
         else { s with stack = rest; pc = pc }
       | _ -> { s with ok = false })
    | DictGet ->
      (match s.stack with
       | key :: map :: rest ->
         (match key with
          | Str k ->
            (let d = val_of_dict map in
             { s with stack = dict_get k d :: rest; pc = pc })
          | _ -> { s with ok = false })
       | _ -> { s with ok = false })
    | DictSet ->
      (match s.stack with
       | v2 :: key :: map :: rest ->
         (match key with
          | Str k ->
            { s with stack = Dict (dict_set k v2 (val_of_dict map)) :: rest; pc = pc }
          | _ -> { s with ok = false })
       | _ -> { s with ok = false })

    // Slot+immediate fused ops
    | SlotAddImm (idx, imm) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> { s with stack = Num (n + imm) :: s.stack; pc = pc }
       | _ -> { s with ok = false })
    | SlotSubImm (idx, imm) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> { s with stack = Num (n - imm) :: s.stack; pc = pc }
       | _ -> { s with ok = false })
    | SlotGtImm (idx, imm) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> { s with stack = Bool (n > imm) :: s.stack; pc = pc }
       | _ -> { s with ok = false })
    | SlotLtImm (idx, imm) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> { s with stack = Bool (n < imm) :: s.stack; pc = pc }
       | _ -> { s with ok = false })

    // Fused: StoreSlot + LoadSlot (same slot)
    | StoreAndLoadSlot idx ->
      (match s.stack with
       | v :: rest -> 
         (match list_set_or_extend s.slots idx v with
          | Some slots' -> { s with slots = slots'; stack = v :: rest; pc = pc }
          | None -> { s with ok = false })
       | _ -> { s with ok = false })

    // Fused: LoadSlot + Return
    | ReturnSlot idx ->
      (match list_nth s.slots idx with
       | Some v ->
         // Same as Return: pop frame or exit
         (match s.frames with
          | caller :: rest ->
            { s with
              code = s.code;
              pc = caller.ret_pc;
              slots = caller.ret_slots;
              stack = v :: caller.ret_stack;
              frames = rest;
            }
          | [] -> { s with stack = [v]; ok = true })
       | None -> { s with ok = false })

    // Closure ops
    | PushClosure chunk_idx ->
      // Build captured values from chunk's runtime_captures (reads current slots)
      (match list_nth s.code_table chunk_idx with
       | None -> { s with ok = false }
       | Some ch ->
         let caps = build_captured ch.chunk_runtime_captures s.slots in
         let inst_id = List.length s.closure_envs in
         { s with
           stack = Num inst_id :: s.stack;
           closure_envs = s.closure_envs @ [(caps, chunk_idx)];
           pc = pc;
         })

    // LoadCaptured: load from current function's captured list
    | LoadCaptured idx ->
      (match list_nth s.captured idx with
       | Some v -> { s with stack = v :: s.stack; pc = pc }
       | None -> { s with ok = false })

    | CallCaptured (argc, _nlocals) -> callcaptured_handler argc s pc

    | CallCapturedRef (idx, argc) -> callcapturedref_handler idx argc s pc

// Return: pop frame if exists, push return value to caller's stack, restore all caller state
    | GetDefaultSlot (map_slot, key_slot, default_slot, result_slot) ->
      (match list_nth s.slots map_slot, list_nth s.slots key_slot with
       | Some map_val, Some (Str k) ->
         let d = val_of_dict map_val in
         let lookup = dict_get k d in
         let result = (match lookup with
                       | Nil -> (match list_nth s.slots default_slot with
                                 | Some d2 -> d2
                                 | None -> Nil)
                       | v -> v) in
         let new_slots = (match list_set_or_extend s.slots result_slot result with
                          | Some sl -> sl
                          | None -> s.slots) in
         { s with slots = new_slots; pc = pc }
       | _, _ -> { s with ok = false })

    // Typed arithmetic
    | OpDiv ->
      (match s.stack with
       | b :: a :: rest ->
         (match a, b with
          | Num na, Num nb ->
            if nb = 0 then { s with ok = false }
            else { s with stack = Num (na / nb) :: rest; pc = pc }
          | _ -> { s with ok = false })
       | _ -> { s with ok = false })
    | OpMod ->
      (match s.stack with
       | b :: a :: rest ->
         (match a, b with
          | Num na, Num nb ->
            if nb = 0 then { s with ok = false }
            else { s with stack = Num (na % nb) :: rest; pc = pc }
          | _ -> { s with ok = false })
       | _ -> { s with ok = false })

    // MakeList: pop n items, reverse, push as list
    | MakeList n ->
      let (remaining, items) = pop_and_bind n s.stack [] in
      { s with stack = List items :: remaining; pc = pc }

    // BuiltinCall: delegate to builtin_result for Z3-friendliness
    | BuiltinCall (name, n_args) ->
      let (remaining, args) = pop_and_bind n_args s.stack [] in
      let result = builtin_result name args in
      { s with stack = result :: remaining; pc = pc }

    // LoadGlobal: would need env model — abstract as ok=false
    | LoadGlobal _ -> { s with ok = false }

    // DictMutSet: mutate dict in slot directly
    | DictMutSet slot_idx ->
      (match s.stack with
       | val2 :: key :: rest ->
         (match key with
          | Str k ->
            (match list_nth s.slots slot_idx with
             | Some (Dict m) ->
               let new_dict = dict_set k val2 m in
               let new_slots = (match list_set_or_extend s.slots slot_idx (Dict new_dict) with
                                | Some sl -> sl
                                | None -> s.slots) in
               { s with slots = new_slots; stack = Dict new_dict :: rest; pc = pc }
             | _ -> { s with ok = false })
          | _ -> { s with ok = false })
       | _ -> { s with ok = false })

    // Recur: loop construct — resets pc to 0, rebinds slots
    | SlotMulImm (idx, imm) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> { s with stack = Num (int_mul n imm) :: s.stack; pc = pc }
       | _ -> { s with ok = false })
    | SlotDivImm (idx, imm) ->
      (match list_nth s.slots idx with
       | Some (Num n) ->
         if imm = 0 then { s with ok = false }
         else { s with stack = Num (n / imm) :: s.stack; pc = pc }
       | _ -> { s with ok = false })
    | SlotEqImm (idx, imm) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> { s with stack = Bool (n = imm) :: s.stack; pc = pc }
       | _ -> { s with ok = false })
    | SlotLeImm (idx, imm) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> { s with stack = Bool (n <= imm) :: s.stack; pc = pc }
       | _ -> { s with ok = false })
    | SlotGeImm (idx, imm) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> { s with stack = Bool (n >= imm) :: s.stack; pc = pc }
       | _ -> { s with ok = false })

    // Jump-if-slot-comparison fused ops
    | JumpIfSlotLtImm (idx, imm, addr) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> if n < imm then { s with pc = addr } else { s with pc = pc }
       | _ -> { s with ok = false })
    | JumpIfSlotLeImm (idx, imm, addr) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> if n <= imm then { s with pc = addr } else { s with pc = pc }
       | _ -> { s with ok = false })
    | JumpIfSlotGtImm (idx, imm, addr) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> if n > imm then { s with pc = addr } else { s with pc = pc }
       | _ -> { s with ok = false })
    | JumpIfSlotGeImm (idx, imm, addr) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> if n >= imm then { s with pc = addr } else { s with pc = pc }
       | _ -> { s with ok = false })
    | JumpIfSlotEqImm (idx, imm, addr) ->
      (match list_nth s.slots idx with
       | Some (Num n) -> if n = imm then { s with pc = addr } else { s with pc = pc }
       | _ -> { s with ok = false })

    // Catch-all: unknown opcode

    | _ -> { s with pc = pc }

// Multi-step — step until fuel runs out or error
val closure_eval_steps : int -> closure_vm -> closure_vm
let rec closure_eval_steps fuel s =
  if fuel <= 0 then s else
  if not s.ok then s else
  closure_eval_steps (fuel - 1) (closure_eval_op s)