(** Lisp Runtime Types — F* Formal Specification

    Mirrors src/types.rs LispVal enum and src/bytecode.rs Op/BinOp/Ty enums.
    Following vWasm's pattern: semantics/wasm/Wasm.Types.fst + Wasm.Values.fst
*)
module Lisp.Types

// === Abstract float type ===
// F* doesn't have native IEEE754 floats. Following vWasm's approach
// (semantics/wasm/Wasm.F32.fst): use an abstract type with assumed operations.

type ffloat                                    (* abstract float type *)

assume val ff_of_int : int -> Tot ffloat
assume val ff_to_int : ffloat -> Tot int
assume val ff_add : ffloat -> ffloat -> Tot ffloat
assume val ff_sub : ffloat -> ffloat -> Tot ffloat
assume val ff_mul : ffloat -> ffloat -> Tot ffloat
assume val ff_div : ffloat -> ffloat -> Tot ffloat
assume val ff_gt  : ffloat -> ffloat -> Tot bool
assume val ff_lt  : ffloat -> ffloat -> Tot bool
assume val ff_ge  : ffloat -> ffloat -> Tot bool
assume val ff_le  : ffloat -> ffloat -> Tot bool
assume val ff_eq  : ffloat -> ffloat -> Tot bool

// === LispVal ===
noeq type lisp_val =
  | Num    of int
  | Float  of ffloat
  | Bool   of bool
  | Nil
  | Str    of string
  | Pair   of lisp_val * lisp_val
  | Dict   of list (string * lisp_val)

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
  | LoadSlot       of nat
  | PushI64        of int
  | PushFloat      of ffloat
  | PushBool       of bool
  | PushStr        of string
  | PushNil
  | MakeList       of nat
  | Dup
  | Pop
  | StoreSlot      of nat

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

  // Fused patterns
  | GetDefaultSlot   of nat * nat * nat * nat
  | StoreAndLoadSlot of nat
  | ReturnSlot       of nat

  // Typed ops
  | TypedBinOp     of binop * ty

// === VM State ===
noeq type vm_state = {
  stack : list lisp_val;
  slots : list lisp_val;
  pc    : nat;
  code  : list opcode;
  ok    : bool;
}
