//! Core bytecode types: BinOp, Ty, Op, CompiledLoop
//! 
//! Extracted from bytecode/mod.rs for maintainability.
//! These types are used by the bytecode compiler and runtime.

#![allow(unreachable_patterns)]
#![allow(dead_code)]

use crate::types::LispVal;

#[derive(Clone, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

/// Known type for typed ops.
#[derive(Clone, Debug)]
pub enum Ty {
    I64,
    F64,
}

/// Bytecode opcodes for the loop VM.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Op {
    /// Push binding slot value onto stack
    LoadSlot(usize),
    /// Push a literal i64
    PushI64(i64),
    /// Push a literal f64
    PushFloat(f64),
    /// Push a literal bool
    PushBool(bool),
    /// Push a literal string
    PushStr(String),
    /// Push nil
    PushNil,
    /// Pop n values, construct a list, push it
    MakeList(usize),
    /// Duplicate top of stack
    Dup,
    /// Pop and discard top of stack
    Pop,
    /// Pop stack into binding slot
    StoreSlot(usize),
    /// Arithmetic: pop 2, push result
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    /// Comparison: pop 2, push bool
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
    /// Pop value, push its boolean negation (using is_truthy)
    Not,
    /// Pop stack, jump to addr if truthy
    JumpIfTrue(usize),
    /// Pop stack, jump to addr if falsy
    JumpIfFalse(usize),
    /// Unconditional jump
    Jump(usize),
    /// Pop TOS, return it as the loop result
    Return,
    /// Pop N args into slots 0..N, jump to loop start
    Recur(usize),
    /// Call a builtin by name with N args from stack
    BuiltinCall(String, usize),
    // --- Compound ops: fused LoadSlot(s) + PushI64(imm) + Arith/Cmp ---
    /// Read slots[s] as i64, add imm, write back to slot AND push result
    SlotAddImm(usize, i64),
    /// Read slots[s] as i64, subtract imm, write back to slot AND push result
    SlotSubImm(usize, i64),
    /// Read slots[s] as i64, multiply by imm, push result
    SlotMulImm(usize, i64),
    /// Read slots[s] as i64, divide by imm, push result
    SlotDivImm(usize, i64),
    /// Read slots[s] as i64, compare with imm for equality, push bool
    SlotEqImm(usize, i64),
    /// Read slots[s] as i64, compare with imm (<), push bool
    SlotLtImm(usize, i64),
    /// Read slots[s] as i64, compare with imm (<=), push bool
    SlotLeImm(usize, i64),
    /// Read slots[s] as i64, compare with imm (>), push bool
    SlotGtImm(usize, i64),
    /// Read slots[s] as i64, compare with imm (>=), push bool
    SlotGeImm(usize, i64),
    /// Like Recur but for small N — no Vec allocation
    RecurDirect(usize),
    // --- Super-fused ops: eliminate stack traffic entirely ---
    /// Compare slots[s] with imm, jump to addr if condition is true (no stack push/pop)
    JumpIfSlotLtImm(usize, i64, usize),
    JumpIfSlotLeImm(usize, i64, usize),
    JumpIfSlotGtImm(usize, i64, usize),
    JumpIfSlotGeImm(usize, i64, usize),
    JumpIfSlotEqImm(usize, i64, usize),
    // --- Mega-fused: entire loop body in one op ---
    /// RecurIncAccum(counter_slot, accum_slot, step_imm, limit_imm, exit_addr):
    /// if slots[counter] >= limit_imm → jump to exit_addr
    /// else: accum += counter; counter += step_imm; jump to loop_start (pc=0)
    /// Covers: (loop ((i 0) (sum 0)) (if (>= i N) sum (recur (+ i 1) (+ sum i))))
    RecurIncAccum(usize, usize, i64, i64, usize),
    /// Call a captured function from slot with N args from stack
    CallCaptured(usize, usize),
    /// Push captured var value from cl.captured[idx] (no slot copy)
    LoadCaptured(usize),
    /// StoreCaptured(idx): pop value, write to cl.captured[idx], push value back.
    /// Used for set! on variables captured from enclosing let-bindings.
    StoreCaptured(usize),
    /// Look up a global variable by name from the live outer env (not frozen)
    LoadGlobal(String),
    /// StoreGlobal(name): pop value, write to outer_env[name], push value back.
    /// Used for set! on top-level/captured variables.
    StoreGlobal(String),
    /// Call captured function from cl.captured[idx] with N args (no slot copy)
    CallCapturedRef(usize, usize),
    /// PushSelf: push the current function value onto the stack (for Y combinator self-passing)
    PushSelf,
    /// Push a pre-compiled closure from cl.closures[idx] onto the stack
    PushClosure(usize),
    /// Push a function name onto the call trace (for stack traces on errors)
    TracePush(String),
    /// Pop a function name from the call trace
    TracePop,
    /// DictGet: pop key, pop map, push map[key] (or Nil)
    DictGet,
    /// DictSet: pop val, pop key, pop map, push map with key=val
    DictSet,
    /// DictMutSet(slot): pop val, pop key, mutate dict in slot in-place (no clone)
    DictMutSet(usize),
    /// CallSelf: call this compiled lambda recursively with N args from stack
    CallSelf(usize),
    /// CallDynamic: call a function value from the stack top with N args below it
    /// Pops: [func, arg1, arg2, ..., argN] → pushes result
    CallDynamic(usize),
    /// GetDefaultSlot(map_slot, key_slot, default_slot, result_slot):
    /// Fused get-default pattern — reads slots directly, no stack traffic.
    /// result = if dict/get(slots[map_slot], slots[key_slot]) is nil
    ///          then slots[default_slot] else dict/get result
    GetDefaultSlot(usize, usize, usize, usize),
    /// StoreAndLoadSlot: pop from stack into slot, then push slot value back.
    /// Fuses StoreSlot(N) + LoadSlot(N) — the slot gets updated and the value
    /// stays on the stack, avoiding a separate load dispatch.
    StoreAndLoadSlot(usize),
    /// ReturnSlot: return the value in slot N directly, no stack push/pop.
    /// Fuses LoadSlot(N) + Return.
    ReturnSlot(usize),
    // --- Typed ops: assume operand types, zero dynamic dispatch ---
    /// Pop 2, perform typed binary op, push result.
    TypedBinOp(BinOp, Ty),
    /// Push a first-class builtin function reference onto the stack.
    /// Used when a builtin name is referenced as a value (not in call position).
    PushBuiltin(String),
    /// PushLiteral(val): push an arbitrary LispVal onto the stack.
    /// Used for quote and other compile-time constant expressions.
    PushLiteral(LispVal),
    // --- Sum-type primitives (deftype) ---
    /// ConstructTag(type_name, variant_id, n_fields): pop n_fields values from stack,
    /// construct a Tagged { type_name, variant_id, fields }, push it.
    ConstructTag(String, u16, u8),
    /// TagTest(type_name, variant_id): peek at stack top, push Bool(true) if it's
    /// a Tagged value matching both type_name and variant_id, else Bool(false).
    /// Does NOT pop the value — use Dup + TagTest + Pop or just TagTest + consume.
    TagTest(String, u16),
    /// GetField(idx): pop a Tagged value, push its fields[idx].
    /// Panics if TOS is not Tagged or idx is out of bounds.
    GetField(u8),
    // --- Fused HOF opcodes: map/filter/reduce with known function in slot ---
    /// MapOp(slot_idx): pop list, apply slots[slot_idx] to each element, push result list.
    /// Function takes exactly 1 arg (the element).
    MapOp(usize),
    /// FilterOp(slot_idx): pop list, keep elements where slots[slot_idx] returns truthy, push filtered list.
    /// Function takes exactly 1 arg (the element).
    FilterOp(usize),
    /// ReduceOp(slot_idx): pop list, pop init, fold with slots[slot_idx](acc, elem), push result.
    /// Function takes exactly 2 args (accumulator, element).
    ReduceOp(usize),
    // --- Vec operations: native immutable arrays ---
    /// Pop n values, construct a Vec, push it
    MakeVec(usize),
    /// Pop index, pop vec, push vec[index] (or Nil if out of bounds)
    VecNth,
    /// Pop value, pop index, pop vec, push vec with value at index (fresh copy)
    VecAssoc,
    /// Pop vec, push its length as Num
    VecLen,
    /// Pop value, pop vec, push vec with value appended (fresh copy)
    VecConj,
    /// Pop vec, pop value, push true if value is in vec (structural equality)
    VecContains,
    /// Pop end, pop start, pop vec, push vec[start:end] (fresh copy)
    VecSlice,
}

/// Compiled loop representation.
#[allow(dead_code)]
pub struct CompiledLoop {
    /// Number of binding slots
    pub num_slots: usize,
    /// Binding names (for fallback)
    pub slot_names: Vec<String>,
    /// Initial values for slots
    pub init_vals: Vec<LispVal>,
    /// Bytecode
    pub code: Vec<Op>,
    /// PC of the loop start (for recur jumps)
    pub loop_start_pc: usize,
    /// Captured outer env variables (name → value), placed in slots after bindings
    pub captured: Vec<(String, LispVal)>,
}
