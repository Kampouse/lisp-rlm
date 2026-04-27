(** Lisp Runtime Types — F* Formal Specification

    Mirrors src/types.rs LispVal enum and src/bytecode.rs Op/BinOp/Ty enums.
    Following vWasm's pattern: semantics/wasm/Wasm.Types.fst + Wasm.Values.fst
*)
module Lisp.Types

// === LispVal ===
// The universal value type. Every runtime value is one of these.

type lisp_val =
  | Num    of int           (* i64 integer   — Rust: LispVal::Num(i64)     *)
  | Float  of float         (* f64 float     — Rust: LispVal::Float(f64)   *)
  | Bool   of bool          (* boolean       — Rust: LispVal::Bool(bool)   *)
  | Nil                     (* unit/void     — Rust: LispVal::Nil          *)
  | Str    of string        (* string        — Rust: LispVal::Str(String)  *)
  | Pair   of lisp_val * lisp_val  (* cons cell — Rust: LispVal::List(Vec) *)
  | Dict   of list (string * lisp_val) (* hashmap  — Rust: LispVal::Dict(HashMap) *)

// === BinOp ===
// Binary operations (both polymorphic and typed).
// Mirrors src/bytecode.rs BinOp enum.

type binop =
  | Add
  | Sub
  | Mul
  | Div
  | Mod
  | Eq
  | Lt
  | Le
  | Gt
  | Ge

// === Ty ===
// Known type annotation for typed ops.
// Mirrors src/bytecode.rs Ty enum.

type ty =
  | I64
  | F64

// === Op (bytecode opcodes) ===
// The full instruction set.
// Mirrors src/bytecode.rs Op enum.

type opcode =
  // Stack manipulation
  | LoadSlot       of nat                           (* Push slot[n]            *)
  | PushI64        of int                           (* Push integer literal    *)
  | PushFloat      of float                         (* Push float literal      *)
  | PushBool       of bool                          (* Push boolean literal    *)
  | PushStr        of string                        (* Push string literal     *)
  | PushNil                                         (* Push nil                *)
  | MakeList       of nat                           (* Pop n, construct list   *)
  | Dup                                             (* Duplicate TOS           *)
  | Pop                                             (* Discard TOS             *)
  | StoreSlot      of nat                           (* Pop into slot[n]        *)

  // Arithmetic (polymorphic)
  | Add
  | Sub
  | Mul
  | Div
  | Mod

  // Comparison (polymorphic)
  | Eq
  | Lt
  | Le
  | Gt
  | Ge

  // Control flow
  | JumpIfTrue     of nat                           (* Pop, jump if truthy     *)
  | JumpIfFalse    of nat                           (* Pop, jump if falsy      *)
  | Jump           of nat                           (* Unconditional jump      *)
  | Return                                          (* Pop, return as result   *)
  | Recur          of nat                           (* Pop N args, restart     *)

  // Builtin dispatch
  | BuiltinCall    of string * nat                  (* Call builtin by name    *)

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
  | DictGet                                         (* Pop key, pop map, push val *)
  | DictSet                                         (* Pop val, key, map → new map *)
  | DictMutSet     of nat                           (* In-place dict update in slot *)

  // Self-call
  | CallSelf       of nat

  // Fused patterns
  | GetDefaultSlot   of nat * nat * nat * nat       (* Fused dict/get with default *)
  | StoreAndLoadSlot of nat                         (* StoreSlot + LoadSlot fused  *)
  | ReturnSlot       of nat                         (* Return slot directly        *)

  // Typed ops (zero dynamic dispatch)
  | TypedBinOp     of binop * ty

// === VM State ===
// The state the VM operates on. Matches the Rust runtime.

type vm_state = {
  stack : list lisp_val;        (* value stack                     *)
  slots : list lisp_val;        (* binding slots (loop variables)  *)
  pc    : nat;                  (* program counter                 *)
  code  : list opcode;          (* instruction sequence            *)
  ok    : bool;                 (* execution status flag           *)
}
