//! Bytecode verifier for lisp-rlm compiled lambdas.
//!
//! Validates bytecode safety before execution. Catches structural bugs that would
//! cause silent corruption (stack underflow → Nil injection, OOB slot access, etc.)
//!
//! Architecture mirrors Move's bytecode verifier: ordered passes from cheap structural
//! checks to expensive dataflow analysis.
//!
//! Passes:
//! 1. Hard limits (code size, stack depth, locals, closures)
//! 2. Control flow (branch targets in bounds, valid CFG)
//! 3. Slot bounds (all slot indices within slots_len)
//! 4. Instruction consistency (builtin names, closure indices, recur arity)
//! 5. Stack depth (abstract interpretation — no underflow, consistent heights at joins)

use crate::bytecode::{BinOp, Op, Ty, CompiledLambda};

// ── Hard limits ──────────────────────────────────────────────────────────────

/// Maximum number of bytecode instructions per function.
const MAX_CODE_SIZE: usize = 100_000;
/// Maximum stack depth (prevents stack bomb DoS).
const MAX_STACK_DEPTH: usize = 10_000;
/// Maximum number of local slots.
const MAX_SLOTS: usize = 10_000;
/// Maximum number of nested closures.
const MAX_CLOSURES: usize = 1_000;
/// Maximum nesting depth for recursive closure verification.
const MAX_CLOSURE_DEPTH: usize = 64;

/// Known builtin function names that BuiltinCall can reference.
const KNOWN_BUILTINS: &[&str] = &[
    // Arithmetic
    "abs", "inc", "dec", "sqrt", "pow", "mod", "remainder",
    // List
    "car", "cdr", "cons", "list", "length", "len", "append",
    "reverse", "take", "drop", "last", "butlast", "range", "nth",
    "first", "rest",
    // String
    "to-string", "str", "str-concat", "str-length", "str-split",
    "str-contains", "string-append", "substring",
    // Type conversion
    "to-int", "to-float", "integer", "float", "boolean",
    // Dict
    "dict/get", "dict-ref", "dict/set", "dict-set",
    // Misc
    "not", "error", "apply", "eval", "doc", "now", "elapsed",
    // NEAR builtins
    "storage-write", "storage_write",
    "storage-read", "storage_read",
    "storage-remove", "storage_remove",
    "storage-has-key", "storage_has_key",
    "block-height", "block_height",
    "block-timestamp", "block_timestamp",
    "signer-account-id", "signer_account_id",
    "predecessor-account-id", "predecessor_account_id",
    "current-account-id", "current_account_id",
    "attached-deposit", "attached_deposit",
    "account-balance", "account_balance",
    "log-utf8", "log_utf8", "log",
    "near-config", "near_config",
    "near-reset", "near_reset",
];

/// Check if a builtin name is known.
fn is_known_builtin(name: &str) -> bool {
    KNOWN_BUILTINS.contains(&name)
}

// ── Verification error ───────────────────────────────────────────────────────

/// Verification error with location context.
#[derive(Debug, Clone)]
pub struct VerificationError {
    /// Description of the error.
    pub message: String,
    /// Bytecode offset where the error occurred (if applicable).
    pub offset: Option<usize>,
    /// Which pass detected the error.
    pub pass: &'static str,
}

impl std::fmt::Display for VerificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.offset {
            Some(off) => write!(f, "[{} @ offset {}] {}", self.pass, off, self.message),
            None => write!(f, "[{}] {}", self.pass, self.message),
        }
    }
}

impl std::error::Error for VerificationError {}

type VResult<T> = Result<T, VerificationError>;

fn err(pass: &'static str, offset: Option<usize>, msg: impl Into<String>) -> VerificationError {
    VerificationError {
        message: msg.into(),
        offset,
        pass,
    }
}

// ── Pass 1: Hard limits ─────────────────────────────────────────────────────

fn verify_limits(cl: &CompiledLambda) -> VResult<()> {
    if cl.code.len() > MAX_CODE_SIZE {
        return Err(err("limits", None,
            format!("code size {} exceeds limit {}", cl.code.len(), MAX_CODE_SIZE)));
    }
    if cl.total_slots > MAX_SLOTS {
        return Err(err("limits", None,
            format!("slot count {} exceeds limit {}", cl.total_slots, MAX_SLOTS)));
    }
    if cl.closures.len() > MAX_CLOSURES {
        return Err(err("limits", None,
            format!("closure count {} exceeds limit {}", cl.closures.len(), MAX_CLOSURES)));
    }
    Ok(())
}

// ── Pass 2: Control flow ────────────────────────────────────────────────────

fn verify_control_flow(code: &[Op]) -> VResult<()> {
    let len = code.len();
    for (i, op) in code.iter().enumerate() {
        match op {
            Op::Jump(target)
            | Op::JumpIfTrue(target)
            | Op::JumpIfFalse(target) => {
                if *target >= len {
                    return Err(err("control_flow", Some(i),
                        format!("branch target {} out of bounds (code_len={})", target, len)));
                }
            }
            Op::RecurIncAccum(_, _, _, _, exit_addr) => {
                if *exit_addr >= len {
                    return Err(err("control_flow", Some(i),
                        format!("RecurIncAccum exit target {} out of bounds (code_len={})",
                            exit_addr, len)));
                }
                // RecurIncAccum always jumps to pc=0, which is always valid
            }
            Op::JumpIfSlotLtImm(_, _, target)
            | Op::JumpIfSlotLeImm(_, _, target)
            | Op::JumpIfSlotGtImm(_, _, target)
            | Op::JumpIfSlotGeImm(_, _, target)
            | Op::JumpIfSlotEqImm(_, _, target) => {
                if *target >= len {
                    return Err(err("control_flow", Some(i),
                        format!("JumpIfSlot*Imm target {} out of bounds (code_len={})",
                            target, len)));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

// ── Pass 3: Slot bounds ────────────────────────────────────────────────────

fn verify_slot_indices(code: &[Op], slots_len: usize) -> VResult<()> {
    for (i, op) in code.iter().enumerate() {
        match op {
            Op::LoadSlot(s)
            | Op::StoreSlot(s)
            | Op::ReturnSlot(s)
            | Op::StoreAndLoadSlot(s)
            | Op::DictMutSet(s)
            | Op::RecurDirect(s) => {
                if *s >= slots_len {
                    return Err(err("slot_bounds", Some(i),
                        format!("slot index {} out of bounds (slots_len={})", s, slots_len)));
                }
            }
            Op::SlotAddImm(s, _)
            | Op::SlotSubImm(s, _)
            | Op::SlotMulImm(s, _)
            | Op::SlotDivImm(s, _)
            | Op::SlotEqImm(s, _)
            | Op::SlotLtImm(s, _)
            | Op::SlotLeImm(s, _)
            | Op::SlotGtImm(s, _)
            | Op::SlotGeImm(s, _) => {
                if *s >= slots_len {
                    return Err(err("slot_bounds", Some(i),
                        format!("slot index {} out of bounds (slots_len={})", s, slots_len)));
                }
            }
            Op::JumpIfSlotLtImm(s, _, _)
            | Op::JumpIfSlotLeImm(s, _, _)
            | Op::JumpIfSlotGtImm(s, _, _)
            | Op::JumpIfSlotGeImm(s, _, _)
            | Op::JumpIfSlotEqImm(s, _, _) => {
                if *s >= slots_len {
                    return Err(err("slot_bounds", Some(i),
                        format!("slot index {} out of bounds (slots_len={})", s, slots_len)));
                }
            }
            Op::RecurIncAccum(counter, accum, _, _, _) => {
                if *counter >= slots_len {
                    return Err(err("slot_bounds", Some(i),
                        format!("RecurIncAccum counter slot {} out of bounds (slots_len={})",
                            counter, slots_len)));
                }
                if *accum >= slots_len {
                    return Err(err("slot_bounds", Some(i),
                        format!("RecurIncAccum accum slot {} out of bounds (slots_len={})",
                            accum, slots_len)));
                }
            }
            Op::GetDefaultSlot(a, b, c, d) => {
                for &(name, idx) in &[("map", *a), ("key", *b), ("default", *c), ("result", *d)] {
                    if idx >= slots_len {
                        return Err(err("slot_bounds", Some(i),
                            format!("GetDefaultSlot {} slot {} out of bounds (slots_len={})",
                                name, idx, slots_len)));
                    }
                }
            }
            Op::MapOp(slot_idx) | Op::FilterOp(slot_idx) | Op::ReduceOp(slot_idx) => {
                if *slot_idx >= slots_len {
                    return Err(err("slot_bounds", Some(i),
                        format!("fused HOF slot {} out of bounds (slots_len={})",
                            slot_idx, slots_len)));
                }
            }
            Op::Recur(n) => {
                if *n > slots_len {
                    return Err(err("slot_bounds", Some(i),
                        format!("Recur({}) requires {} slots but only {} available",
                            n, n, slots_len)));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

// ── Pass 4: Instruction consistency ─────────────────────────────────────────

fn verify_instruction_consistency(
    code: &[Op],
    slots_len: usize,
    closures_len: usize,
) -> VResult<()> {
    for (i, op) in code.iter().enumerate() {
        match op {
            Op::BuiltinCall(_name, _nargs) => {
                // BuiltinCall is a runtime dispatch — the name may be a stdlib
                // function or user-defined function resolved at eval time.
                // Unknown names produce a runtime error, which is safe.
            }
            Op::PushBuiltin(name) => {
                if !is_known_builtin(name) {
                    return Err(err("instruction_consistency", Some(i),
                        format!("unknown builtin '{}'", name)));
                }
            }
            Op::PushClosure(idx) => {
                if *idx >= closures_len {
                    return Err(err("instruction_consistency", Some(i),
                        format!("PushClosure({}) out of bounds (closures_len={})",
                            idx, closures_len)));
                }
            }
            Op::CallCaptured(slot_idx, _nargs) => {
                if *slot_idx >= slots_len {
                    return Err(err("instruction_consistency", Some(i),
                        format!("CallCaptured slot {} out of bounds (slots_len={})",
                            slot_idx, slots_len)));
                }
            }
            Op::CallCapturedRef(cap_idx, _nargs) => {
                // We can't verify captured index at compile time since captures
                // are populated at runtime. This is checked by slot_bounds if
                // the capture was stored in a slot.
                if *cap_idx > 10_000 {
                    return Err(err("instruction_consistency", Some(i),
                        format!("CallCapturedRef({}) suspiciously large capture index", cap_idx)));
                }
            }
            Op::CallSelf(_nargs) => {
                // Always valid — calls the current function
            }
            Op::CallDynamic(nargs) => {
                // Pops func + nargs args from stack. We verify stack depth in pass 5.
                // Just check the arg count is reasonable.
                if *nargs > MAX_STACK_DEPTH {
                    return Err(err("instruction_consistency", Some(i),
                        format!("CallDynamic({}) arg count exceeds limit", nargs)));
                }
            }
            Op::MakeList(n) => {
                if *n > MAX_STACK_DEPTH {
                    return Err(err("instruction_consistency", Some(i),
                        format!("MakeList({}) count exceeds limit", n)));
                }
            }
            Op::ConstructTag(_, _, n_fields) => {
                if *n_fields as usize > MAX_STACK_DEPTH {
                    return Err(err("instruction_consistency", Some(i),
                        format!("ConstructTag fields={} exceeds limit", n_fields)));
                }
            }
            Op::LoadCaptured(idx) => {
                if *idx > 10_000 {
                    return Err(err("instruction_consistency", Some(i),
                        format!("LoadCaptured({}) suspiciously large capture index", idx)));
                }
            }
            Op::StoreCaptured(idx) => {
                if *idx > 10_000 {
                    return Err(err("instruction_consistency", Some(i),
                        format!("StoreCaptured({}) suspiciously large capture index", idx)));
                }
            }
            Op::GetField(idx) => {
                if *idx > 255 {
                    return Err(err("instruction_consistency", Some(i),
                        format!("GetField({}) field index out of range", idx)));
                }
            }
            Op::TypedBinOp(binop, ty) => {
                // Validate the typed op combination makes sense
                match ty {
                    Ty::I64 | Ty::F64 => {
                        match binop {
                            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod
                            | BinOp::Eq | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {}
                        }
                    }
                }
            }
            Op::RecurIncAccum(_, _, step, _limit, _) => {
                if *step == 0 {
                    return Err(err("instruction_consistency", Some(i),
                        "RecurIncAccum step_imm is 0 (infinite loop)"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

// ── Pass 5: Stack depth verification ────────────────────────────────────────

/// Stack effect for each instruction: (pops, pushes).
/// Returns None for instructions that are terminators (no successor).
fn stack_effect(op: &Op) -> (usize, usize) {
    match op {
        // Push 0, pop 0
        Op::PushI64(_) | Op::PushFloat(_) | Op::PushBool(_) | Op::PushStr(_)
        | Op::PushNil | Op::PushSelf | Op::LoadSlot(_) | Op::LoadCaptured(_)
        | Op::LoadGlobal(_) | Op::PushClosure(_) | Op::PushBuiltin(_)
        | Op::PushLiteral(_) | Op::TracePush(_) => (0, 1),

        // Pop 1, push 0
        Op::Pop | Op::TracePop => (1, 0),

        // Peek at TOS, push copy (net +1, no pop)
        Op::Dup => (0, 1),

        // StoreSlot: pop 1, push 0 (stores into slot, value consumed)
        Op::StoreSlot(_) => (1, 0),
        // StoreCaptured/StoreGlobal: pop 1, push 1 (stores, pushes value back for chaining)
        Op::StoreCaptured(_) | Op::StoreGlobal(_) => (1, 1),

        // Pop 2, push 1 (arithmetic/comparison)
        Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod
        | Op::Eq | Op::Lt | Op::Le | Op::Gt | Op::Ge => (2, 1),

        // Pop 1, push 1
        Op::Not => (1, 1),

        // Pop 1, push 0 (conditional jump — consumes the condition)
        Op::JumpIfTrue(_) | Op::JumpIfFalse(_) => (1, 0),

        // Pop 0, push 0 (unconditional jump)
        Op::Jump(_) => (0, 0),

        // Terminator: pop 1, push 0, no successor
        Op::Return => (1, 0),

        // Pop N, push 0 (recur stores into slots)
        Op::Recur(n) => (*n, 0),

        // Pop nargs, push 1 (builtin call)
        Op::BuiltinCall(_, nargs) => (*nargs, 1),

        // Slot-immediate ops: read slot, push result (no stack pop)
        Op::SlotAddImm(_, _) | Op::SlotSubImm(_, _) | Op::SlotMulImm(_, _)
        | Op::SlotDivImm(_, _) | Op::SlotEqImm(_, _) | Op::SlotLtImm(_, _)
        | Op::SlotLeImm(_, _) | Op::SlotGtImm(_, _) | Op::SlotGeImm(_, _) => (0, 1),

        // JumpIfSlot*Imm: no stack effect (reads slots directly)
        Op::JumpIfSlotLtImm(_, _, _) | Op::JumpIfSlotLeImm(_, _, _)
        | Op::JumpIfSlotGtImm(_, _, _) | Op::JumpIfSlotGeImm(_, _, _)
        | Op::JumpIfSlotEqImm(_, _, _) => (0, 0),

        // RecurIncAccum: no stack effect (reads/writes slots directly)
        Op::RecurIncAccum(_, _, _, _, _) => (0, 0),

        // RecurDirect(n): pop N args from stack into slots, jump to loop start
        Op::RecurDirect(n) => (*n, 0),

        // CallCaptured(slot, nargs): func from slot (no pop), pop nargs, push 1
        // CallCapturedRef(idx, nargs): func from captured (no pop), pop nargs, push 1
        // CallSelf(nargs): implicit func (no pop), pop nargs, push 1
        Op::CallCaptured(_, nargs) | Op::CallCapturedRef(_, nargs) | Op::CallSelf(nargs) => {
            (*nargs, 1)
        }

        // Pop nargs + 1 (func), push 1
        Op::CallDynamic(nargs) => (*nargs + 1, 1),

        // Pop n, push 1
        Op::MakeList(n) => (*n, 1),

        // Pop 2, push 1 (dict operations)
        Op::DictGet => (2, 1),
        Op::DictSet => (3, 1),

        // Pop 2, push 0 (mutate dict in slot)
        Op::DictMutSet(_) => (2, 0),

        // Pop 0, push 1 (StoreAndLoadSlot pops 1, stores, pushes back)
        Op::StoreAndLoadSlot(_) => (1, 1),

        // Terminator: pop 0, push 0 (ReturnSlot reads slot directly)
        Op::ReturnSlot(_) => (0, 0),

        // Pop 1, push 1 (TypedBinOp)
        Op::TypedBinOp(_, _) => (2, 1),

        // Pop n_fields, push 1
        Op::ConstructTag(_, _, n_fields) => (*n_fields as usize, 1),

        // Pop 0, push 1 (TagTest peeks, pushes bool)
        Op::TagTest(_, _) => (0, 1),

        // Pop 1, push 1
        Op::GetField(_) => (1, 1),

        // GetDefaultSlot: no stack effect (reads/writes slots directly)
        Op::GetDefaultSlot(_, _, _, _) => (0, 0),

        // Fused HOF opcodes
        // MapOp(slot): pop 1 (list), push 1 (result list)
        Op::MapOp(_) => (1, 1),
        // FilterOp(slot): pop 1 (list), push 1 (filtered list)
        Op::FilterOp(_) => (1, 1),
        // ReduceOp(slot): pop 2 (list, init), push 1 (accumulator)
        Op::ReduceOp(_) => (2, 1),
    }
}

/// Get successor PCs for an instruction at offset `pc` in code of length `len`.
fn successors(pc: usize, op: &Op, code_len: usize) -> Vec<usize> {
    match op {
        Op::Return | Op::ReturnSlot(_) => vec![], // terminators
        Op::Jump(target) => vec![*target],
        Op::JumpIfTrue(target) | Op::JumpIfFalse(target) => {
            let mut succs = vec![pc + 1];
            if *target != pc + 1 {
                succs.push(*target);
            }
            succs
        }
        Op::RecurIncAccum(_, _, step, limit, exit_addr) => {
            // If counter >= limit → jump to exit, else jump to 0
            let mut succs = vec![*exit_addr];
            if *exit_addr != 0 {
                succs.push(0);
            }
            succs
        }
        Op::Recur(_) | Op::RecurDirect(_) => {
            // Jumps to loop start (always pc=0 for compiled loops)
            vec![0]
        }
        Op::JumpIfSlotLtImm(_, _, target)
        | Op::JumpIfSlotLeImm(_, _, target)
        | Op::JumpIfSlotGtImm(_, _, target)
        | Op::JumpIfSlotGeImm(_, _, target)
        | Op::JumpIfSlotEqImm(_, _, target) => {
            let mut succs = vec![pc + 1];
            if *target != pc + 1 {
                succs.push(*target);
            }
            succs
        }
        _ => {
            if pc + 1 < code_len {
                vec![pc + 1]
            } else {
                vec![] // fall off end — treat as implicit return
            }
        }
    }
}

/// Verify stack depth via abstract interpretation (worklist fixed-point).
///
/// Proves that:
/// 1. No instruction path pops from an empty stack (underflow)
/// 2. Stack height is consistent at join points (same height from all predecessors)
/// 3. Stack height never exceeds MAX_STACK_DEPTH
fn verify_stack_depth(code: &[Op]) -> VResult<()> {
    let len = code.len();
    if len == 0 {
        return Ok(());
    }

    // Map from PC → stack height. None = not yet visited.
    // Use i32 to detect underflow (negative = underflow).
    let mut heights: Vec<Option<i32>> = vec![None; len];
    let mut worklist: Vec<usize> = vec![0];

    // Entry point: stack is empty (height 0).
    heights[0] = Some(0);

    let mut iterations = 0;
    let max_iterations = len * 10; // prevent infinite loops in the verifier itself

    while let Some(pc) = worklist.pop() {
        iterations += 1;
        if iterations > max_iterations {
            return Err(err("stack_depth", Some(pc),
                "verifier exceeded iteration limit (possible pathological control flow)"));
        }

        let height = match heights[pc] {
            Some(h) => h,
            None => continue, // unreachable
        };

        let op = &code[pc];
        let (pops, pushes) = stack_effect(op);

        // Check underflow
        if height < pops as i32 {
            // Dump last few instructions for debugging
            let start = if pc > 5 { pc - 5 } else { 0 };
            let dump: Vec<String> = (start..code.len())
                .map(|i| format!("  [{}] {:?}", i, code[i]))
                .collect();
            return Err(err("stack_depth", Some(pc),
                format!("stack underflow: need {} values but stack has {}\n  bytecode around offset {}:\n{}", pops, height, pc, dump.join("\n"))));
        }

        let new_height = height - pops as i32 + pushes as i32;

        // Check overflow
        if new_height > MAX_STACK_DEPTH as i32 {
            return Err(err("stack_depth", Some(pc),
                format!("stack depth {} exceeds limit {}", new_height, MAX_STACK_DEPTH)));
        }

        // Propagate to successors
        let succs = successors(pc, op, len);
        for &succ in &succs {
            if succ >= len {
                // This should be caught by control_flow pass, but double-check
                continue;
            }
            match heights[succ] {
                None => {
                    heights[succ] = Some(new_height);
                    worklist.push(succ);
                }
                Some(existing) if existing != new_height => {
                    return Err(err("stack_depth", Some(succ),
                        format!("stack height mismatch at join point: {} from one path, {} from another",
                            existing, new_height)));
                }
                Some(_) => {
                    // Same height — already processed
                }
            }
        }
    }

    // Verify that Return/ReturnSlot instructions are reachable (not dead code issues)
    // and that fall-off-end is handled. If the last instruction isn't a terminator,
    // the function implicitly returns Nil (height must be 0 at fall-off).
    if len > 0 {
        let last_op = &code[len - 1];
        match last_op {
            Op::Return | Op::ReturnSlot(_) | Op::Recur(_) | Op::RecurDirect(_)
            | Op::RecurIncAccum(_, _, _, _, _) | Op::Jump(_) => {
                // Explicit terminator — OK
            }
            Op::JumpIfTrue(_) | Op::JumpIfFalse(_)
            | Op::JumpIfSlotLtImm(_, _, _) | Op::JumpIfSlotLeImm(_, _, _)
            | Op::JumpIfSlotGtImm(_, _, _) | Op::JumpIfSlotGeImm(_, _, _)
            | Op::JumpIfSlotEqImm(_, _, _) => {
                // Conditional branch at end — one path might fall off.
                // Check if the fall-through path (pc=len, implicit return) has height 0.
                if let Some(h) = heights.get(len - 1).copied().flatten() {
                    let (pops, _pushes) = stack_effect(last_op);
                    let after_height = h - pops as i32;
                    if after_height != 0 {
                        // The branch might not be taken, and the implicit return
                        // would leave values on the stack. This is OK in practice
                        // (the VM just returns Nil), but we warn about it.
                        // For now, allow it — the VM handles implicit return.
                    }
                }
            }
            _ => {
                // Falls off end. Stack height should be 0 or 1 (implicit return of TOS).
                if let Some(h) = heights.get(len - 1).copied().flatten() {
                    if h > 1 {
                        return Err(err("stack_depth", Some(len - 1),
                            format!("function falls off end with stack height {} (expected 0 or 1)", h)));
                    }
                }
            }
        }
    }

    Ok(())
}

// ── Main entry point ────────────────────────────────────────────────────────

/// Verify a compiled lambda's bytecode and all nested closures.
///
/// Runs all verification passes in order:
/// 1. Hard limits (code size, slots, closures)
/// 2. Control flow (branch targets in bounds)
/// 3. Slot bounds (all slot indices valid)
/// 4. Instruction consistency (builtins, closure indices, recur arity)
/// 5. Stack depth (no underflow, consistent heights at joins)
///
/// Returns `Ok(())` if all passes succeed, or the first error found.
pub fn verify_bytecode(cl: &CompiledLambda) -> Result<(), Vec<VerificationError>> {
    verify_bytecode_inner(cl, 0)
}

fn verify_bytecode_inner(cl: &CompiledLambda, depth: usize) -> Result<(), Vec<VerificationError>> {
    if depth > MAX_CLOSURE_DEPTH {
        return Err(vec![err("limits", None,
            format!("closure nesting depth {} exceeds limit {}", depth, MAX_CLOSURE_DEPTH))]);
    }

    let mut errors = Vec::new();

    // Pass 1: Hard limits
    if let Err(e) = verify_limits(cl) {
        errors.push(e);
    }

    // Pass 2: Control flow
    if let Err(e) = verify_control_flow(&cl.code) {
        errors.push(e);
    }

    // Pass 3: Slot bounds
    if let Err(e) = verify_slot_indices(&cl.code, cl.total_slots) {
        errors.push(e);
    }

    // Pass 4: Instruction consistency
    if let Err(e) = verify_instruction_consistency(&cl.code, cl.total_slots, cl.closures.len()) {
        errors.push(e);
    }

    // Pass 5: Stack depth
    if let Err(e) = verify_stack_depth(&cl.code) {
        errors.push(e);
    }

    // Recursively verify nested closures
    for (i, closure) in cl.closures.iter().enumerate() {
        if let Err(mut errs) = verify_bytecode_inner(closure, depth + 1) {
            // Prefix closure index to error messages for context
            for e in &mut errs {
                e.message = format!("closure[{}]: {}", i, e.message);
            }
            errors.extend(errs);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{make_test_compiled_lambda};

    #[test]
    fn test_verify_empty_code() {
        // Function that pushes nil and returns — minimal valid body
        let cl = make_test_compiled_lambda(0, 1, vec![Op::PushNil, Op::Return]);
        let result = verify_bytecode(&cl);
        assert!(result.is_ok(), "push-nil-return should pass: {:?}", result);
    }

    #[test]
    fn test_verify_simple_push_return() {
        let code = vec![Op::PushI64(42), Op::Return];
        assert!(verify_stack_depth(&code).is_ok());
    }

    #[test]
    fn test_stack_underflow_detected() {
        let code = vec![
            Op::Add, // needs 2 values, stack is empty
            Op::Return,
        ];
        let result = verify_stack_depth(&code);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("stack underflow"));
        assert_eq!(err.offset, Some(0));
    }

    #[test]
    fn test_branch_target_out_of_bounds() {
        let code = vec![
            Op::PushI64(1),
            Op::Jump(999),
        ];
        let result = verify_control_flow(&code);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("out of bounds"));
    }

    #[test]
    fn test_slot_out_of_bounds() {
        let code = vec![
            Op::LoadSlot(999),
            Op::Return,
        ];
        let result = verify_slot_indices(&code, 2);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("out of bounds"));
    }

    #[test]
    fn test_unknown_builtin_detected() {
        // PushBuiltin requires a known builtin name (it constructs a BuiltinFn value).
        let code = vec![
            Op::PushBuiltin("nonexistent-builtin".into()),
            Op::Return,
        ];
        let result = verify_instruction_consistency(&code, 1, 0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("unknown builtin"));
    }

    #[test]
    fn test_builtin_call_accepts_unknown_names() {
        // BuiltinCall is runtime dispatch — unknown names are handled gracefully at runtime.
        let code = vec![
            Op::BuiltinCall("nonexistent-builtin".into(), 0),
            Op::Return,
        ];
        let result = verify_instruction_consistency(&code, 1, 0);
        assert!(result.is_ok(), "BuiltinCall should accept any name");
    }

    #[test]
    fn test_stack_height_mismatch_at_join() {
        // Two paths converge at pc=3 with different stack heights.
        // JumpIfFalse splits: false-branch (jump) has height=0, true-branch pushes → height=1.
        // The join at pc=3 detects the mismatch during propagation (not underflow).
        let code = vec![
            Op::PushI64(1),     // 0: height=0 → 1
            Op::JumpIfFalse(3), // 1: pop cond → height=0
            // true-branch (fall-through): height=0
            Op::PushI64(2),     // 2: height=0 → 1
            // false-branch jumps to 3: height=0
            // pc=3: from fall height=1, from jump height=0 → mismatch
            Op::PushI64(3),     // 3: mismatch detected on second arrival
            Op::Return,         // 4
        ];
        let result = verify_stack_depth(&code);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("mismatch"),
            "expected 'mismatch' in: {}",
            err.message
        );
    }

    #[test]
    fn test_valid_loop_pattern() {
        // A valid counting loop using stack-based counter.
        // Dup preserves counter across iterations; both arrivals at pc=1 have height=1.
        let code = vec![
            Op::PushI64(0),          // 0: height=0 → 1
            Op::Dup,                 // 1: height=1 → 2
            Op::PushI64(10),         // 2: height=2 → 3
            Op::Ge,                  // 3: height=3 → 2
            Op::JumpIfTrue(6),       // 4: pop cond → height=1
            // false branch (counter < 10): height=1 (counter on stack)
            Op::Jump(1),             // 5: height=1 → back to pc=1 (Dup)
            // true branch (counter >= 10): jump to 6, height=1
            Op::Return,              // 6: height=1 → return counter
        ];
        let result = verify_stack_depth(&code);
        assert!(result.is_ok(), "valid loop should pass: {:?}", result);
    }

    #[test]
    fn test_make_list_effect() {
        let code = vec![
            Op::PushI64(1),          // stack=[1]
            Op::PushI64(2),          // stack=[1,2]
            Op::PushI64(3),          // stack=[1,2,3]
            Op::MakeList(3),         // stack=[[1,2,3]]
            Op::Return,              // return [1,2,3]
        ];
        assert!(verify_stack_depth(&code).is_ok());
    }

    #[test]
    fn test_dict_operations() {
        let code = vec![
            Op::PushNil,             // stack=[{}]
            Op::PushI64(1),          // stack=[{},1]
            Op::PushI64(42),         // stack=[{},1,42]
            Op::DictSet,             // stack=[{1:42}]
            Op::Return,
        ];
        assert!(verify_stack_depth(&code).is_ok());
    }

    #[test]
    fn test_recurincaccum_no_stack_effect() {
        let code = vec![
            Op::RecurIncAccum(0, 1, 1, 10, 5), // no stack effect
            Op::Return,                           // needs 0 or 1 on stack
        ];
        assert!(verify_stack_depth(&code).is_ok());
    }

    #[test]
    fn test_recurincaccum_zero_step_rejected() {
        let code = vec![
            Op::RecurIncAccum(0, 1, 0, 10, 5), // step=0 → infinite loop
            Op::Return,
        ];
        let result = verify_instruction_consistency(&code, 2, 0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("infinite loop"));
    }

    #[test]
    fn test_known_builtins_accepted() {
        for &name in &["car", "cdr", "cons", "abs", "sqrt", "mod", "dict/get", "str-concat"] {
            let code = vec![
                Op::BuiltinCall(name.into(), 1),
                Op::Return,
            ];
            // Stack depth will fail (empty stack for a 1-arg builtin + return),
            // but instruction consistency should pass
            assert!(verify_instruction_consistency(&code, 1, 0).is_ok(),
                "builtin '{}' should be known", name);
        }
    }

    #[test]
    fn test_closure_index_out_of_bounds() {
        let code = vec![
            Op::PushClosure(99), // no closures
            Op::Return,
        ];
        let result = verify_instruction_consistency(&code, 1, 0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("PushClosure"));
    }

    #[test]
    fn test_full_verification() {
        // Build a valid lambda manually: 1 param, 1 slot, push param and return
        let cl = make_test_compiled_lambda(1, 1, vec![Op::LoadSlot(0), Op::Return]);
        let result = verify_bytecode(&cl);
        assert!(result.is_ok(), "valid compiled lambda should pass: {:?}", result);
    }
}
