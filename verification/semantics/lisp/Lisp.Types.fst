(** Lisp Runtime Types -- F* Formal Specification

    Mirrors src/types.rs LispVal enum and src/bytecode.rs Op/BinOp/Ty enums.
    Following vWasm pattern: semantics/wasm/Wasm.Types.fst + Wasm.Values.fst
*)
module Lisp.Types

open FStar.Seq

// === Abstract float type ===

type ffloat
assume val ff_eq  : ffloat -> ffloat -> Tot bool

assume val ff_of_int : int -> Tot ffloat
assume val ff_to_int : ffloat -> Tot int
assume val ff_add : ffloat -> ffloat -> Tot ffloat
assume val ff_sub : ffloat -> ffloat -> Tot ffloat
assume val ff_mul : ffloat -> ffloat -> Tot ffloat
assume val ff_div : ffloat -> ffloat -> Tot ffloat
assume val ff_rem : ffloat -> ffloat -> Tot ffloat
assume val ff_gt  : ffloat -> ffloat -> Tot bool
assume val ff_lt  : ffloat -> ffloat -> Tot bool
assume val ff_ge  : ffloat -> ffloat -> Tot bool
assume val ff_le  : ffloat -> ffloat -> Tot bool

// === LispVal ===
noeq type lisp_val =
  | Num    of int
  | Float  of ffloat
  | Bool   of bool
  | Nil
  | Str    of string
  | Sym    of string
  | List   of list lisp_val
  | Pair   of lisp_val * lisp_val
  | Dict   of list (string * lisp_val)
  | Lambda of list string * lisp_val * list (string * lisp_val)  (* params, body, env *)
  | BuiltinFn of string
  | Tagged  of string * int * list (string * lisp_val)  (* type_name, variant_id, [(field_name, field_val)] *)
  | Vec     of seq lisp_val

// === BinOp ===
type binop =
  | Add | Sub | Mul | Div | Mod
  | Eq | Lt | Le | Gt | Ge

// === Ty (type annotation for typed ops) ===
type ty =
  | I64
  | F64

// === Op (bytecode opcodes) ===
noeq type opcode =
  // Stack manipulation
  | LoadSlot       of int
  | PushI64        of int
  | PushFloat      of ffloat
  | PushBool       of bool
  | PushStr        of string
  | PushNil
  | MakeList       of nat
  | Dup
  | Pop
  | StoreSlot      of int

  // Arithmetic (polymorphic)
  | OpAdd | OpSub | OpMul | OpDiv | OpMod

  // Comparison (polymorphic)
  | OpEq | OpLt | OpLe | OpGt | OpGe

  // Control flow
  | JumpIfTrue     of nat
  | JumpIfFalse    of nat
  | Jump           of nat
  | Return
  | Recur          of nat

  // Builtin dispatch
  | BuiltinCall    of string * nat

  // Fused slot+immediate ops
  | SlotAddImm     of nat * int
  | SlotSubImm     of nat * int
  | SlotMulImm     of nat * int
  | SlotDivImm     of nat * int
  | SlotEqImm      of nat * int
  | SlotLtImm      of nat * int
  | SlotLeImm      of nat * int
  | SlotGtImm      of nat * int
  | SlotGeImm      of nat * int

  // Super-fused: slot compare + conditional jump
  | JumpIfSlotLtImm  of nat * int * nat
  | JumpIfSlotLeImm  of nat * int * nat
  | JumpIfSlotGtImm  of nat * int * nat
  | JumpIfSlotGeImm  of nat * int * nat
  | JumpIfSlotEqImm  of nat * int * nat

  // Mega-fused: entire loop body
  | RecurIncAccum  of nat * nat * int * int * nat

  // Closure operations
  | CallCaptured   of nat * nat
  | LoadCaptured   of nat
  | LoadGlobal     of string
  | CallCapturedRef of nat * nat
  | PushClosure    of nat

  // Dict operations
  | DictGet
  | DictSet
  | DictMutSet     of nat

  // Self-call
  | CallSelf       of nat

  // Dynamic builtin
  | PushBuiltin    of string
  | CallDynamic    of nat
  | RecurDirect    of nat

  // Literal value (for quote)
  | PushLiteral    of lisp_val

  // Global environment operations (env is a functional dict)
  | StoreGlobal    of string

  // Captured variable mutation
  | StoreCaptured  of nat

  // Sum type (deftype) operations
  | ConstructTag   of string * nat * nat  (* type_name, n_args, variant_idx *)
  | TagTest        of string * nat         (* type_name, variant_idx *)
  | GetField       of nat                  (* field index *)

  // Fused patterns
  | GetDefaultSlot   of nat * nat * nat * nat
  | StoreAndLoadSlot of nat
  | ReturnSlot       of nat

  // Fused HOF opcodes
  | MapOp          of nat   (* slot_idx — function takes exactly 1 arg *)
  | FilterOp       of nat   (* slot_idx — function takes exactly 1 arg *)
  | ReduceOp       of nat   (* slot_idx — function takes exactly 2 args: acc, elem *)

  // Typed ops
  | TypedBinOp     of binop * ty

  // Vec operations
  | MakeVec        of nat   (* n_args: pop n, reverse, push Vec *)
  | VecNth              (* pop idx, pop vec, push vec[idx] or Nil *)
  | VecAssoc            (* pop val, pop idx, pop vec, push updated vec *)
  | VecLen              (* pop vec, push length as Num *)
  | VecConj             (* pop val, pop vec, push vec+[val] *)
  | VecContains         (* pop val, pop vec, push Bool *)
  | VecSlice            (* pop end, pop start, pop vec, push sub-vec *)

// === VM State ===
noeq type vm_state = {
  stack : list lisp_val;
  slots : list lisp_val;
  pc    : nat;
  code  : list opcode;
  ok    : bool;
}
