use crate::helpers::is_truthy;
use crate::types::{Env, EvalState, LispVal};

/// Replace `(old_name args...)` with `(new_name args...)` recursively in an expression.
fn replace_sym_call(expr: &LispVal, old_name: &str, new_name: &str) -> LispVal {
    match expr {
        LispVal::List(list) => {
            let replaced: Vec<LispVal> = list.iter().map(|e| replace_sym_call(e, old_name, new_name)).collect();
            // Check if this is a call to old_name
            if let Some(LispVal::Sym(s)) = replaced.first() {
                if s == old_name {
                    let mut result = replaced;
                    result[0] = LispVal::Sym(new_name.into());
                    return LispVal::List(result);
                }
            }
            LispVal::List(replaced)
        }
        other => other.clone(),
    }
}

/// Expand a macro call at compile time.
pub fn expand_macro_call(
    macro_val: &LispVal,
    unevaluated_args: &[LispVal],
) -> Result<LispVal, String> {
    match macro_val {
        LispVal::Macro {
            params,
            rest_param,
            body,
            closed_env,
        } => {
            let mut macro_env = Env::new();
            if let Ok(guard) = closed_env.read() {
                for (k, v) in guard.iter() {
                    macro_env.insert_mut(k.clone(), v.clone());
                }
            }
            // Bind params to UNEVALUATED args
            for (i, param) in params.iter().enumerate() {
                if let Some(arg) = unevaluated_args.get(i) {
                    macro_env.insert_mut(param.clone(), arg.clone());
                } else {
                    return Err(format!(
                        "macro: not enough args, expected {}",
                        params.len()
                    ));
                }
            }
            // Bind rest param
            if let Some(rest) = rest_param {
                let rest_args: Vec<LispVal> = unevaluated_args[params.len()..].to_vec();
                macro_env.insert_mut(rest.clone(), LispVal::List(rest_args));
            }
            // Evaluate macro body in macro_env
            let mut state = EvalState::new();
            crate::program::run_program(&[body.as_ref().clone()], &mut macro_env, &mut state)
        }
        _ => Err("not a macro".into()),
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Compiles (loop ((i init) ...) body) into flat opcodes with slot-indexed
// env. Falls back to lisp_eval for unsupported expressions.
//
// Supported body patterns:
//   (if TEST then-expr (recur ARG1 ARG2 ...))
//   (if TEST then-expr else-expr)
// where TEST and ARGs can use: Num, Sym (binding ref), +, -, *, /, =, <, <=, >, >=
//
// ~20-50x faster than tree-walking because:
//   - No string matching per eval step (flat opcode array, PC increment)
//   - No env linear scan (slot-indexed Vec<LispVal>)
//   - No AST traversal (compiled jump targets)
//   - No LispVal::List construction for recur args
// ---------------------------------------------------------------------------

/// Typed binary operation kind.
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
}

/// Compiled loop representation.
#[allow(dead_code)]
pub struct CompiledLoop {
    /// Number of binding slots
    num_slots: usize,
    /// Binding names (for fallback)
    slot_names: Vec<String>,
    /// Initial values for slots
    init_vals: Vec<LispVal>,
    /// Bytecode
    code: Vec<Op>,
    /// PC of the loop start (for recur jumps)
    loop_start_pc: usize,
    /// Captured outer env variables (name → value), placed in slots after bindings
    captured: Vec<(String, LispVal)>,
}

/// Compilation context
struct LoopCompiler {
    slot_map: Vec<String>, // slot index → binding name
    code: Vec<Op>,
    /// Outer env variables captured at compile time (name, value)
    captured: Vec<(String, LispVal)>,
    /// Pre-compiled inner lambdas
    closures: Vec<CompiledLambda>,
    /// Name of the function being compiled (for CallSelf detection)
    self_name: Option<String>,
    /// Active loops: (jump_target_pc, var_slot_indices)
    loop_stack: Vec<(usize, Vec<usize>)>,
    /// Outer function's slot_map — for closure capture of parent parameters/let-bindings.
    /// When compiling an inner lambda, this tells us which parent slots to capture at runtime.
    parent_slots: Vec<String>,
    /// Runtime captures: (name, outer_slot_index) — read from caller's slots at PushClosure time
    runtime_captures: Vec<(String, usize)>,
    /// Forward-referenced captures: names captured from outer env as Nil (pre-populated defines).
    /// These need LoadGlobal at runtime instead of LoadCaptured.
    forward_captures: Vec<String>,
    /// Variables that have set! in this scope — use LoadGlobal/StoreGlobal for them
    /// instead of LoadCaptured, since StoreGlobal writes to outer_env but LoadCaptured
    /// reads frozen captured values.
    set_target_globals: std::collections::HashSet<String>,
    /// Per-slot type info: true if slot is known to always hold Num(i64)
    slot_is_i64: Vec<bool>,
    /// Per-slot type info: true if slot is known to always hold Float(f64)
    slot_is_f64: Vec<bool>,
    /// Whether the last compile_expr call produced an i64 value on the stack.
    /// Used by callers (e.g., let-binding) to propagate type info to new slots.
    last_result_i64: bool,
    /// Whether the last compile_expr call produced an f64 value on the stack.
    /// Used by callers (e.g., let-binding) to propagate type info to new slots.
    last_result_f64: bool,
    /// When compiling a let-binding whose value is a lambda, this holds the binding name.
    /// The inner lambda compiler reads this to set self_name for recursive calls.
    pending_lambda_name: Option<String>,
}

impl LoopCompiler {
    fn new(slot_names: Vec<String>) -> Self {
        Self {
            slot_map: slot_names,
            code: Vec::new(),
            captured: Vec::new(),
            closures: Vec::new(),
            self_name: None,
            loop_stack: Vec::new(),
            parent_slots: Vec::new(),
            runtime_captures: Vec::new(),
            forward_captures: Vec::new(),
            set_target_globals: std::collections::HashSet::new(),
            slot_is_i64: Vec::new(),
            slot_is_f64: Vec::new(),
            last_result_i64: false,
            last_result_f64: false,
            pending_lambda_name: None,
        }
    }

    /// Look up binding name → slot index (bindings first, then captured env)
    fn slot_of(&self, name: &str) -> Option<usize> {
        if let Some(idx) = self.slot_map.iter().position(|s| s == name) {
            return Some(idx);
        }
        None
    }

    /// Mark a slot as known to always hold Num(i64)
    fn mark_slot_i64(&mut self, slot: usize) {
        while self.slot_is_i64.len() <= slot {
            self.slot_is_i64.push(false);
        }
        self.slot_is_i64[slot] = true;
        // i64 and f64 are mutually exclusive
        while self.slot_is_f64.len() <= slot {
            self.slot_is_f64.push(false);
        }
        self.slot_is_f64[slot] = false;
    }

    /// Mark a slot as known to always hold Float(f64)
    fn mark_slot_f64(&mut self, slot: usize) {
        while self.slot_is_f64.len() <= slot {
            self.slot_is_f64.push(false);
        }
        self.slot_is_f64[slot] = true;
        // i64 and f64 are mutually exclusive
        while self.slot_is_i64.len() <= slot {
            self.slot_is_i64.push(false);
        }
        self.slot_is_i64[slot] = false;
    }

    /// Check if a slot is known to always hold Num(i64)
    fn is_slot_i64(&self, slot: usize) -> bool {
        self.slot_is_i64.get(slot).copied().unwrap_or(false)
    }

    /// Return the captured var index (into self.captured) for a name.
    fn captured_idx(&self, name: &str) -> Option<usize> {
        self.captured.iter().position(|(s, _)| s == name)
    }

    /// Try to capture an unknown symbol from outer env. Returns true if captured.
    fn try_capture(&mut self, name: &str, outer_env: &Env) -> bool {
        if self.slot_of(name).is_some() || self.captured_idx(name).is_some() {
            return true;
        }
        if let Some(val) = outer_env.get(name) {
            self.captured.push((name.to_string(), val.clone()));
            // ALL captures from outer_env are "env captures" — the captured snapshot
            // is frozen at closure-creation time. If any inner lambda set!'s this
            // variable, the snapshot goes stale. To handle this, always use
            // LoadGlobal/StoreGlobal for env captures, which read/write the live
            // outer_env instead of the frozen snapshot.
            self.forward_captures.push(name.to_string());
            return true;
        }
        // Check if name is a slot in the parent (outer function's parameters/let-bindings)
        if let Some(parent_slot) = self.parent_slots.iter().position(|s| s == name) {
            // Record as a runtime capture — will be read from caller's slots at PushClosure time
            if self.runtime_captures.iter().all(|(n, _)| n != name) {
                self.runtime_captures.push((name.to_string(), parent_slot));
            }
            // Also add to captured list with a Nil placeholder so captured_idx() can find it.
            // The real value will be filled in at PushClosure time from the runtime slots.
            if self.captured_idx(name).is_none() {
                self.captured.push((name.to_string(), LispVal::Nil));
            }
            return true;
        }
        false
    }

    /// Maximum number of ops in a callee to be eligible for inlining.
    const INLINE_THRESHOLD: usize = 80;

    /// Extract a constant LispVal from an Op, if it's a pure constant push.
    fn const_val(op: &Op) -> Option<LispVal> {
        match op {
            Op::PushI64(n) => Some(LispVal::Num(*n)),
            Op::PushFloat(f) => Some(LispVal::Float(*f)),
            Op::PushBool(b) => Some(LispVal::Bool(*b)),
            Op::PushStr(s) => Some(LispVal::Str(s.clone())),
            Op::PushNil => Some(LispVal::Nil),
            _ => None,
        }
    }

    /// Emit a single op to push a constant LispVal onto the stack.
    fn emit_const(&mut self, val: &LispVal) {
        match val {
            LispVal::Num(n) => self.code.push(Op::PushI64(*n)),
            LispVal::Float(f) => self.code.push(Op::PushFloat(*f)),
            LispVal::Bool(b) => self.code.push(Op::PushBool(*b)),
            LispVal::Str(s) => self.code.push(Op::PushStr(s.clone())),
            LispVal::Nil => self.code.push(Op::PushNil),
            _ => {} // can't represent as a single const op
        }
    }

    /// Try constant folding: if the last n_args ops are all constants AND
    /// the captured function at idx is pure+compiled, evaluate at compile time
    /// and replace with a single constant op.
    fn try_const_fold(&mut self, idx: usize, n_args: usize) -> bool {
        if n_args == 0 || self.code.len() < n_args {
            return false;
        }

        // Check if callee is pure compiled
        let (is_pure, callee) = match self.captured.get(idx).map(|(_, v)| v) {
            Some(LispVal::Lambda {
                pure_type: Some(_),
                compiled: Some(ref cl),
                rest_param: None,
                ..
            }) => (true, cl.clone()),
            _ => return false,
        };
        if !is_pure {
            return false;
        }

        // Extract constant args from the last n_args ops
        let code_len = self.code.len();
        let mut const_args = Vec::with_capacity(n_args);
        for i in 0..n_args {
            match Self::const_val(&self.code[code_len - 1 - i]) {
                Some(v) => const_args.push(v),
                None => return false,
            }
        }
        const_args.reverse(); // we extracted in reverse order

        // Evaluate the compiled lambda with the constant args
        let mut state = EvalState::new();
        let mut env = Env::new();
        match run_compiled_lambda(&callee, &const_args, &mut env, &mut state) {
            Ok(result) => {
                // Remove the n_args constant ops
                self.code.truncate(code_len - n_args);
                // Emit single constant op with the result
                self.emit_const(&result);
                true
            }
            Err(_) => false,
        }
    }

    /// Try to inline a call to a captured pure compiled lambda.
    /// Returns true if inlined (caller should not emit CallCapturedRef/CallCaptured).
    /// `n_args` = number of args already compiled onto the stack.
    fn try_inline_call(&mut self, idx: usize, n_args: usize) -> bool {
        let callee = match &self.captured.get(idx).map(|(_, v)| v) {
            Some(LispVal::Lambda {
                compiled: Some(ref cl),
                rest_param: None,
                ..
            }) => cl.clone(),
            _ => return false,
        };

        // Don't inline if callee is too large
        if callee.code.len() > Self::INLINE_THRESHOLD {
            return false;
        }
        // Don't inline if callee has CallSelf (recursive) — would need special handling
        if callee.code.iter().any(|op| matches!(op, Op::CallSelf(_))) {
            return false;
        }
        // Don't inline if callee has PushClosure — closures complicate inlining
        if callee
            .code
            .iter()
            .any(|op| matches!(op, Op::PushClosure(_)))
        {
            return false;
        }
        // Don't inline if callee has BuiltinCall (storage, context ops need shared state)
        if callee
            .code
            .iter()
            .any(|op| matches!(op, Op::BuiltinCall(_, _)))
        {
            return false;
        }
        // Don't inline if callee has CallCaptured/CallCapturedRef/BuiltinCall
        // that call non-inlinable things — deep inlining is risky
        // Actually, we CAN inline these — they'll just stay as call ops.
        // But skip if arg count doesn't match
        if n_args != callee.num_param_slots {
            return false;
        }

        // Slot remapping: callee slots 0..N map to caller slots base..base+N
        let base = self.slot_map.len();

        // Extend slot map to cover ALL callee slots (params + let bindings + temporaries).
        // This prevents collisions between callee's internal slots and caller's slots.
        for i in 0..callee.total_slots {
            self.slot_map.push(format!("__inline_{}_{}", idx, base + i));
        }

        // Store args from stack into remapped callee param slots (reverse order — stack is LIFO)
        for i in (0..n_args).rev() {
            self.code.push(Op::StoreSlot(base + i));
        }

        // Merge callee's captured vars into caller's captured list.
        // Build a mapping: callee captured idx → caller captured idx
        let mut captured_remap: Vec<usize> = Vec::new();
        for (name, val) in callee.captured.read().unwrap().iter() {
            if let Some(existing_idx) = self.captured_idx(name) {
                // Already captured by caller
                captured_remap.push(existing_idx);
            } else {
                // Add to caller's captured list
                let new_idx = self.captured.len();
                self.captured.push((name.clone(), val.clone()));
                captured_remap.push(new_idx);
            }
        }

        // Emit callee ops with slot offset + captured remap + jump target offset
        let callee_code_len = callee.code.len();

        // Wrap inlined call with trace for stack traces
        if let Some(ref name) = callee.name {
            self.code.push(Op::TracePush(name.clone()));
        }

        let code_start = self.code.len(); // AFTER TracePush, so jump offsets are correct

        for (i, op) in callee.code.iter().enumerate() {
            if i == callee_code_len - 1 && matches!(op, Op::Return | Op::ReturnSlot(_)) {
                // If ReturnSlot(s), push the value onto the stack before breaking
                if let Op::ReturnSlot(s) = op {
                    self.code.push(Op::LoadSlot(base + s));
                }
                break;
            }
            self.code
                .push(remap_op(op, base, &captured_remap, code_start));
        }

        if let Some(ref name) = callee.name {
            self.code.push(Op::TracePop);
        }

        true
    }

    /// Try to compile an expression. Returns false if unsupported.
    fn compile_expr(&mut self, expr: &LispVal, outer_env: &Env) -> bool {
        self.last_result_i64 = false; // default: unknown type
        self.last_result_f64 = false; // default: unknown type
        match expr {
            LispVal::Num(n) => {
                self.code.push(Op::PushI64(*n));
                self.last_result_i64 = true;
                true
            }
            LispVal::Float(f) => {
                self.code.push(Op::PushFloat(*f));
                self.last_result_f64 = true;
                true
            }
            LispVal::Bool(b) => {
                self.code.push(Op::PushBool(*b));
                true
            }
            LispVal::Str(s) => {
                self.code.push(Op::PushStr(s.clone()));
                true
            }
            LispVal::Nil => {
                self.code.push(Op::PushNil);
                true
            }
            LispVal::Sym(name) => {
                // Literal booleans and nil — don't capture as variables
                match name.as_str() {
                    "true" => {
                        self.code.push(Op::PushBool(true));
                        return true;
                    }
                    "false" => {
                        self.code.push(Op::PushBool(false));
                        return true;
                    }
                    "nil" => {
                        self.code.push(Op::PushNil);
                        return true;
                    }
                    _ => {}
                }
                if let Some(slot) = self.slot_of(name) {
                    self.code.push(Op::LoadSlot(slot));
                    self.last_result_i64 = self.is_slot_i64(slot);
                    true
                } else if name.starts_with('*') && name.ends_with('*') && name.len() > 2 {
                    // Global variable (*foo*): use live env lookup, not frozen capture
                    self.code.push(Op::LoadGlobal(name.to_string()));
                    self.last_result_i64 = false;
                    self.last_result_f64 = false;
                    true
                } else if let Some(idx) = self.captured_idx(name) {
                    // Check if this is a forward-referenced capture or set! target — use LoadGlobal
                    if self.forward_captures.contains(&name.to_string())
                        || self.set_target_globals.contains(name)
                    {
                        self.code.push(Op::LoadGlobal(name.to_string()));
                        self.last_result_i64 = false;
                        self.last_result_f64 = false;
                        return true;
                    }
                    self.code.push(Op::LoadCaptured(idx));
                    true
                } else if self.try_capture(name, outer_env) {
                    // Just captured — check if it's a forward reference or set! target
                    if self.forward_captures.contains(&name.to_string())
                        || self.set_target_globals.contains(name)
                    {
                        self.code.push(Op::LoadGlobal(name.to_string()));
                        self.last_result_i64 = false;
                        self.last_result_f64 = false;
                        return true;
                    }
                    let idx = self.captured_idx(name).unwrap();
                    self.code.push(Op::LoadCaptured(idx));
                    true
                } else if crate::helpers::is_builtin_name(name) {
                    self.code.push(Op::PushBuiltin(name.to_string()));
                    self.last_result_i64 = false;
                    self.last_result_f64 = false;
                    true
                } else if let Some(ctor) = crate::helpers::lookup_constructor(name) {
                    // Nullary type constructor used as a value (e.g., None)
                    if ctor.n_fields == 0 {
                        self.code.push(Op::ConstructTag(
                            ctor.type_name.clone(),
                            ctor.variant_id,
                            0,
                        ));
                        true
                    } else {
                        // N-ary constructor used as a value — push as callable
                        // For now, compilation fails (constructors must be called directly)
                        false
                    }
                } else if self.set_target_globals.contains(name) {
                    // Variable targeted by set! but not in local slots or captured —
                    // it's a runtime capture (let-bound in enclosing lambda).
                    // Emit LoadGlobal; will be resolved from outer_env at runtime.
                    self.code.push(Op::LoadGlobal(name.to_string()));
                    self.last_result_i64 = false;
                    self.last_result_f64 = false;
                    true
                } else {
                    // Unknown symbol — push as literal (e.g., keyword type descriptors like :int, :str)
                    self.code.push(Op::PushLiteral(LispVal::Sym(name.clone())));
                    self.last_result_i64 = false;
                    self.last_result_f64 = false;
                    true
                }
            }
            LispVal::List(list) if list.is_empty() => {
                self.code.push(Op::PushNil);
                true
            }
            LispVal::List(list) => {
                if let LispVal::Sym(op) = &list[0] {
                    match op.as_str() {
                        // require: no-op special form (stdlib prelude handles module loading)
                        "require" => {
                            if list.len() != 2 {
                                return false;
                            }
                            self.code.push(Op::PushLiteral(LispVal::Nil));
                            self.last_result_i64 = false;
                            self.last_result_f64 = false;
                            true
                        }
                        // contract: typed lambda with runtime checks
                        // (contract ((x :int) (y :str) -> :str) body)
                        // (contract (x :int y :str -> :str) body)
                        // (contract (x :int) body)
                        "contract" => {
                            if list.len() < 3 { return false; }
                            // Parse the contract spec (list[1]) and body (list[2..])
                            let spec = &list[1];
                            // Contract only uses the first body expression (multi-body not supported)
                            let body = list[2].clone();

                            // Parse param names, types, and optional return type
                            let mut param_names: Vec<String> = Vec::new();
                            let mut param_types: Vec<LispVal> = Vec::new();
                            let mut return_type: Option<LispVal> = None;

                            // Check if spec is grouped: ((x :int) (y :str) -> :str)
                            // or flat: (x :int y :str -> :str)
                            let spec_list = match spec {
                                LispVal::List(l) => l,
                                _ => return false,
                            };

                            // Find -> or → separator
                            let arrow_idx = spec_list.iter().position(|v| {
                                matches!(v, LispVal::Sym(s) if s == "->" || s == "→")
                            });

                            let param_specs: &[LispVal] = if let Some(ai) = arrow_idx {
                                // Return type is after arrow
                                if ai + 1 < spec_list.len() {
                                    return_type = Some(spec_list[ai + 1].clone());
                                }
                                &spec_list[..ai]
                            } else {
                                spec_list.as_slice()
                            };

                            // Parse param specs
                            if param_specs.is_empty() {
                                // No params, no return type arrow — just a bare contract
                            } else if param_specs.len() == 1 {
                                // Could be single grouped param (x :int) or flat single (x :int)
                                match &param_specs[0] {
                                    LispVal::List(inner) => {
                                        // Grouped: (x :int)
                                        if inner.len() >= 2 {
                                            if let LispVal::Sym(name) = &inner[0] {
                                                param_names.push(name.clone());
                                                param_types.push(inner[1].clone());
                                            }
                                        }
                                    }
                                    LispVal::Sym(name) => {
                                        // Flat: (x :int) — name is first, type is second in spec_list
                                        param_names.push(name.clone());
                                        if param_specs.len() >= 2 {
                                            param_types.push(param_specs[1].clone());
                                        } else {
                                            param_types.push(LispVal::Sym(":any".into()));
                                        }
                                    }
                                    _ => return false,
                                }
                            } else {
                                // Check if grouped (all elements are lists) or flat
                                let all_grouped = param_specs.iter().all(|v| matches!(v, LispVal::List(_)));
                                if all_grouped {
                                    for item in param_specs {
                                        if let LispVal::List(inner) = item {
                                            if inner.len() >= 2 {
                                                if let LispVal::Sym(name) = &inner[0] {
                                                    param_names.push(name.clone());
                                                    param_types.push(inner[1].clone());
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    // Flat: (x :int y :str)
                                    let mut i = 0;
                                    while i + 1 < param_specs.len() {
                                        if let LispVal::Sym(name) = &param_specs[i] {
                                            param_names.push(name.clone());
                                            param_types.push(param_specs[i + 1].clone());
                                        }
                                        i += 2;
                                    }
                                }
                            }

                            // Desugar to: (lambda (params...) (begin param-checks... body ... return-check))
                            let mut body_forms: Vec<LispVal> = Vec::new();

                            // Insert param type checks: (contract-check-param "x" x :int)
                            for (idx, (name, ty)) in param_names.iter().zip(param_types.iter()).enumerate() {
                                body_forms.push(LispVal::List(vec![
                                    LispVal::Sym("contract-check-param".into()),
                                    LispVal::Str(name.clone()),
                                    LispVal::Sym(name.clone()),
                                    ty.clone(),
                                ]));
                            }

                            // The actual body
                            body_forms.push(body.clone());

                            // Return type check: wrap in let to capture result
                            if let Some(ref ret) = return_type {
                                let result_name = format!("__contract_result_{}", param_names.len());
                                body_forms.pop(); // remove body
                                // Wrap let body in begin so both check-return and result-name execute
                                let let_body = LispVal::List(vec![
                                    LispVal::Sym("begin".into()),
                                    LispVal::List(vec![
                                        LispVal::Sym("contract-check-return".into()),
                                        LispVal::Sym(result_name.clone()),
                                        ret.clone(),
                                    ]),
                                    LispVal::Sym(result_name.clone()),
                                ]);
                                body_forms.push(LispVal::List(vec![
                                    LispVal::Sym("let".into()),
                                    LispVal::List(vec![
                                        LispVal::List(vec![LispVal::Sym(result_name), body.clone()]),
                                    ]),
                                    let_body,
                                ]));
                            }

                            let desugared = LispVal::List(vec![
                                LispVal::Sym("lambda".into()),
                                LispVal::List(param_names.into_iter().map(LispVal::Sym).collect()),
                                if body_forms.len() == 1 { body_forms.into_iter().next().unwrap() } else {
                                    LispVal::List(vec![LispVal::Sym("begin".into())].into_iter().chain(body_forms.into_iter()).collect())
                                },
                            ]);

                            self.compile_expr(&desugared, outer_env)
                        }
                        // quote: return the datum unevaluated
                        "quote" => {
                            if list.len() != 2 {
                                return false;
                            }
                            // Push the quoted value as a literal
                            self.code.push(Op::PushLiteral(list[1].clone()));
                            self.last_result_i64 = false;
                            self.last_result_f64 = false;
                            true
                        }
                        // quasiquote: expand at compile time, then compile the expansion
                        "quasiquote" => {
                            if list.len() != 2 {
                                return false;
                            }
                            match crate::dispatch::quasiquote::expand_quasiquote(&list[1]) {
                                Ok(expansion) => self.compile_expr(&expansion, outer_env),
                                Err(_) => false,
                            }
                        }
                        // unquote / unquote-splicing: only valid inside quasiquote (expanded away)
                        "unquote" | "unquote-splicing" => {
                            // These should never appear at top level after quasiquote expansion.
                            // Treat as error (return false → fall back to interpreter).
                            false
                        }
                        // macroexpand: expand a macro call and return the expansion as data
                        "macroexpand" => {
                            if list.len() != 2 { return false; }
                            match &list[1] {
                                LispVal::List(ref form_list) if !form_list.is_empty() => {
                                    if let LispVal::Sym(ref name) = form_list[0] {
                                        if let Some(macro_val) = outer_env.get(name) {
                                            if matches!(macro_val, LispVal::Macro { .. }) {
                                                match expand_macro_call(&macro_val, &form_list[1..]) {
                                                    Ok(expansion) => {
                                                        self.code.push(Op::PushLiteral(expansion));
                                                        self.last_result_i64 = false;
                                                        self.last_result_f64 = false;
                                                        return true;
                                                    }
                                                    Err(_) => return false,
                                                }
                                            }
                                        }
                                        // Not a macro — return the form as-is
                                        self.code.push(Op::PushLiteral(list[1].clone()));
                                        self.last_result_i64 = false;
                                        self.last_result_f64 = false;
                                        return true;
                                    }
                                }
                                _ => {}
                            }
                            false
                        }
                        // begin/progn: evaluate all forms, return last value
                        "begin" | "progn" => {
                            if list.is_empty() {
                                self.code.push(Op::PushNil);
                                self.last_result_i64 = false;
                                self.last_result_f64 = false;
                                return true;
                            }
                            for (i, form) in list[1..].iter().enumerate() {
                                if !self.compile_expr(form, outer_env) {
                                    return false;
                                }
                                // Pop all but last result
                                if i < list.len() - 2 {
                                    self.code.push(Op::Pop);
                                }
                            }
                            true
                        }
                        // Variadic arithmetic: chain binary ops
                        "+" | "-" | "*" | "/" | "%" => {
                            let opcode = match op.as_str() {
                                "+" => Op::Add,
                                "-" => Op::Sub,
                                "*" => Op::Mul,
                                "/" => Op::Div,
                                "%" => Op::Mod,
                                _ => unreachable!(),
                            };
                            if list.len() < 2 {
                                return false;
                            }
                            // Unary minus: (- x) → push 0, push x, sub
                            if list.len() == 2 && op.as_str() == "-" {
                                self.code.push(Op::PushI64(0));
                                if !self.compile_expr(&list[1], outer_env) {
                                    return false;
                                }
                                self.code.push(Op::Sub);
                                // Result type follows operand
                                return true;
                            }
                            if list.len() < 3 {
                                return false;
                            }
                            if !self.compile_expr(&list[1], outer_env) {
                                return false;
                            }
                            let mut all_i64 = self.last_result_i64;
                            let mut any_f64 = self.last_result_f64;
                            for arg in &list[2..] {
                                if !self.compile_expr(arg, outer_env) {
                                    return false;
                                }
                                all_i64 = all_i64 && self.last_result_i64;
                                any_f64 = any_f64 || self.last_result_f64;
                                self.code.push(opcode.clone());
                            }
                            // For int arithmetic (+, -, *), if all operands were i64 and none f64, result is i64
                            if all_i64 && !any_f64 && matches!(op.as_str(), "+" | "-" | "*") {
                                self.last_result_i64 = true;
                            }
                            // If any operand was f64, result is f64 for arithmetic ops
                            if any_f64 && matches!(op.as_str(), "+" | "-" | "*" | "/" | "%") {
                                self.last_result_f64 = true;
                            }
                            true
                        }
                        // Variadic comparison: chain binary ops
                        "=" | "<" | "<=" | ">" | ">=" => {
                            let opcode = match op.as_str() {
                                "=" => Op::Eq,
                                "<" => Op::Lt,
                                "<=" => Op::Le,
                                ">" => Op::Gt,
                                ">=" => Op::Ge,
                                _ => unreachable!(),
                            };
                            if list.len() < 3 {
                                return false;
                            }
                            if !self.compile_expr(&list[1], outer_env) {
                                return false;
                            }
                            for arg in &list[2..] {
                                if !self.compile_expr(arg, outer_env) {
                                    return false;
                                }
                                self.code.push(opcode.clone());
                            }
                            true
                        }
                        "not" => {
                            let arg = match list.get(1) {
                                Some(a) => a,
                                None => return false,
                            };
                            if !self.compile_expr(arg, outer_env) {
                                return false;
                            }
                            self.code.push(Op::Not);
                            true
                        }
                        // Nested if: (if test then else) — compiles to jump instructions
                        "if" => {
                            let test = match list.get(1) {
                                Some(t) => t,
                                None => return false,
                            };
                            let then_branch = match list.get(2) {
                                Some(t) => t,
                                None => return false,
                            };
                            let else_branch = list.get(3);
                            if !self.compile_expr(test, outer_env) {
                                return false;
                            }
                            let jf_idx = self.code.len();
                            self.code.push(Op::JumpIfFalse(0));
                            if !self.compile_expr(then_branch, outer_env) {
                                return false;
                            }
                            let jmp_idx = self.code.len();
                            self.code.push(Op::Jump(0));
                            let else_start = self.code.len();
                            self.code[jf_idx] = Op::JumpIfFalse(else_start);
                            if let Some(ee) = else_branch {
                                if !self.compile_expr(ee, outer_env) {
                                    return false;
                                }
                            } else {
                                self.code.push(Op::PushNil);
                            }
                            self.code[jmp_idx] = Op::Jump(self.code.len());
                            true
                        }
                        // recur: compile args, store into loop var slots, jump to loop start
                        "recur" => {
                            if let Some((loop_start, var_slots)) = self.loop_stack.last().cloned() {
                                let n_args = list.len() - 1;
                                if n_args != var_slots.len() {
                                    return false;
                                }
                                for arg in &list[1..] {
                                    if !self.compile_expr(arg, outer_env) {
                                        return false;
                                    }
                                }
                                // Store args into loop var slots in reverse order
                                // (StoreSlot pops from stack, and stack is LIFO)
                                for &slot_idx in var_slots.iter().rev() {
                                    self.code.push(Op::StoreSlot(slot_idx));
                                }
                                self.code.push(Op::Jump(loop_start));
                                true
                            } else {
                                false
                            }
                        }
                        // and: short-circuit, returns first falsy or last value
                        // Pattern: compile arg; Dup; JumpIfFalse(end); Pop; ...next arg...
                        "and" => {
                            if list.len() < 2 {
                                return false;
                            }
                            let mut jump_patches: Vec<usize> = Vec::new();
                            for (i, arg) in list[1..].iter().enumerate() {
                                if !self.compile_expr(arg, outer_env) {
                                    return false;
                                }
                                if i + 1 < list.len() - 1 {
                                    self.code.push(Op::Dup);
                                    let jf_idx = self.code.len();
                                    self.code.push(Op::JumpIfFalse(0));
                                    self.code.push(Op::Pop);
                                    jump_patches.push(jf_idx);
                                }
                            }
                            let end_pc = self.code.len();
                            for idx in jump_patches {
                                self.code[idx] = Op::JumpIfFalse(end_pc);
                            }
                            true
                        }
                        // or: short-circuit, returns first truthy or last value
                        "or" => {
                            if list.len() < 2 {
                                return false;
                            }
                            let mut jump_patches: Vec<usize> = Vec::new();
                            for (i, arg) in list[1..].iter().enumerate() {
                                if !self.compile_expr(arg, outer_env) {
                                    return false;
                                }
                                if i + 1 < list.len() - 1 {
                                    self.code.push(Op::Dup);
                                    let jt_idx = self.code.len();
                                    self.code.push(Op::JumpIfTrue(0));
                                    self.code.push(Op::Pop);
                                    jump_patches.push(jt_idx);
                                }
                            }
                            let end_pc = self.code.len();
                            for idx in jump_patches {
                                self.code[idx] = Op::JumpIfTrue(end_pc);
                            }
                            true
                        }
                        // progn / begin: evaluate all, return last
                        "progn" | "begin" | "do" => {
                            if list.len() < 2 {
                                self.code.push(Op::PushNil);
                                return true;
                            }
                            for (i, arg) in list[1..].iter().enumerate() {
                                if !self.compile_expr(arg, outer_env) {
                                    return false;
                                }
                                if i + 1 < list.len() - 1 {
                                    self.code.push(Op::Pop);
                                }
                            }
                            true
                        }
                        // cond: multi-branch — chained JumpIfFalse
                        // (cond (t1 r1) (t2 r2) (else rN))
                        "cond" => {
                            if list.len() < 2 {
                                return false;
                            }
                            let mut end_jumps: Vec<usize> = Vec::new();
                            let mut i = 1;
                            let mut last_was_else = false;
                            while i < list.len() {
                                let clause = match list.get(i) {
                                    Some(LispVal::List(c)) if c.len() >= 2 => c.clone(),
                                    _ => {
                                        return false;
                                    }
                                };
                                // else/t clause — just compile result
                                if clause[0] == LispVal::Sym("else".into())
                                    || clause[0] == LispVal::Sym("t".into())
                                {
                                    last_was_else = true;
                                    // Compile all body expressions (last one is the value)
                                    for (bi, body) in clause.iter().skip(1).enumerate() {
                                        if bi == clause.len() - 2 {
                                            if !self.compile_expr(body, outer_env) {
                                                return false;
                                            }
                                        } else {
                                            if !self.compile_expr(body, outer_env) {
                                                return false;
                                            }
                                        }
                                    }
                                    break;
                                }
                                // compile test
                                if !self.compile_expr(&clause[0], outer_env) {
                                    return false;
                                }
                                let jf_idx = self.code.len();
                                self.code.push(Op::JumpIfFalse(0)); // placeholder
                                                                    // compile result
                                if !self.compile_expr(&clause[1], outer_env) {
                                    return false;
                                }
                                end_jumps.push(self.code.len());
                                self.code.push(Op::Jump(0)); // jump to end
                                                             // patch JF to skip to next clause
                                self.code[jf_idx] = Op::JumpIfFalse(self.code.len());
                                i += 1;
                            }
                            // If the last clause wasn't `else`, the last JumpIfFalse
                            // lands here with empty stack. Push nil so Return has a value.
                            if !last_was_else {
                                self.code.push(Op::PushNil);
                            }
                            // patch all end jumps
                            let end_pc = self.code.len();
                            for idx in end_jumps {
                                self.code[idx] = Op::Jump(end_pc);
                            }
                            true
                        }
                        // match: (match expr clause1 clause2 ...)
                        // Each clause: (pattern body)
                        "match" => {
                            if list.len() < 3 {
                                return false;
                            }
                            let scrutinee = &list[1];
                            // Compile scrutinee and store in a temp slot
                            if !self.compile_expr(scrutinee, outer_env) {
                                return false;
                            }
                            let scrutinee_slot = self.slot_map.len();
                            self.slot_map.push("__match_scrutinee".to_string());
                            self.code.push(Op::StoreSlot(scrutinee_slot));

                            let mut end_jumps: Vec<usize> = Vec::new();
                            let mut i = 2;
                            let mut last_was_else = false;

                            while i < list.len() {
                                let clause = match list.get(i) {
                                    Some(LispVal::List(c)) if c.len() >= 2 => c.clone(),
                                    _ => return false,
                                };
                                let pattern = &clause[0];
                                let body = &clause[1];

                                // else clause — always matches
                                if *pattern == LispVal::Sym("else".into()) {
                                    last_was_else = true;
                                    // Add bindings from the clause (none for else)
                                    let _bindings_start = self.slot_map.len();
                                    if !self.compile_expr(body, outer_env) {
                                        return false;
                                    }
                                    // Restore slot_map (no new bindings for else)
                                    break;
                                }

                                // Compile pattern match check → pushes Bool on stack
                                // Also extracts bindings into slots (adding to slot_map)
                                let bindings_start = self.slot_map.len();
                                if !self.compile_pattern_check(pattern, scrutinee_slot, outer_env) {
                                    return false;
                                }
                                let _bindings_end = self.slot_map.len();

                                let jf_idx = self.code.len();
                                self.code.push(Op::JumpIfFalse(0)); // placeholder

                                // Pattern matched! Bindings are now in slots.
                                // Compile the body with bindings visible.
                                if !self.compile_expr(body, outer_env) {
                                    return false;
                                }

                                // Jump to end of match
                                end_jumps.push(self.code.len());
                                self.code.push(Op::Jump(0));

                                // Patch: if pattern didn't match, jump here
                                self.code[jf_idx] = Op::JumpIfFalse(self.code.len());

                                // Restore slot_map (remove pattern bindings)
                                self.slot_map.truncate(bindings_start);

                                i += 1;
                            }

                            // If no else clause, push nil for no-match
                            if !last_was_else {
                                self.code.push(Op::PushNil);
                            }

                            // Patch all end jumps
                            let end_pc = self.code.len();
                            for idx in end_jumps {
                                self.code[idx] = Op::Jump(end_pc);
                            }

                            // Remove scrutinee slot from slot_map
                            // (We can't easily remove from middle, but it's at scrutinee_slot)
                            // The scrutinee slot is no longer needed; truncate if it was last.
                            // Since we may have added bindings after, just leave it.
                            // The slot is allocated but unused after match — harmless.

                            true
                        }
                        // let: (let ((x init) ...) body)
                        "let" | "let*" => {
                            // Named let: (let name ((var init) ...) body ...)
                            // Desugar to: (loop ((var init) ...) body ...) with (name args...) → (recur args...)
                            if let Some(LispVal::Sym(loop_name)) = list.get(1) {
                                let bindings = match list.get(2) {
                                    Some(LispVal::List(b)) => b,
                                    _ => return false,
                                };
                                let body_forms = if list.len() > 3 { &list[3..] } else { &[] };
                                let body = if body_forms.len() == 1 {
                                    body_forms[0].clone()
                                } else {
                                    LispVal::List(std::iter::once(LispVal::Sym("begin".into()))
                                        .chain(body_forms.iter().cloned()).collect())
                                };
                                // Replace (loop_name args...) with (recur args...) in body
                                let body = replace_sym_call(&body, loop_name, "recur");
                                let new_form = LispVal::List(vec![
                                    LispVal::Sym("loop".into()),
                                    LispVal::List(bindings.clone()),
                                    body,
                                ]);
                                return self.compile_expr(&new_form, outer_env);
                            }
                            let bindings = match list.get(1) {
                                Some(LispVal::List(b)) => b,
                                _ => return false,
                            };
                            let body = match list.get(2) {
                                Some(b) => b,
                                _ => return false,
                            };
                            // Track slots that need cleanup (only newly allocated ones)
                            let let_start = self.slot_map.len();
                            // Save area: beyond all slots this let could allocate.
                            // Worst case: all bindings are new, each gets one slot.
                            let save_base = let_start + bindings.len();
                            // Track slots we shadow so we can restore them
                            let mut shadowed: Vec<usize> = Vec::new(); // (original_slot, save at save_base+i)
                            let mut shadow_idx = 0usize;
                            let mut all_ok = true;
                            for binding in bindings {
                                match binding {
                                    LispVal::List(pair) if pair.len() >= 2 => {
                                        if let LispVal::Sym(name) = &pair[0] {
                                            // If value is a lambda, set self_name for recursion
                                            let is_lambda = matches!(
                                                &pair[1],
                                                LispVal::List(l) if !l.is_empty() && matches!(&l[0], LispVal::Sym(s) if s == "lambda")
                                            );
                                            if is_lambda {
                                                self.pending_lambda_name = Some(name.clone());
                                            }
                                            if !self.compile_expr(&pair[1], outer_env) {
                                                self.pending_lambda_name = None;
                                                all_ok = false;
                                                break;
                                            }
                                            self.pending_lambda_name = None;
                                            let val_is_i64 = self.last_result_i64;
                                            let val_is_f64 = self.last_result_f64;
                                            // Check if this name already has a slot (shadowing)
                                            if let Some(existing) =
                                                self.slot_map.iter().position(|s| s == name)
                                            {
                                                // Save old value to a temporary slot
                                                let save_slot = save_base + shadow_idx;
                                                self.code.push(Op::LoadSlot(existing)); // push old value
                                                self.code.push(Op::StoreSlot(save_slot)); // save it
                                                // Now store the new value
                                                self.code.push(Op::StoreSlot(existing));
                                                shadowed.push(existing);
                                                shadow_idx += 1;
                                                if val_is_i64 {
                                                    self.mark_slot_i64(existing);
                                                }
                                                if val_is_f64 {
                                                    self.mark_slot_f64(existing);
                                                }
                                            } else {
                                                let slot_idx = self.slot_map.len();
                                                self.slot_map.push(name.clone());
                                                self.code.push(Op::StoreSlot(slot_idx));
                                                if val_is_i64 {
                                                    self.mark_slot_i64(slot_idx);
                                                }
                                                if val_is_f64 {
                                                    self.mark_slot_f64(slot_idx);
                                                }
                                            }
                                        } else {
                                            all_ok = false;
                                            break;
                                        }
                                    }
                                    _ => {
                                        all_ok = false;
                                        break;
                                    }
                                }
                            }
                            if all_ok {
                                all_ok = self.compile_expr(body, outer_env);
                                if !all_ok {
                                }
                            }
                            // Restore shadowed slots (reverse order)
                            for (i, &original_slot) in shadowed.iter().enumerate().rev() {
                                let save_slot = save_base + i;
                                self.code.push(Op::LoadSlot(save_slot));
                                self.code.push(Op::StoreSlot(original_slot));
                            }
                            // Remove any newly added slot names (not shadows)
                            self.slot_map.truncate(let_start);
                            all_ok
                        }
                        // letrec: (letrec ((f (lambda ...)) (g (lambda ...))) body...)
                        // All bindings are visible to all init expressions.
                        // For lambda bindings, set pending_lambda_name so CallSelf is used
                        // for self-referencing recursive calls (avoids stale capture problem).
                        "letrec" => {
                            if list.len() < 3 {
                                return false;
                            }
                            let bindings = match &list[1] {
                                LispVal::List(b) => b,
                                _ => return false,
                            };
                            let let_start = self.slot_map.len();
                            let mut var_slots: Vec<(String, usize)> = Vec::new();
                            for binding in bindings.iter() {
                                let (name, _) = match binding {
                                    LispVal::List(pair) if pair.len() == 2 => match &pair[0] {
                                        LispVal::Sym(n) => (n.clone(), &pair[1]),
                                        _ => return false,
                                    },
                                    _ => return false,
                                };
                                let slot = self.slot_map.len();
                                self.slot_map.push(name.clone());
                                var_slots.push((name, slot));
                            }
                            // Initialize all slots to Nil
                            for (_, slot) in &var_slots {
                                self.code.push(Op::PushNil);
                                self.code.push(Op::StoreSlot(*slot));
                            }
                            // Compile each init expression with all vars visible
                            let mut all_ok = true;
                            for binding in bindings.iter() {
                                if let LispVal::List(pair) = binding {
                                    let sym_name = match &pair[0] {
                                        LispVal::Sym(n) => n,
                                        _ => {
                                            all_ok = false;
                                            break;
                                        }
                                    };
                                    // If init is a lambda, set pending_lambda_name for CallSelf
                                    let is_lambda_init = matches!(&pair[1],
                                        LispVal::List(ref l) if !l.is_empty() &&
                                        matches!(&l[0], LispVal::Sym(ref s) if s == "lambda" || s == "fn")
                                    );
                                    if is_lambda_init {
                                        self.pending_lambda_name = Some(sym_name.clone());
                                    }
                                    if !self.compile_expr(&pair[1], outer_env) {
                                        self.pending_lambda_name = None;
                                        all_ok = false;
                                        break;
                                    }
                                    self.pending_lambda_name = None;
                                    let slot = self.slot_map.iter().position(|s| s == sym_name).unwrap();
                                    self.code.push(Op::StoreSlot(slot));
                                }
                            }
                            if !all_ok {
                                self.slot_map.truncate(let_start);
                                return false;
                            }
                            // Compile body
                            for expr in &list[2..] {
                                if !self.compile_expr(expr, outer_env) {
                                    self.slot_map.truncate(let_start);
                                    return false;
                                }
                            }
                            self.slot_map.truncate(let_start);
                            true
                        }
                        // pure: (pure (define (name ...) body)) → compile define, then annotate
                        "pure" => {
                            if list.len() != 2 { return false; }
                            let inner = &list[1];
                            // Compile the inner expression normally (e.g., a define)
                            if !self.compile_expr(inner, outer_env) {
                                return false;
                            }
                            // Extract function name from (define (name ...) body) or (define name ...)
                            if let LispVal::List(ref def_form) = inner {
                                if def_form.len() >= 3 {
                                    if let LispVal::Sym(ref name) = def_form[1] {
                                        // Push the name as a string, then call mark-pure
                                        self.code.push(Op::PushLiteral(LispVal::Str(name.clone())));
                                        self.code.push(Op::BuiltinCall("mark-pure".to_string(), 1));
                                        self.last_result_i64 = false;
                                    } else if let LispVal::List(ref sig) = def_form[1] {
                                        // (define (name params...) body...) — extract name from sig
                                        if let Some(LispVal::Sym(ref name)) = sig.first() {
                                            self.code.push(Op::PushLiteral(LispVal::Str(name.clone())));
                                            self.code.push(Op::BuiltinCall("mark-pure".to_string(), 1));
                                            self.last_result_i64 = false;
                                        }
                                    }
                                }
                            }
                            true
                        }
                        // pure-type: (pure-type name) → look up pure type annotation by name
                        // Compiler special form so the arg is NOT evaluated — we need the symbol name.
                        "pure-type" => {
                            if list.len() != 2 { return false; }
                            // Push the arg as a literal symbol (not evaluated)
                            self.code.push(Op::PushLiteral(list[1].clone()));
                            self.code.push(Op::BuiltinCall("pure-type".to_string(), 1));
                            self.last_result_i64 = false;
                            true
                        }
                        // fork: (fork expr) → evaluate expr in isolated env, return result, discard changes
                        "fork" => {
                            if list.len() != 2 { return false; }
                            // Wrap body in a thunk (like delay)
                            let thunk_form = LispVal::List(vec![
                                LispVal::Sym("lambda".into()),
                                LispVal::List(vec![]), // 0 params
                                list[1].clone(),
                            ]);
                            if !self.compile_expr(&thunk_form, outer_env) {
                                return false;
                            }
                            // fork-exec receives the thunk, snapshots env, runs thunk, restores
                            self.code.push(Op::BuiltinCall("fork-exec".to_string(), 1));
                            self.last_result_i64 = false;
                            true
                        }
                        // delay: (delay expr) → creates a promise (lazy evaluation)
                        "delay" => {
                            if list.len() != 2 { return false; }
                            // Compile the expression as a 0-param lambda (thunk)
                            let thunk_form = LispVal::List(vec![
                                LispVal::Sym("lambda".into()),
                                LispVal::List(vec![]),  // 0 params
                                list[1].clone(),         // body expression
                            ]);
                            if !self.compile_expr(&thunk_form, outer_env) {
                                return false;
                            }
                            // Wrap in Delay at runtime via builtin
                            self.code.push(Op::BuiltinCall("make-promise".to_string(), 1));
                            self.last_result_i64 = false;
                            true
                        }
                        // try: (try expr (catch var handler...))
                        "try" => {
                            // Parse: (try expr (catch var handler-body...))
                            if list.len() < 3 { return false; }
                            let try_expr = &list[1];
                            // Parse catch clause: (catch var handler...)
                            let catch_clause = match &list[2] {
                                LispVal::List(clause) if clause.len() >= 3 => {
                                    match &clause[0] {
                                        LispVal::Sym(s) if s == "catch" => &clause[1..],
                                        _ => return false,
                                    }
                                }
                                _ => return false,
                            };
                            let catch_var = match catch_clause.get(0) {
                                Some(LispVal::Sym(s)) => s.clone(),
                                _ => return false,
                            };
                            let catch_body = &catch_clause[1..];

                            // Build try-catch-lambda call:
                            // (try-catch-impl (lambda () expr) (lambda (var) handler...))
                            let try_lambda = LispVal::List(vec![
                                LispVal::Sym("lambda".into()),
                                LispVal::List(vec![]),
                                try_expr.clone(),
                            ]);
                            let catch_body_form = if catch_body.len() == 1 {
                                catch_body[0].clone()
                            } else {
                                LispVal::List(
                                    std::iter::once(LispVal::Sym("begin".into()))
                                        .chain(catch_body.iter().cloned())
                                        .collect(),
                                )
                            };
                            let catch_lambda = LispVal::List(vec![
                                LispVal::Sym("lambda".into()),
                                LispVal::List(vec![LispVal::Sym(catch_var)]),
                                catch_body_form,
                            ]);
                            let call_form = LispVal::List(vec![
                                LispVal::Sym("try-catch-impl".into()),
                                try_lambda,
                                catch_lambda,
                            ]);
                            // Recursively compile the generated call
                            self.compile_expr(&call_form, outer_env)
                        }
                        // case-lambda: (case-lambda (params body...) ... rest-catch-clause)
                        "case-lambda" => {
                            if list.len() < 2 { return false; }
                            let n_clauses = list.len() - 1;
                            // Compile each clause as a separate lambda closure
                            // Each clause: (params body...) → (lambda (params) body...)
                            for clause in &list[1..] {
                                match clause {
                                    LispVal::List(clause_parts) if clause_parts.len() >= 2 => {
                                        match &clause_parts[0] {
                                            // Single symbol = rest-param catch-all: (args body...)
                                            LispVal::Sym(rest_name) => {
                                                let body = if clause_parts.len() == 2 {
                                                    clause_parts[1].clone()
                                                } else {
                                                    LispVal::List(
                                                        std::iter::once(LispVal::Sym("begin".into()))
                                                            .chain(clause_parts[1..].iter().cloned())
                                                            .collect(),
                                                    )
                                                };
                                                // Compile as (lambda (&rest rest_name) body)
                                                let lambda_form = LispVal::List(vec![
                                                    LispVal::Sym("lambda".into()),
                                                    LispVal::List(vec![
                                                        LispVal::Sym("&rest".into()),
                                                        LispVal::Sym(rest_name.clone()),
                                                    ]),
                                                    body,
                                                ]);
                                                if !self.compile_expr(&lambda_form, outer_env) {
                                                    return false;
                                                }
                                            }
                                            // List of params: ((a b) body...)
                                            LispVal::List(param_list) => {
                                                let body = if clause_parts.len() == 2 {
                                                    clause_parts[1].clone()
                                                } else {
                                                    LispVal::List(
                                                        std::iter::once(LispVal::Sym("begin".into()))
                                                            .chain(clause_parts[1..].iter().cloned())
                                                            .collect(),
                                                    )
                                                };
                                                let lambda_form = LispVal::List(vec![
                                                    LispVal::Sym("lambda".into()),
                                                    LispVal::List(param_list.clone()),
                                                    body,
                                                ]);
                                                if !self.compile_expr(&lambda_form, outer_env) {
                                                    return false;
                                                }
                                            }
                                            _ => return false,
                                        }
                                    }
                                    _ => return false,
                                }
                            }
                            // Now n_clauses compiled lambdas are on the stack.
                            // make-case-lambda builtin pops them and creates a CaseLambda.
                            self.code.push(Op::BuiltinCall("make-case-lambda".to_string(), n_clauses));
                            self.last_result_i64 = false;
                            true
                        }
                        // when: (when test body...) → if test (begin body...)
                        "when" => {
                            if list.len() < 3 {
                                return false;
                            }
                            let test = &list[1];
                            if !self.compile_expr(test, outer_env) {
                                return false;
                            }
                            let jf_idx = self.code.len();
                            self.code.push(Op::JumpIfFalse(0));
                            // Compile body as implicit begin
                            for (i, arg) in list[2..].iter().enumerate() {
                                if !self.compile_expr(arg, outer_env) {
                                    return false;
                                }
                                if i + 1 < list.len() - 2 {
                                    self.code.push(Op::Pop);
                                }
                            }
                            let jmp_idx = self.code.len();
                            self.code.push(Op::Jump(0));
                            let else_start = self.code.len();
                            self.code[jf_idx] = Op::JumpIfFalse(else_start);
                            self.code.push(Op::PushNil);
                            self.code[jmp_idx] = Op::Jump(self.code.len());
                            true
                        }
                        // unless: (unless test body...) → if (not test) (begin body...)
                        "unless" => {
                            if list.len() < 3 {
                                return false;
                            }
                            let test = &list[1];
                            if !self.compile_expr(test, outer_env) {
                                return false;
                            }
                            let jt_idx = self.code.len();
                            self.code.push(Op::JumpIfTrue(0));
                            // Compile body as implicit begin
                            for (i, arg) in list[2..].iter().enumerate() {
                                if !self.compile_expr(arg, outer_env) {
                                    return false;
                                }
                                if i + 1 < list.len() - 2 {
                                    self.code.push(Op::Pop);
                                }
                            }
                            let jmp_idx = self.code.len();
                            self.code.push(Op::Jump(0));
                            let else_start = self.code.len();
                            self.code[jt_idx] = Op::JumpIfTrue(else_start);
                            self.code.push(Op::PushNil);
                            self.code[jmp_idx] = Op::Jump(self.code.len());
                            true
                        }
                        // loop: (loop ((var init) ...) body) with (recur val ...) inside body
                        "loop" => {
                            let bindings = match list.get(1) {
                                Some(LispVal::List(b)) => b,
                                _ => return false,
                            };
                            let body = match list.get(2) {
                                Some(b) => b,
                                None => return false,
                            };
                            // Parse bindings: ((var1 init1) (var2 init2) ...) or flat (var1 init1 var2 init2 ...)
                            let mut var_slots: Vec<usize> = Vec::new();
                            // Check if this is flat syntax: first element is a Sym, not a List
                            let is_flat = bindings.first().map_or(false, |b| matches!(b, LispVal::Sym(_)));
                            if is_flat {
                                // Flat: (var1 init1 var2 init2 ...)
                                if bindings.len() % 2 != 0 {
                                    return false;
                                }
                                for chunk in bindings.chunks(2) {
                                    if let LispVal::Sym(name) = &chunk[0] {
                                        let slot = self.slot_map.len();
                                        self.slot_map.push(name.clone());
                                        var_slots.push(slot);
                                        if !self.compile_expr(&chunk[1], outer_env) {
                                            return false;
                                        }
                                        self.code.push(Op::StoreSlot(slot));
                                    } else {
                                        return false;
                                    }
                                }
                            } else {
                                // Pair: ((var1 init1) (var2 init2) ...)
                                for binding in bindings.iter() {
                                    if let LispVal::List(pair) = binding {
                                        if pair.len() == 2 {
                                            if let LispVal::Sym(name) = &pair[0] {
                                                let slot = self.slot_map.len();
                                                self.slot_map.push(name.clone());
                                                var_slots.push(slot);
                                                // If init is a lambda, set pending_lambda_name
                                                // so the inner compiler enables CallSelf for recursion.
                                                // This supports letrec-style self-reference:
                                                // (let ((f (lambda ...))) ... (f ...))
                                                let is_lambda_init = matches!(&pair[1],
                                                    LispVal::List(ref l) if !l.is_empty() &&
                                                    matches!(&l[0], LispVal::Sym(ref s) if s == "lambda" || s == "fn")
                                                );
                                                if is_lambda_init {
                                                    self.pending_lambda_name = Some(name.clone());
                                                }
                                                if !self.compile_expr(&pair[1], outer_env) {
                                                    self.pending_lambda_name = None;
                                                    return false;
                                                }
                                                self.pending_lambda_name = None;
                                                self.code.push(Op::StoreSlot(slot));
                                                continue;
                                            }
                                        }
                                    }
                                    return false;
                                }
                            }
                            let loop_start = self.code.len();
                            self.loop_stack.push((loop_start, var_slots));
                            if !self.compile_expr(body, outer_env) {
                                self.loop_stack.pop();
                                return false;
                            }
                            self.loop_stack.pop();
                            true
                        }
                        "set!" => {
                            if list.len() != 3 {
                                return false;
                            }
                            let name = match &list[1] {
                                LispVal::Sym(s) => s.clone(),
                                _ => return false,
                            };
                            // Check if it's a local slot (param/let)
                            if let Some(slot) = self.slot_of(&name) {
                                if !self.compile_expr(&list[2], outer_env) {
                                    return false;
                                }
                                self.code.push(Op::StoreSlot(slot));
                                self.code.push(Op::LoadSlot(slot)); // set! returns the new value
                                true
                            } else if let Some(idx) = self.captured_idx(&name) {
                                if self.forward_captures.contains(&name) {
                                    // Env capture — use StoreGlobal so other closures see the mutation
                                    // Compile RHS BEFORE inserting into set_target_globals,
                                    // so the RHS reads the captured value (LoadCaptured), not env (LoadGlobal).
                                    if !self.compile_expr(&list[2], outer_env) {
                                        return false;
                                    }
                                    self.set_target_globals.insert(name.clone());
                                    self.code.push(Op::StoreGlobal(name.clone()));
                                    self.code.push(Op::LoadGlobal(name));
                                } else {
                                    // Runtime capture (let/param in enclosing scope) —
                                    // use StoreCaptured to mutate the closure's captured snapshot
                                    if !self.compile_expr(&list[2], outer_env) {
                                        return false;
                                    }
                                    self.code.push(Op::StoreCaptured(idx));
                                    self.code.push(Op::LoadCaptured(idx));
                                }
                                true
                            } else if self.try_capture(&name, outer_env) {
                                // Variable was a runtime capture from parent slots.
                                // Now captured_idx should return Some. Re-check forward_captures.
                                let idx = self.captured_idx(&name).unwrap();
                                if self.forward_captures.contains(&name) {
                                    self.set_target_globals.insert(name.clone());
                                    if !self.compile_expr(&list[2], outer_env) {
                                        return false;
                                    }
                                    self.code.push(Op::StoreGlobal(name.clone()));
                                    // StoreGlobal already pushes the value back, no need for LoadGlobal
                                } else {
                                    if !self.compile_expr(&list[2], outer_env) {
                                        return false;
                                    }
                                    self.code.push(Op::StoreCaptured(idx));
                                    // StoreCaptured already pushes the value back
                                }
                                true
                            } else {
                                // Truly unknown variable — top-level define or undefined.
                                // Emit StoreGlobal/LoadGlobal as fallback.
                                self.set_target_globals.insert(name.clone());
                                if !self.compile_expr(&list[2], outer_env) {
                                    return false;
                                }
                                self.code.push(Op::StoreGlobal(name.clone()));
                                // StoreGlobal already pushes the value back, no need for LoadGlobal
                                true
                            }
                        }
                        "lambda" | "fn" => {
                            // (lambda (params...) body...) or (lambda (a b &rest rest) body...)
                            if list.len() < 3 {
                                return false;
                            }
                            // Parse params, detecting &rest
                            let (fixed_params, rest_param) = match list.get(1) {
                                Some(LispVal::List(ps)) => {
                                    let mut fixed = Vec::new();
                                    let mut rest = None;
                                    let mut seen_rest = false;
                                    for p in ps.iter() {
                                        if let LispVal::Sym(s) = p {
                                            if s == "&rest" {
                                                seen_rest = true;
                                            } else if seen_rest {
                                                rest = Some(s.clone());
                                            } else {
                                                fixed.push(s.clone());
                                            }
                                        }
                                    }
                                    (fixed, rest)
                                }
                                Some(LispVal::Sym(s)) => (vec![s.clone()], None),
                                _ => return false,
                            };
                            let params: Vec<String> = if let Some(ref rp) = rest_param {
                                let mut p = fixed_params.clone();
                                p.push(rp.clone());
                                p
                            } else {
                                fixed_params.clone()
                            };
                            let n_fixed = fixed_params.len();
                            // Compile lambda body in a new compiler
                            let mut inner = LoopCompiler::new(params.clone());
                            inner.parent_slots = self.slot_map.clone();
                            // If this lambda is the value of a let-binding, enable self_name
                            // for recursive calls (e.g., (define fib (lambda ...)) → (let fib (lambda ...) ...))
                            if self.pending_lambda_name.is_some() {
                                inner.self_name = self.pending_lambda_name.clone();
                            }
                            let body = &list[2..];
                            let mut ok = true;
                            for (bi, expr) in body.iter().enumerate() {
                                if !inner.compile_expr(expr, outer_env) {
                                    ok = false;
                                    break;
                                }
                            }
                            if ok {
                            }
                            if !ok {
                                return false;
                            }
                            inner.code.push(Op::Return);
                            // Compute total_slots
                            let base = params.len();
                            let mut max_slot = base;
                            for op in &inner.code {
                                match op {
                                    Op::LoadSlot(s) | Op::StoreSlot(s) => {
                                        if *s >= max_slot {
                                            max_slot = *s + 1;
                                        }
                                    }
                                    Op::SlotAddImm(s, _) | Op::SlotMulImm(s, _) => {
                                        if *s >= max_slot {
                                            max_slot = *s + 1;
                                        }
                                    }
                                    Op::CallCaptured(s, _) => {
                                        if *s >= max_slot {
                                            max_slot = *s + 1;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            let idx = self.closures.len();
                            self.closures.push(CompiledLambda {
                                name: inner.self_name.clone(),
                                num_param_slots: params.len(),
                                total_slots: max_slot,
                                code: inner.code,
                                captured: std::sync::RwLock::new(inner.captured.clone()),
                                closures: inner.closures,
                                runtime_captures: inner.runtime_captures,
                                rest_param_idx: rest_param.as_ref().map(|_| n_fixed),
                                num_fixed_params: n_fixed,
                            });
                            self.code.push(Op::PushClosure(idx));
                            true
                        }
                        _ => {
                            // Function call: captured var, self-call, inline op, or assumed builtin
                            let n_args = list.len() - 1;
                            // Check if it's a macro — expand at compile time before compiling args
                            if let Some(macro_val) = outer_env.get(op) {
                                if matches!(macro_val, LispVal::Macro { .. }) {
                                    match expand_macro_call(macro_val, &list[1..]) {
                                        Ok(expansion) => {
                                            return self.compile_expr(&expansion, outer_env);
                                        }
                                        Err(_) => return false,
                                    }
                                }
                            }
                            // If callee is an unresolvable symbol (not a slot, not captured,
                            // not a builtin), compile the whole form as a list literal.
                            // This handles type descriptors like (:fn :int → :int) and
                            // other data-as-code patterns.
                            if self.slot_of(op).is_none()
                                && self.captured_idx(op).is_none()
                                && !self.try_capture(op, outer_env)
                                && !crate::helpers::is_builtin_name(op)
                                && crate::helpers::lookup_constructor(op).is_none()
                                && self.self_name.as_deref() != Some(op)
                            {
                                // Compile each element, then build a list
                                // But we need the UNEVALUATED forms as data, not evaluated values.
                                // Use PushLiteral for symbols/atoms, compile_expr for sub-lists.
                                for elem in list.iter() {
                                    match elem {
                                        LispVal::Sym(s) => {
                                            self.code.push(Op::PushLiteral(LispVal::Sym(s.clone())));
                                        }
                                        LispVal::Num(n) => {
                                            self.code.push(Op::PushI64(*n));
                                        }
                                        LispVal::Str(s) => {
                                            self.code.push(Op::PushLiteral(LispVal::Str(s.clone())));
                                        }
                                        LispVal::Bool(b) => {
                                            self.code.push(Op::PushLiteral(LispVal::Bool(*b)));
                                        }
                                        other => {
                                            // For sub-lists and other complex forms, push as literal
                                            self.code.push(Op::PushLiteral(other.clone()));
                                        }
                                    }
                                }
                                self.code.push(Op::BuiltinCall("list".to_string(), list.len()));
                                return true;
                            }
                            // Check for inline dict ops first
                            if op == "dict/get" || op == "dict-ref" {
                                if n_args == 2 {
                                    for arg in &list[1..] {
                                        if !self.compile_expr(arg, outer_env) {
                                            return false;
                                        }
                                    }
                                    self.code.push(Op::DictGet);
                                    return true;
                                }
                            } else if op == "dict/set" || op == "dict-set" {
                                if n_args == 3 {
                                    // Check if first arg is a loop var — emit DictMutSet for in-place mutation
                                    let mut dict_mut_slot: Option<usize> = None;
                                    if let Some(LispVal::Sym(name)) = list.get(1) {
                                        if let Some((_, ref var_slots)) = self.loop_stack.last() {
                                            if let Some(slot) = self.slot_of(name) {
                                                if var_slots.contains(&slot) {
                                                    dict_mut_slot = Some(slot);
                                                }
                                            }
                                        }
                                    }
                                    if let Some(slot) = dict_mut_slot {
                                        // Compile key and val (skip the map arg — it's in the slot)
                                        if !self.compile_expr(&list[2], outer_env) {
                                            return false;
                                        }
                                        if !self.compile_expr(&list[3], outer_env) {
                                            return false;
                                        }
                                        self.code.push(Op::DictMutSet(slot));
                                        return true;
                                    }
                                    for arg in &list[1..] {
                                        if !self.compile_expr(arg, outer_env) {
                                            return false;
                                        }
                                    }
                                    self.code.push(Op::DictSet);
                                    return true;
                                }
                            }
                            // Fast path: (list e1 e2 ...) → MakeList(n)
                            if op == "list" {
                                for arg in &list[1..] {
                                    if !self.compile_expr(arg, outer_env) {
                                        return false;
                                    }
                                }
                                self.code.push(Op::MakeList(n_args));
                                return true;
                            }
                            // Y combinator: (me me args...) — self-passing pattern
                            // If callee is a local slot AND first arg is the same symbol,
                            // skip the self-pass arg and compile only real args, then CallSelf.
                            if n_args > 0 {
                                if let Some(LispVal::Sym(first_arg)) = list.get(1) {
                                    if first_arg == op && self.slot_of(op).is_some() {
                                        let has_self_name = self.self_name.as_deref() == Some(op.as_str());
                                        if !has_self_name {
                                            // Push self as first arg (the "me" parameter)
                                            self.code.push(Op::PushSelf);
                                            // Compile only real args (list[2..])
                                            for real_arg in &list[2..] {
                                                if !self.compile_expr(real_arg, outer_env) {
                                                    return false;
                                                }
                                            }
                                            self.code.push(Op::CallSelf(n_args)); // n_args = original count including self-pass
                                            return true;
                                        }
                                    }
                                }
                            }
                            for arg in &list[1..] {
                                if !self.compile_expr(arg, outer_env) {
                                    return false;
                                }
                            }
                            if let Some(ref sn) = self.self_name {
                                if op == sn {
                                    self.code.push(Op::CallSelf(n_args));
                                    return true;
                                }
                            }
                            if let Some(idx) = self.captured_idx(op) {
                                // Check if forward-referenced capture — use LoadGlobal + CallDynamic
                                if self.forward_captures.contains(&op.to_string()) {
                                    self.code.push(Op::LoadGlobal(op.to_string()));
                                    self.code.push(Op::CallDynamic(n_args));
                                    return true;
                                }
                                // Try const fold (pure + all-const args → eval at compile time)
                                // Then try inline (small compiled lambda → paste body)
                                // Otherwise emit call
                                if !self.try_const_fold(idx, n_args)
                                    && !self.try_inline_call(idx, n_args)
                                {
                                    self.code.push(Op::CallCapturedRef(idx, n_args));
                                }
                            } else if let Some(slot) = self.slot_of(op) {
                                self.code.push(Op::CallCaptured(slot, n_args));
                            } else if self.try_capture(op, outer_env) {
                                let idx = self.captured_idx(op).unwrap();
                                // Check if just-captured is a forward reference
                                if self.forward_captures.contains(&op.to_string()) {
                                    self.code.push(Op::LoadGlobal(op.to_string()));
                                    self.code.push(Op::CallDynamic(n_args));
                                    return true;
                                }
                                if !self.try_const_fold(idx, n_args)
                                    && !self.try_inline_call(idx, n_args)
                                {
                                    self.code.push(Op::CallCapturedRef(idx, n_args));
                                }
                            } else {
                                // Check if it's a registered type constructor (deftype)
                                if let Some(ctor) = crate::helpers::lookup_constructor(op) {
                                    if n_args == ctor.n_fields as usize {
                                        // Args already compiled at line 1157 — just emit ConstructTag
                                        self.code.push(Op::ConstructTag(
                                            ctor.type_name.clone(),
                                            ctor.variant_id,
                                            ctor.n_fields,
                                        ));
                                        return true;
                                    }
                                }
                                self.code.push(Op::BuiltinCall(op.clone(), n_args));
                            }
                            true
                        }
                    }
                } else if let LispVal::List(ref callee) = list[0] {
                    // Computed function call: ((expr) args...)
                    // CallDynamic expects: [..., arg1, ..., argN, func]
                    // So compile args first, then callee.
                    let n_args = list.len() - 1;
                    // Compile arguments first
                    for arg in &list[1..] {
                        if !self.compile_expr(arg, outer_env) {
                            return false;
                        }
                    }
                    // Compile callee (pushes function on stack top)
                    if !self.compile_expr(&list[0], outer_env) {
                        return false;
                    }
                    // Stack: [arg1, ..., argN, func]
                    self.code.push(Op::CallDynamic(n_args));
                    return true;
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Compile a pattern match check against a value in `value_slot`.
    /// Pushes Bool(true) on stack if pattern matches, Bool(false) otherwise.
    /// For binding patterns (?x, nested destructuring), also allocates slots
    /// and emits code to extract values into those slots.
    /// Adds binding names to self.slot_map as needed.
    fn compile_pattern_check(
        &mut self,
        pattern: &LispVal,
        value_slot: usize,
        _outer_env: &Env,
    ) -> bool {
        match pattern {
            // Variable binding: ?x — matches anything, binds x to value
            LispVal::Sym(name) if name.starts_with('?') => {
                let bind_name = &name[1..]; // strip '?' prefix
                let slot_idx = self.slot_map.len();
                self.slot_map.push(bind_name.to_string());
                self.code.push(Op::LoadSlot(value_slot));
                self.code.push(Op::StoreSlot(slot_idx));
                // Always matches
                self.code.push(Op::PushBool(true));
                true
            }
            // Wildcard: _ — matches anything, no binding
            LispVal::Sym(name) if name == "_" => {
                self.code.push(Op::PushBool(true));
                true
            }
            // Plain symbol binding (used in nested destructuring): a, b, c, etc.
            // Treat as variable binding (same as ?x but without ? prefix)
            LispVal::Sym(name) => {
                // Skip special keywords that shouldn't be bindings
                if name == "true" || name == "false" || name == "nil" || name == "else" {
                    // Treat as literal
                    match name.as_str() {
                        "true" => {
                            self.code.push(Op::LoadSlot(value_slot));
                            self.code.push(Op::PushBool(true));
                            self.code.push(Op::Eq);
                        }
                        "false" => {
                            self.code.push(Op::LoadSlot(value_slot));
                            self.code.push(Op::PushBool(false));
                            self.code.push(Op::Eq);
                        }
                        "nil" => {
                            self.code.push(Op::LoadSlot(value_slot));
                            self.code.push(Op::PushNil);
                            self.code.push(Op::Eq);
                        }
                        _ => return false,
                    }
                    return true;
                }
                let slot_idx = self.slot_map.len();
                self.slot_map.push(name.clone());
                self.code.push(Op::LoadSlot(value_slot));
                self.code.push(Op::StoreSlot(slot_idx));
                // Always matches
                self.code.push(Op::PushBool(true));
                true
            }
            // Literal patterns: numbers, bools, strings, nil
            LispVal::Num(n) => {
                self.code.push(Op::LoadSlot(value_slot));
                self.code.push(Op::PushI64(*n));
                self.code.push(Op::Eq);
                true
            }
            LispVal::Float(f) => {
                self.code.push(Op::LoadSlot(value_slot));
                self.code.push(Op::PushFloat(*f));
                self.code.push(Op::Eq);
                true
            }
            LispVal::Bool(b) => {
                self.code.push(Op::LoadSlot(value_slot));
                self.code.push(Op::PushBool(*b));
                self.code.push(Op::Eq);
                true
            }
            LispVal::Str(s) => {
                self.code.push(Op::LoadSlot(value_slot));
                self.code.push(Op::PushStr(s.clone()));
                self.code.push(Op::Eq);
                true
            }
            LispVal::Nil => {
                self.code.push(Op::LoadSlot(value_slot));
                self.code.push(Op::PushNil);
                self.code.push(Op::Eq);
                true
            }
            // List patterns: (cons ...), (list ...), or nested destructuring
            LispVal::List(pat_list) if !pat_list.is_empty() => {
                let is_cons = matches!(
                    &pat_list[0],
                    LispVal::Sym(op) if op == "cons" && pat_list.len() == 3
                );
                let is_list = matches!(
                    &pat_list[0],
                    LispVal::Sym(op) if op == "list"
                );

                if is_cons {
                    self.compile_cons_pattern(pat_list, value_slot, _outer_env)
                } else if is_list {
                    self.compile_list_pattern(pat_list, value_slot, _outer_env)
                } else {
                    // Nested destructuring: (pat1 pat2 ...) — match list by length
                    self.compile_destructure_pattern(pat_list, value_slot, _outer_env)
                }
            }
            _ => false,
        }
    }
    /// Compile (cons ?h ?t) pattern: non-empty list, bind head and tail
    fn compile_cons_pattern(
        &mut self,
        pat_list: &[LispVal],
        value_slot: usize,
        outer_env: &Env,
    ) -> bool {
        // Check non-empty: (= (length scrutinee) 0) → if empty, fail
        self.code.push(Op::LoadSlot(value_slot));
        self.code.push(Op::BuiltinCall("length".to_string(), 1));
        self.code.push(Op::PushI64(0));
        self.code.push(Op::Eq);
        let nil_check_idx = self.code.len();
        self.code.push(Op::JumpIfFalse(0)); // if NOT empty (length > 0), continue
        self.code.push(Op::PushBool(false)); // is nil → no match
        let nil_end_idx = self.code.len();
        self.code.push(Op::Jump(0)); // skip to end
        self.code[nil_check_idx] = Op::JumpIfFalse(self.code.len()); // not nil → continue

        // Store head in temp slot
        let head_slot = self.slot_map.len();
        self.slot_map.push("__cons_head".to_string());
        self.code.push(Op::LoadSlot(value_slot));
        self.code.push(Op::BuiltinCall("car".to_string(), 1));
        self.code.push(Op::StoreSlot(head_slot));

        // Store tail in temp slot
        let tail_slot = self.slot_map.len();
        self.slot_map.push("__cons_tail".to_string());
        self.code.push(Op::LoadSlot(value_slot));
        self.code.push(Op::BuiltinCall("cdr".to_string(), 1));
        self.code.push(Op::StoreSlot(tail_slot));

        // Compile head sub-pattern against head_slot
        if !self.compile_pattern_check(&pat_list[1], head_slot, outer_env) {
            return false;
        }
        // head check result is on stack (Bool)
        let head_fail_idx = self.code.len();
        self.code.push(Op::JumpIfFalse(0)); // if head doesn't match, fail

        // Compile tail sub-pattern against tail_slot
        if !self.compile_pattern_check(&pat_list[2], tail_slot, outer_env) {
            return false;
        }
        // tail check result is on stack (Bool) — this is the final result for the success path

        // Success path ends here with the tail check result on stack.
        // Jump past the failure block.
        let success_end = self.code.len();
        self.code.push(Op::Jump(0)); // skip over failure blocks

        // Failure block: head failed → push false
        self.code[head_fail_idx] = Op::JumpIfFalse(self.code.len());
        self.code.push(Op::PushBool(false));

        // Patch nil fail jump to go here (end)
        self.code[nil_end_idx] = Op::Jump(self.code.len());
        // Patch success jump to skip failure block
        self.code[success_end] = Op::Jump(self.code.len());

        // Don't truncate slots here — binding slots added by sub-patterns (e.g., ?h, ?t)
        // need to remain for the match body to access. The match handler will clean up
        // all slots after the body (or on pattern failure, before the next clause).

        true
    }
    /// Compile (list p1 p2 ...) pattern: match by length + element patterns
    fn compile_list_pattern(
        &mut self,
        pat_list: &[LispVal],
        value_slot: usize,
        outer_env: &Env,
    ) -> bool {
        let expected_len = pat_list.len() - 1;

        // Check length: (eq (length scrutinee) expected_len)
        self.code.push(Op::LoadSlot(value_slot));
        self.code.push(Op::BuiltinCall("length".to_string(), 1));
        self.code.push(Op::PushI64(expected_len as i64));
        self.code.push(Op::Eq);
        let len_fail_idx = self.code.len();
        self.code.push(Op::JumpIfFalse(0)); // if length doesn't match, fail

        // Extract each element: (nth scrutinee i) for each i
        let _elem_slots_start = self.slot_map.len();
        let mut elem_slots: Vec<usize> = Vec::new();
        for i in 0..expected_len {
            let slot = self.slot_map.len();
            self.slot_map.push(format!("__list_elem_{}", i));
            elem_slots.push(slot);
            self.code.push(Op::LoadSlot(value_slot));
            self.code.push(Op::PushI64(i as i64));
            self.code.push(Op::BuiltinCall("nth".to_string(), 2));
            self.code.push(Op::StoreSlot(slot));
        }

        // Check each element pattern
        // On any element failure, jump to the end and push false.
        // Collect jump indices for non-last elements to patch later.
        let mut fail_jump_indices: Vec<usize> = Vec::new();
        for (i, sub_pat) in pat_list[1..].iter().enumerate() {
            if !self.compile_pattern_check(sub_pat, elem_slots[i], outer_env) {
                return false;
            }
            // Element check pushes bool on stack.
            if i + 1 < expected_len {
                // Not the last element: if check fails, jump to fail block
                let elem_fail_idx = self.code.len();
                self.code.push(Op::JumpIfFalse(0));
                fail_jump_indices.push(elem_fail_idx);
                // If element matched, JumpIfFalse doesn't jump, bool is popped.
                // Continue to next element. Stack is empty.
            }
            // If last element, leave its bool on stack as final result.
        }

        // All elements matched — last element's bool is on stack. This is the success path.
        // Jump past the failure block.
        let success_end = self.code.len();
        self.code.push(Op::Jump(0)); // skip over failure block
        let fail_block = self.code.len();

        // Failure block: push false
        self.code.push(Op::PushBool(false));

        // Patch success jump to skip failure block
        self.code[success_end] = Op::Jump(self.code.len());

        // Patch all non-last element fail jumps to jump to the failure block
        for &fail_idx in &fail_jump_indices {
            self.code[fail_idx] = Op::JumpIfFalse(fail_block);
        }

        // Patch length fail jump to jump to the failure block
        self.code[len_fail_idx] = Op::JumpIfFalse(fail_block);

        // Don't truncate slots here — binding slots added by sub-patterns need to remain
        // for the match body to access. The match handler will clean up all slots after
        // the body (or on pattern failure, before the next clause).

        true
    }

    /// Compile nested destructuring pattern: (pat1 pat2 ...)
    /// Matches list of exactly this length, binds/destructures each element
    fn compile_destructure_pattern(
        &mut self,
        pat_list: &[LispVal],
        value_slot: usize,
        outer_env: &Env,
    ) -> bool {
        let expected_len = pat_list.len();

        // Check length
        self.code.push(Op::LoadSlot(value_slot));
        self.code.push(Op::BuiltinCall("length".to_string(), 1));
        self.code.push(Op::PushI64(expected_len as i64));
        self.code.push(Op::Eq);
        let len_fail_idx = self.code.len();
        self.code.push(Op::JumpIfFalse(0)); // if length doesn't match, fail

        // Extract each element into temp slots
        let _elem_slots_start = self.slot_map.len();
        let mut elem_slots: Vec<usize> = Vec::new();
        for i in 0..expected_len {
            let slot = self.slot_map.len();
            self.slot_map.push(format!("__destr_elem_{}", i));
            elem_slots.push(slot);
            self.code.push(Op::LoadSlot(value_slot));
            self.code.push(Op::PushI64(i as i64));
            self.code.push(Op::BuiltinCall("nth".to_string(), 2));
            self.code.push(Op::StoreSlot(slot));
        }

        // Check each element sub-pattern
        let mut fail_jump_indices: Vec<usize> = Vec::new();
        for (i, sub_pat) in pat_list.iter().enumerate() {
            if !self.compile_pattern_check(sub_pat, elem_slots[i], outer_env) {
                return false;
            }
            if i + 1 < expected_len {
                let elem_fail_idx = self.code.len();
                self.code.push(Op::JumpIfFalse(0));
                fail_jump_indices.push(elem_fail_idx);
            }
        }

        // All matched — last element's bool is on stack
        let success_end = self.code.len();
        self.code.push(Op::Jump(0)); // skip over failure block
        let fail_block = self.code.len();
        self.code.push(Op::PushBool(false));

        // Patch success jump to skip failure block
        self.code[success_end] = Op::Jump(self.code.len());

        for &fail_idx in &fail_jump_indices {
            self.code[fail_idx] = Op::JumpIfFalse(fail_block);
        }
        self.code[len_fail_idx] = Op::JumpIfFalse(fail_block);

        // Don't truncate slots here — binding slots added by sub-patterns need to remain
        // for the match body to access. The match handler will clean up all slots after
        // the body (or on pattern failure, before the next clause).

        true
    }
    /// Compile the loop body. Returns the compiled loop or None.
    fn compile_body(
        mut self,
        init_vals: Vec<LispVal>,
        body: &LispVal,
        outer_env: &Env,
    ) -> Option<CompiledLoop> {
        let num_slots = self.slot_map.len();

        if let LispVal::List(parts) = body {
            if parts.first() == Some(&LispVal::Sym("if".into())) {
                let test = parts.get(1)?;
                let then_branch = parts.get(2)?;
                let else_branch = parts.get(3);

                // --- Mega-fuse: detect classic (if (>= counter limit) accum (recur (+ counter step) (+ accum counter))) ---
                if num_slots == 2 {
                    if let (
                        &LispVal::List(ref test_parts),
                        &LispVal::Sym(ref then_name),
                        Some(&LispVal::List(ref else_parts)),
                    ) = (test, then_branch, else_branch)
                    {
                        // test_parts = [">=", counter_sym, limit_num]
                        // else_parts = ["recur", (+ counter step), (+ accum counter)]
                        if test_parts.len() == 3
                            && test_parts[0] == LispVal::Sym(">=".into())
                            && else_parts.len() == 3
                            && else_parts[0] == LispVal::Sym("recur".into())
                        {
                            if let (LispVal::Sym(ref counter_name), LispVal::Num(limit)) =
                                (&test_parts[1], &test_parts[2])
                            {
                                let recur_args = &else_parts[1..];
                                if let (LispVal::List(ref arg1), LispVal::List(ref arg2)) =
                                    (&recur_args[0], &recur_args[1])
                                {
                                    if arg1.len() == 3
                                        && arg2.len() == 3
                                        && arg1[0] == LispVal::Sym("+".into())
                                        && arg2[0] == LispVal::Sym("+".into())
                                    {
                                        if let (
                                            LispVal::Sym(ref a1_sym),
                                            LispVal::Num(a1_step),
                                            LispVal::Sym(ref a2_sym),
                                            LispVal::Sym(ref a2_rhs),
                                        ) = (&arg1[1], &arg1[2], &arg2[1], &arg2[2])
                                        {
                                            // a1 = counter+step, a2 = accum+counter
                                            if a1_sym == counter_name
                                                && a2_sym == then_name
                                                && a2_rhs == counter_name
                                                && counter_name != then_name
                                            {
                                                if let (Some(cs), Some(as_)) = (
                                                    self.slot_of(counter_name),
                                                    self.slot_of(then_name),
                                                ) {
                                                    let jf_idx = self.code.len();
                                                    self.code
                                                        .push(Op::JumpIfSlotGeImm(cs, *limit, 0)); // placeholder
                                                    self.code.push(Op::RecurIncAccum(
                                                        cs, as_, *a1_step, *limit, 0,
                                                    )); // placeholder
                                                        // exit path: LoadSlot(accum), Return — this is what both ops jump to
                                                    let exit_target = self.code.len();
                                                    self.code.push(Op::LoadSlot(as_));
                                                    self.code.push(Op::Return);
                                                    // Patch: both jump to the LoadSlot instruction
                                                    self.code[jf_idx] = Op::JumpIfSlotGeImm(
                                                        cs,
                                                        *limit,
                                                        exit_target,
                                                    );
                                                    self.code[jf_idx + 1] = Op::RecurIncAccum(
                                                        cs,
                                                        as_,
                                                        *a1_step,
                                                        *limit,
                                                        exit_target,
                                                    );

                                                    let captured = self.captured.clone();
                                                    let code = self.code;
                                                    return Some(CompiledLoop {
                                                        num_slots,
                                                        slot_names: self.slot_map,
                                                        init_vals,
                                                        code,
                                                        loop_start_pc: 0,
                                                        captured,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // --- Generic if/recur compilation (fallback) ---
                // Emit else/recur FIRST so peephole sees contiguous window:
                //   test → JumpIfTrue(then_start) → recur args → Recur → then → Return
                if !self.compile_expr(test, outer_env) {
                    return None;
                }
                let jt_idx = self.code.len();
                self.code.push(Op::JumpIfTrue(0)); // placeholder: jump to then when test is true (done)

                // Recur body (else branch) — comes right after condition for contiguous peephole
                if let Some(else_expr) = else_branch {
                    if let LispVal::List(else_parts) = else_expr {
                        if else_parts.first() == Some(&LispVal::Sym("recur".into())) {
                            let recur_args = &else_parts[1..];
                            if recur_args.len() != num_slots {
                                return None;
                            }
                            for arg in recur_args {
                                if !self.compile_expr(arg, outer_env) {
                                    return None;
                                }
                            }
                            self.code.push(Op::Recur(num_slots));
                        } else {
                            if !self.compile_expr(else_expr, outer_env) {
                                return None;
                            }
                            self.code.push(Op::Return);
                        }
                    } else {
                        if !self.compile_expr(else_expr, outer_env) {
                            return None;
                        }
                        self.code.push(Op::Return);
                    }
                } else {
                    self.code.push(Op::PushNil);
                    self.code.push(Op::Return);
                }

                // Then branch — at the end, jumped to when loop is done
                let then_start = self.code.len();
                self.code[jt_idx] = Op::JumpIfTrue(then_start);
                if !self.compile_expr(then_branch, outer_env) {
                    return None;
                }
                self.code.push(Op::Return);
                let captured = self.captured.clone();
                let mut code = self.code;
                peephole_optimize(&mut code, &[], &[]);
                // Second pass: now that 3-op and 2-op fusions are done, check for mega-fuse
                peephole_optimize(&mut code, &[], &[]);
                // Third pass: 2-op fusion may have created new JumpIfSlotCmpImm for mega-fuse
                peephole_optimize(&mut code, &[], &[]);
                return Some(CompiledLoop {
                    num_slots,
                    slot_names: self.slot_map,
                    init_vals,
                    code,
                    loop_start_pc: 0,
                    captured,
                });
            }
            if !self.compile_expr(body, outer_env) {
                return None;
            }
            self.code.push(Op::Return);
            let captured = self.captured.clone();
            let mut code = self.code;
            peephole_optimize(&mut code, &[], &[]);
            // Second pass: now that 3-op and 2-op fusions are done, check for mega-fuse
            peephole_optimize(&mut code, &[], &[]);
            // Third pass: 2-op fusion may have created new JumpIfSlotCmpImm for mega-fuse
            peephole_optimize(&mut code, &[], &[]);
            return Some(CompiledLoop {
                num_slots,
                slot_names: self.slot_map,
                init_vals,
                code,
                loop_start_pc: 0,
                captured,
            });
        }
        None
    }
}

/// Remap an op from a callee's context into the caller's context.
/// - `slot_offset`: add to all slot references
/// - `captured_remap`: remap captured var indices
/// - `jump_offset`: add to all jump targets
fn remap_op(op: &Op, slot_offset: usize, captured_remap: &[usize], jump_offset: usize) -> Op {
    match op {
        // Slot-reading ops — offset slot index
        Op::LoadSlot(s) => Op::LoadSlot(s + slot_offset),
        Op::StoreSlot(s) => Op::StoreSlot(s + slot_offset),
        Op::SlotAddImm(s, imm) => Op::SlotAddImm(s + slot_offset, *imm),
        Op::SlotSubImm(s, imm) => Op::SlotSubImm(s + slot_offset, *imm),
        Op::SlotMulImm(s, imm) => Op::SlotMulImm(s + slot_offset, *imm),
        Op::SlotDivImm(s, imm) => Op::SlotDivImm(s + slot_offset, *imm),
        Op::SlotEqImm(s, imm) => Op::SlotEqImm(s + slot_offset, *imm),
        Op::SlotLtImm(s, imm) => Op::SlotLtImm(s + slot_offset, *imm),
        Op::SlotLeImm(s, imm) => Op::SlotLeImm(s + slot_offset, *imm),
        Op::SlotGtImm(s, imm) => Op::SlotGtImm(s + slot_offset, *imm),
        Op::SlotGeImm(s, imm) => Op::SlotGeImm(s + slot_offset, *imm),

        // Super-fused compare+jump — offset both slot and jump target
        Op::JumpIfSlotLtImm(s, imm, addr) => {
            Op::JumpIfSlotLtImm(s + slot_offset, *imm, addr + jump_offset)
        }
        Op::JumpIfSlotLeImm(s, imm, addr) => {
            Op::JumpIfSlotLeImm(s + slot_offset, *imm, addr + jump_offset)
        }
        Op::JumpIfSlotGtImm(s, imm, addr) => {
            Op::JumpIfSlotGtImm(s + slot_offset, *imm, addr + jump_offset)
        }
        Op::JumpIfSlotGeImm(s, imm, addr) => {
            Op::JumpIfSlotGeImm(s + slot_offset, *imm, addr + jump_offset)
        }
        Op::JumpIfSlotEqImm(s, imm, addr) => {
            Op::JumpIfSlotEqImm(s + slot_offset, *imm, addr + jump_offset)
        }

        // Mega-fused — offset slots and jump target
        Op::RecurIncAccum(a, b, step, limit, addr) => Op::RecurIncAccum(
            a + slot_offset,
            b + slot_offset,
            *step,
            *limit,
            addr + jump_offset,
        ),

        // Jump ops — offset target
        Op::JumpIfTrue(addr) => Op::JumpIfTrue(addr + jump_offset),
        Op::JumpIfFalse(addr) => Op::JumpIfFalse(addr + jump_offset),
        Op::Jump(addr) => Op::Jump(addr + jump_offset),

        // Captured var access — remap captured index
        Op::LoadCaptured(idx) => {
            Op::LoadCaptured(captured_remap.get(*idx).copied().unwrap_or(*idx))
        }
        Op::StoreCaptured(idx) => {
            Op::StoreCaptured(captured_remap.get(*idx).copied().unwrap_or(*idx))
        }
        Op::CallCapturedRef(idx, n) => {
            Op::CallCapturedRef(captured_remap.get(*idx).copied().unwrap_or(*idx), *n)
        }
        Op::LoadGlobal(name) => Op::LoadGlobal(name.clone()),
        Op::StoreGlobal(name) => Op::StoreGlobal(name.clone()),
        // DictMutSet — offset slot
        Op::DictMutSet(s) => Op::DictMutSet(s + slot_offset),

        // CallCaptured — offset slot
        Op::CallCaptured(s, n) => Op::CallCaptured(s + slot_offset, *n),

        // GetDefaultSlot — offset all slots
        Op::GetDefaultSlot(m, k, d, r) => Op::GetDefaultSlot(
            m + slot_offset,
            k + slot_offset,
            d + slot_offset,
            r + slot_offset,
        ),

        // StoreAndLoadSlot — offset slot
        Op::StoreAndLoadSlot(s) => Op::StoreAndLoadSlot(s + slot_offset),

        // ReturnSlot — offset slot
        Op::ReturnSlot(s) => Op::ReturnSlot(s + slot_offset),

        // Everything else — no remapping needed
        _ => op.clone(),
    }
}

/// Peephole optimizer: fuse LoadSlot + PushI64 + Arith/Cmp sequences,
/// convert small Recur → RecurDirect, fuse SlotCmpImm + JumpIfFalse,
/// and remap jump targets.
/// `slot_is_i64` maps slot index → true if known to always hold Num(i64).
/// When provided, converts generic Arith/Cmp ops to typed I64 variants
/// when both source slots are known i64.
/// `slot_is_f64` maps slot index → true if known to always hold Float(f64).
fn peephole_optimize(code: &mut Vec<Op>, slot_is_i64: &[bool], slot_is_f64: &[bool]) {
    // Pre-compute set of jump targets — positions that are jumped to from elsewhere.
    // Used to prevent fusing ops at jump targets (which would break fallthrough semantics).
    let jump_targets: std::collections::HashSet<usize> = code
        .iter()
        .filter_map(|op| match op {
            Op::Jump(t) | Op::JumpIfTrue(t) | Op::JumpIfFalse(t) => Some(*t),
            _ => None,
        })
        .collect();

    let mut i = 0;
    let mut new_code = Vec::with_capacity(code.len());
    // Build old_pc → new_pc mapping so jump targets stay valid
    let mut index_map: Vec<usize> = Vec::with_capacity(code.len());
    while i < code.len() {
        index_map.push(new_code.len());

        // --- Mega-fuse: 6 ops → 1 for the classic sum loop pattern ---
        // JumpIfSlot*CmpImm(counter, limit, exit)
        // SlotAddImm(counter, step)
        // LoadSlot(accum)
        // LoadSlot(counter)
        // Add
        // RecurDirect(2)
        // → RecurIncAccum(counter, accum, step, adjusted_limit, exit)
        // where adjusted_limit accounts for the comparison type:
        //   Ge: limit as-is, Gt: limit+1, Le: limit+1, Lt: limit, Eq: limit
        if i + 5 < code.len() {
            // Extract the counter, limit, and exit from any comparison variant
            let cmp_info: Option<(usize, i64, usize)> = match &code[i] {
                Op::JumpIfSlotGeImm(s, imm, addr) => Some((*s, *imm, *addr)), // >= imm → exit at >= imm
                Op::JumpIfSlotGtImm(s, imm, addr) => Some((*s, imm + 1, *addr)), // > imm → exit at >= imm+1
                Op::JumpIfSlotLeImm(s, imm, addr) => Some((*s, imm + 1, *addr)), // <= imm → exit at >= imm+1
                Op::JumpIfSlotLtImm(s, imm, addr) => Some((*s, *imm, *addr)), // < imm → exit at >= imm
                Op::JumpIfSlotEqImm(s, imm, addr) => Some((*s, *imm, *addr)), // == imm → exit at >= imm (approx)
                _ => None,
            };
            if let Some((counter, limit, exit)) = cmp_info {
                if let (
                    Op::SlotAddImm(cs, step),
                    Op::LoadSlot(accum),
                    Op::LoadSlot(as2),
                    Op::Add,
                    Op::RecurDirect(n),
                ) = (
                    &code[i + 1],
                    &code[i + 2],
                    &code[i + 3],
                    &code[i + 4],
                    &code[i + 5],
                ) {
                    // counter slot must be consistent, n==2 slots, accum != counter,
                    // and second LoadSlot loads the counter (accum += counter)
                    if *n == 2 && counter == *cs && *accum != counter && *as2 == counter {
                        // 6 ops consumed (indices i..i+5); first index_map entry already pushed at top of loop
                        // Push index_map entries for the remaining 5 consumed ops
                        for _ in 0..5 {
                            index_map.push(new_code.len());
                        }
                        new_code.push(Op::RecurIncAccum(counter, *accum, *step, limit, exit));
                        i += 6;
                        continue;
                    }
                }
            }
        }

        // Try to fuse LoadSlot(s) + PushI64(imm) + Arith/Cmp
        if i + 2 < code.len() {
            if let (Op::LoadSlot(s), Op::PushI64(imm)) = (&code[i], &code[i + 1]) {
                let s = *s;
                let imm = *imm;
                let fused = match &code[i + 2] {
                    Op::Add => Some(Op::SlotAddImm(s, imm)),
                    Op::Sub => Some(Op::SlotSubImm(s, imm)),
                    Op::Mul => Some(Op::SlotMulImm(s, imm)),
                    Op::Div => Some(Op::SlotDivImm(s, imm)),
                    Op::Eq => Some(Op::SlotEqImm(s, imm)),
                    Op::Lt => Some(Op::SlotLtImm(s, imm)),
                    Op::Le => Some(Op::SlotLeImm(s, imm)),
                    Op::Gt => Some(Op::SlotGtImm(s, imm)),
                    Op::Ge => Some(Op::SlotGeImm(s, imm)),
                    _ => None,
                };
                if let Some(op) = fused {
                    // Mark fused ops as mapping to the same new index
                    index_map.push(new_code.len());
                    index_map.push(new_code.len());
                    new_code.push(op);
                    i += 3;
                    continue;
                }
            }
        }

        // Convert LoadSlot(s) + PushFloat(f) + BinOp → keep operands, use TypedBinOp F64
        // PushFloat guarantees the operation produces an f64 result.
        if i + 2 < code.len() {
            if let (Op::LoadSlot(_), Op::PushFloat(_)) = (&code[i], &code[i + 1]) {
                let typed = match &code[i + 2] {
                    Op::Add => Some(Op::TypedBinOp(BinOp::Add, Ty::F64)),
                    Op::Sub => Some(Op::TypedBinOp(BinOp::Sub, Ty::F64)),
                    Op::Mul => Some(Op::TypedBinOp(BinOp::Mul, Ty::F64)),
                    Op::Div => Some(Op::TypedBinOp(BinOp::Div, Ty::F64)),
                    Op::Mod => Some(Op::TypedBinOp(BinOp::Mod, Ty::F64)),
                    Op::Lt => Some(Op::TypedBinOp(BinOp::Lt, Ty::F64)),
                    Op::Le => Some(Op::TypedBinOp(BinOp::Le, Ty::F64)),
                    Op::Gt => Some(Op::TypedBinOp(BinOp::Gt, Ty::F64)),
                    Op::Ge => Some(Op::TypedBinOp(BinOp::Ge, Ty::F64)),
                    Op::Eq => Some(Op::TypedBinOp(BinOp::Eq, Ty::F64)),
                    _ => None,
                };
                if let Some(top) = typed {
                    index_map.push(new_code.len());
                    new_code.push(code[i].clone());
                    index_map.push(new_code.len());
                    new_code.push(code[i + 1].clone());
                    index_map.push(new_code.len());
                    new_code.push(top);
                    i += 3;
                    continue;
                }
            }
        }

        // Also match PushFloat(f) + LoadSlot(s) + BinOp (reversed operand order)
        if i + 2 < code.len() {
            if let (Op::PushFloat(_), Op::LoadSlot(_)) = (&code[i], &code[i + 1]) {
                let typed = match &code[i + 2] {
                    Op::Add => Some(Op::TypedBinOp(BinOp::Add, Ty::F64)),
                    Op::Sub => Some(Op::TypedBinOp(BinOp::Sub, Ty::F64)),
                    Op::Mul => Some(Op::TypedBinOp(BinOp::Mul, Ty::F64)),
                    Op::Div => Some(Op::TypedBinOp(BinOp::Div, Ty::F64)),
                    Op::Mod => Some(Op::TypedBinOp(BinOp::Mod, Ty::F64)),
                    Op::Lt => Some(Op::TypedBinOp(BinOp::Lt, Ty::F64)),
                    Op::Le => Some(Op::TypedBinOp(BinOp::Le, Ty::F64)),
                    Op::Gt => Some(Op::TypedBinOp(BinOp::Gt, Ty::F64)),
                    Op::Ge => Some(Op::TypedBinOp(BinOp::Ge, Ty::F64)),
                    Op::Eq => Some(Op::TypedBinOp(BinOp::Eq, Ty::F64)),
                    _ => None,
                };
                if let Some(top) = typed {
                    index_map.push(new_code.len());
                    new_code.push(code[i].clone());
                    index_map.push(new_code.len());
                    new_code.push(code[i + 1].clone());
                    index_map.push(new_code.len());
                    new_code.push(top);
                    i += 3;
                    continue;
                }
            }
        }

        // Convert LoadSlot(a) + LoadSlot(b) + {Add|Sub|...} → typed variant
        // when both slots are known to hold the same type.
        if i + 2 < code.len() {
            if let (Op::LoadSlot(a), Op::LoadSlot(b)) = (&code[i], &code[i + 1]) {
                let a = *a;
                let b = *b;
                let sa = slot_is_i64.get(a).copied().unwrap_or(false);
                let sb = slot_is_i64.get(b).copied().unwrap_or(false);
                let fa = slot_is_f64.get(a).copied().unwrap_or(false);
                let fb = slot_is_f64.get(b).copied().unwrap_or(false);
                if sa && sb {
                    let typed = match &code[i + 2] {
                        Op::Add => Some(Op::TypedBinOp(BinOp::Add, Ty::I64)),
                        Op::Sub => Some(Op::TypedBinOp(BinOp::Sub, Ty::I64)),
                        Op::Mul => Some(Op::TypedBinOp(BinOp::Mul, Ty::I64)),
                        Op::Lt => Some(Op::TypedBinOp(BinOp::Lt, Ty::I64)),
                        Op::Le => Some(Op::TypedBinOp(BinOp::Le, Ty::I64)),
                        Op::Gt => Some(Op::TypedBinOp(BinOp::Gt, Ty::I64)),
                        Op::Ge => Some(Op::TypedBinOp(BinOp::Ge, Ty::I64)),
                        Op::Eq => Some(Op::TypedBinOp(BinOp::Eq, Ty::I64)),
                        _ => None,
                    };
                    if let Some(top) = typed {
                        index_map.push(new_code.len());
                        new_code.push(code[i].clone());
                        index_map.push(new_code.len());
                        new_code.push(code[i + 1].clone());
                        index_map.push(new_code.len());
                        new_code.push(top);
                        i += 3;
                        continue;
                    }
                } else if fa && fb {
                    let typed = match &code[i + 2] {
                        Op::Add => Some(Op::TypedBinOp(BinOp::Add, Ty::F64)),
                        Op::Sub => Some(Op::TypedBinOp(BinOp::Sub, Ty::F64)),
                        Op::Mul => Some(Op::TypedBinOp(BinOp::Mul, Ty::F64)),
                        Op::Lt => Some(Op::TypedBinOp(BinOp::Lt, Ty::F64)),
                        Op::Le => Some(Op::TypedBinOp(BinOp::Le, Ty::F64)),
                        Op::Gt => Some(Op::TypedBinOp(BinOp::Gt, Ty::F64)),
                        Op::Ge => Some(Op::TypedBinOp(BinOp::Ge, Ty::F64)),
                        Op::Eq => Some(Op::TypedBinOp(BinOp::Eq, Ty::F64)),
                        _ => None,
                    };
                    if let Some(top) = typed {
                        index_map.push(new_code.len());
                        new_code.push(code[i].clone());
                        index_map.push(new_code.len());
                        new_code.push(code[i + 1].clone());
                        index_map.push(new_code.len());
                        new_code.push(top);
                        i += 3;
                        continue;
                    }
                }
            }
        }
        // Try to fuse SlotCmpImm(s, imm) + JumpIfTrue(addr)
        // JumpIfTrue: jump when condition is true → fused op matches its name directly
        if i + 1 < code.len() {
            let fused = match (&code[i], &code[i + 1]) {
                (Op::SlotLtImm(s, imm), Op::JumpIfTrue(addr)) => {
                    Some(Op::JumpIfSlotLtImm(*s, *imm, *addr))
                }
                (Op::SlotLeImm(s, imm), Op::JumpIfTrue(addr)) => {
                    Some(Op::JumpIfSlotLeImm(*s, *imm, *addr))
                }
                (Op::SlotGtImm(s, imm), Op::JumpIfTrue(addr)) => {
                    Some(Op::JumpIfSlotGtImm(*s, *imm, *addr))
                }
                (Op::SlotGeImm(s, imm), Op::JumpIfTrue(addr)) => {
                    Some(Op::JumpIfSlotGeImm(*s, *imm, *addr))
                }
                (Op::SlotEqImm(s, imm), Op::JumpIfTrue(addr)) => {
                    Some(Op::JumpIfSlotEqImm(*s, *imm, *addr))
                }
                _ => None,
            };
            if let Some(op) = fused {
                index_map.push(new_code.len());
                new_code.push(op);
                i += 2;
                continue;
            }
        }
        // Convert Recur(n) with n <= 4 to RecurDirect(n)
        if let Op::Recur(n) = &code[i] {
            if *n <= 4 {
                new_code.push(Op::RecurDirect(*n));
                i += 1;
                continue;
            }
        }
        // Fuse get-default pattern (11 ops → 1):
        // LoadSlot(m) → LoadSlot(k) → DictGet → StoreSlot(tmp) → LoadSlot(tmp)
        // → BuiltinCall("nil?", 1) → JumpIfFalse(else) → LoadSlot(default) → Jump(end)
        // → LoadSlot(tmp) → StoreSlot(result)
        // → GetDefaultSlot(m, k, default, result)
        if i + 10 < code.len() {
            if let (
                Op::LoadSlot(m_slot),
                Op::LoadSlot(k_slot),
                Op::DictGet,
                Op::StoreSlot(tmp_slot),
                Op::LoadSlot(ls5),
                Op::BuiltinCall(name, 1),
                Op::JumpIfFalse(else_addr),
                Op::LoadSlot(default_slot),
                Op::Jump(end_addr),
                Op::LoadSlot(ls10),
                Op::StoreSlot(result_slot),
            ) = (
                &code[i],
                &code[i + 1],
                &code[i + 2],
                &code[i + 3],
                &code[i + 4],
                &code[i + 5],
                &code[i + 6],
                &code[i + 7],
                &code[i + 8],
                &code[i + 9],
                &code[i + 10],
            ) {
                if name == "nil?"
                    && ls5 == tmp_slot
                    && ls10 == tmp_slot
                    && *else_addr == i + 9
                    && *end_addr == i + 10
                {
                    // All 11 indices map to the same new index
                    for _ in 0..10 {
                        index_map.push(new_code.len());
                    }
                    new_code.push(Op::GetDefaultSlot(
                        *m_slot,
                        *k_slot,
                        *default_slot,
                        *result_slot,
                    ));
                    i += 11;
                    continue;
                }
            }
        }
        // Fuse standalone get-default pattern (10 ops → 1):
        // Same as above but ends with Return instead of StoreSlot
        // Result is pushed onto stack (not stored in a slot)
        if i + 9 < code.len() {
            if let (
                Op::LoadSlot(m_slot),
                Op::LoadSlot(k_slot),
                Op::DictGet,
                Op::StoreSlot(tmp_slot),
                Op::LoadSlot(ls4),
                Op::BuiltinCall(name, 1),
                Op::JumpIfFalse(else_addr),
                Op::LoadSlot(default_slot),
                Op::Jump(end_addr),
                Op::LoadSlot(ls9),
            ) = (
                &code[i],
                &code[i + 1],
                &code[i + 2],
                &code[i + 3],
                &code[i + 4],
                &code[i + 5],
                &code[i + 6],
                &code[i + 7],
                &code[i + 8],
                &code[i + 9],
            ) {
                if name == "nil?"
                    && ls4 == tmp_slot
                    && ls9 == tmp_slot
                    && *else_addr == i + 9
                    && *end_addr == i + 10
                {
                    // 10 ops → push default onto stack, skip Return
                    for _ in 0..9 {
                        index_map.push(new_code.len());
                    }
                    // Reuse GetDefaultSlot but we need to push result onto stack.
                    // Use a temp: store to a dummy slot, then load it.
                    // Actually, just emit the fused op with result going to the tmp slot,
                    // then LoadSlot to push it.
                    new_code.push(Op::GetDefaultSlot(
                        *m_slot,
                        *k_slot,
                        *default_slot,
                        *tmp_slot,
                    ));
                    new_code.push(Op::LoadSlot(*tmp_slot));
                    i += 10; // consume 10 ops (excluding the Return after)
                    continue;
                }
            }
        }
        // Fuse StoreSlot(N) + LoadSlot(N) → StoreAndLoadSlot(N)
        // Only if i+1 is not a jump target (the LoadSlot must be reachable only from StoreSlot)
        if i + 1 < code.len() && !jump_targets.contains(&(i + 1)) {
            if let (Op::StoreSlot(s1), Op::LoadSlot(s2)) = (&code[i], &code[i + 1]) {
                if s1 == s2 {
                    index_map.push(new_code.len());
                    new_code.push(Op::StoreAndLoadSlot(*s1));
                    i += 2;
                    continue;
                }
            }
        }
        // Fuse LoadSlot(N) + Return → ReturnSlot(N)
        // Only if i is not a jump target (the LoadSlot must be reachable only from predecessor)
        if i + 1 < code.len() && !jump_targets.contains(&i) {
            if let (Op::LoadSlot(s), Op::Return) = (&code[i], &code[i + 1]) {
                index_map.push(new_code.len());
                new_code.push(Op::ReturnSlot(*s));
                i += 2;
                continue;
            }
        }
        // Eliminate LoadSlot(N) + StoreSlot(N) — loads value then stores it back, a no-op.
        // Only if i+1 is not a jump target.
        if i + 1 < code.len() && !jump_targets.contains(&(i + 1)) {
            if let (Op::LoadSlot(s1), Op::StoreSlot(s2)) = (&code[i], &code[i + 1]) {
                if s1 == s2 {
                    index_map.push(new_code.len());
                    // Both ops are a no-op — skip them
                    i += 2;
                    continue;
                }
            }
        }
        new_code.push(code[i].clone());
        i += 1;
    }
    // Remap jump targets using the index map
    for op in &mut new_code {
        remap_jump_target(op, &index_map);
    }
    *code = new_code;
}

/// Remap a jump target from old PC to new PC using the index map.
fn remap_jump_target(op: &mut Op, index_map: &[usize]) {
    match op {
        Op::JumpIfFalse(addr) | Op::JumpIfTrue(addr) | Op::Jump(addr) => {
            if *addr < index_map.len() {
                *addr = index_map[*addr];
            }
        }
        Op::JumpIfSlotLtImm(_, _, addr)
        | Op::JumpIfSlotLeImm(_, _, addr)
        | Op::JumpIfSlotGtImm(_, _, addr)
        | Op::JumpIfSlotGeImm(_, _, addr)
        | Op::JumpIfSlotEqImm(_, _, addr)
        | Op::RecurIncAccum(_, _, _, _, addr) => {
            if *addr < index_map.len() {
                *addr = index_map[*addr];
            }
        }
        _ => {}
    }
}

/// Run a compiled loop. Returns the result.
fn run_compiled_loop(cl: &CompiledLoop) -> Result<LispVal, String> {
    // Slot-based env: binding slots + captured env slots, direct index access
    let mut slots: Vec<LispVal> = cl.init_vals.clone();
    // Append captured env values after binding slots
    for (_, val) in &cl.captured {
        slots.push(val.clone());
    }
    let mut stack: Vec<LispVal> = Vec::with_capacity(16);
    let code = &cl.code;
    let mut pc: usize = 0;

    loop {
        match &code[pc] {
            Op::LoadSlot(s) => {
                // Num fast path: avoid full Clone for the common case
                let slot_ref = safe_slot(&slots, *s);
                match slot_ref {
                    LispVal::Num(n) => stack.push(LispVal::Num(*n)),
                    _ => stack.push(slot_ref.clone()),
                }
                pc += 1;
            }
            Op::LoadCaptured(idx) => {
                stack.push(cl.captured[*idx].1.clone());
                pc += 1;
            }
            Op::StoreCaptured(idx) => {
                // CompiledLoop uses plain Vec — StoreCaptured shouldn't appear in loops
                return Err(format!("StoreCaptured({}) in loop VM — not supported", idx));
            }
            Op::LoadGlobal(name) => {
                // Loop VM doesn't have outer_env access — globals shouldn't appear in loops
                return Err(format!("LoadGlobal({}) in loop VM — not supported", name));
            }
            Op::StoreGlobal(name) => {
                return Err(format!("StoreGlobal({}) in loop VM — not supported", name));
            }
            Op::PushI64(n) => {
                stack.push(LispVal::Num(*n));
                pc += 1;
            }
            Op::PushFloat(f) => {
                stack.push(LispVal::Float(*f));
                pc += 1;
            }
            Op::PushBool(b) => {
                stack.push(LispVal::Bool(*b));
                pc += 1;
            }
            Op::PushStr(s) => {
                stack.push(LispVal::Str(s.clone()));
                pc += 1;
            }
            Op::PushNil => {
                stack.push(LispVal::Nil);
                pc += 1;
            }
            Op::MakeList(n) => {
                let mut items = Vec::with_capacity(*n);
                for _ in 0..*n {
                    items.push(stack.pop().unwrap_or(LispVal::Nil));
                }
                items.reverse();
                stack.push(LispVal::List(items));
                pc += 1;
            }
            Op::Dup => {
                if let Some(top) = stack.last() {
                    stack.push(top.clone());
                }
                pc += 1;
            }
            Op::Pop => {
                stack.pop();
                pc += 1;
            }
            Op::StoreSlot(s) => {
                slots[*s] = stack.pop().unwrap_or(LispVal::Nil);
                pc += 1;
            }
            Op::Add => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(num_arith_checked(&a, &b, "add", i64::checked_add, |x, y| x + y)?);
                pc += 1;
            }
            Op::Sub => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(num_arith_checked(&a, &b, "sub", i64::checked_sub, |x, y| x - y)?);
                pc += 1;
            }
            Op::Mul => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(num_arith_checked(&a, &b, "mul", i64::checked_mul, |x, y| x * y)?);
                pc += 1;
            }
            Op::Div => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                check_float_zero(&a, &b, "division")?;
                stack.push(num_arith_checked(&a, &b, "div", i64::checked_div, |x, y| x / y)?);
                pc += 1;
            }
            Op::Mod => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                check_float_zero(&a, &b, "modulo")?;
                stack.push(num_arith_checked(&a, &b, "mod", i64::checked_rem, |x, y| x % y)?);
                pc += 1;
            }
            Op::Eq => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(lisp_eq(&a, &b)));
                pc += 1;
            }
            Op::Lt => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(num_cmp(&a, &b, |x, y| x < y, |x, y| x < y)));
                pc += 1;
            }
            Op::Le => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(num_cmp(&a, &b, |x, y| x <= y, |x, y| x <= y)));
                pc += 1;
            }
            Op::Gt => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(num_cmp(&a, &b, |x, y| x > y, |x, y| x > y)));
                pc += 1;
            }
            Op::Ge => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(num_cmp(&a, &b, |x, y| x >= y, |x, y| x >= y)));
                pc += 1;
            }
            Op::Not => {
                let v = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(!is_truthy(&v)));
                pc += 1;
            }
            // Typed binary ops — zero dynamic dispatch
            Op::TypedBinOp(op, ty) => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                match ty {
                    Ty::I64 => {
                        let av = match &a {
                            LispVal::Num(n) => *n,
                            _ => 0,
                        };
                        let bv = match &b {
                            LispVal::Num(n) => *n,
                            _ => 0,
                        };
                        stack.push(match op {
                            BinOp::Add => {
                                LispVal::Num(i64::checked_add(av, bv)
                                    .ok_or("integer overflow in add")?)
                            }
                            BinOp::Sub => {
                                LispVal::Num(i64::checked_sub(av, bv)
                                    .ok_or("integer overflow in sub")?)
                            }
                            BinOp::Mul => {
                                LispVal::Num(i64::checked_mul(av, bv)
                                    .ok_or("integer overflow in mul")?)
                            }
                            BinOp::Div => {
                                LispVal::Num(i64::checked_div(av, bv)
                                    .ok_or("integer overflow in div")?)
                            }
                            BinOp::Mod => {
                                LispVal::Num(i64::checked_rem(av, bv)
                                    .ok_or("integer overflow in mod")?)
                            }
                            BinOp::Lt => LispVal::Bool(av < bv),
                            BinOp::Le => LispVal::Bool(av <= bv),
                            BinOp::Gt => LispVal::Bool(av > bv),
                            BinOp::Ge => LispVal::Bool(av >= bv),
                            BinOp::Eq => LispVal::Bool(av == bv),
                        });
                    }
                    Ty::F64 => {
                        let av = match &a {
                            LispVal::Float(f) => *f,
                            LispVal::Num(n) => *n as f64,
                            _ => 0.0,
                        };
                        let bv = match &b {
                            LispVal::Float(f) => *f,
                            LispVal::Num(n) => *n as f64,
                            _ => 0.0,
                        };
                        stack.push(match op {
                            BinOp::Add => LispVal::Float(av + bv),
                            BinOp::Sub => LispVal::Float(av - bv),
                            BinOp::Mul => LispVal::Float(av * bv),
                            BinOp::Div => LispVal::Float(av / bv),
                            BinOp::Mod => LispVal::Float(av % bv),
                            BinOp::Lt => LispVal::Bool(av < bv),
                            BinOp::Le => LispVal::Bool(av <= bv),
                            BinOp::Gt => LispVal::Bool(av > bv),
                            BinOp::Ge => LispVal::Bool(av >= bv),
                            BinOp::Eq => LispVal::Bool(av == bv),
                        });
                    }
                }
                pc += 1;
            }
            Op::JumpIfTrue(addr) => {
                let v = stack.pop().unwrap_or(LispVal::Nil);
                if is_truthy(&v) {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfFalse(addr) => {
                let v = stack.pop().unwrap_or(LispVal::Nil);
                if !is_truthy(&v) {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::Jump(addr) => {
                pc = *addr;
            }
            Op::Return => {
                return Ok(stack.pop().unwrap_or(LispVal::Nil));
            }
            Op::Recur(n) => {
                // Direct reverse-order pop into slots — no Vec, no reverse
                for i in (0..*n).rev() {
                    slots[i] = stack.pop().unwrap_or(LispVal::Nil);
                }
                pc = 0; // jump to loop start
            }
            Op::RecurDirect(n) => {
                // Same as Recur but guaranteed small N (no Vec allocation)
                for i in (0..*n).rev() {
                    slots[i] = stack.pop().unwrap_or(LispVal::Nil);
                }
                pc = 0; // jump to loop start
            }
            // --- Compound ops: fused LoadSlot + PushI64 + Arith/Cmp ---
            Op::SlotAddImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                match i64::checked_add(v, *imm) {
                    Some(result) => {
                        stack.push(LispVal::Num(result));
                        pc += 1;
                    }
                    None => return Err("integer overflow in add".into()),
                }
            }
            Op::SlotSubImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                match i64::checked_sub(v, *imm) {
                    Some(result) => {
                        stack.push(LispVal::Num(result));
                        pc += 1;
                    }
                    None => return Err("integer overflow in sub".into()),
                }
            }
            Op::SlotMulImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                match i64::checked_mul(v, *imm) {
                    Some(result) => {
                        stack.push(LispVal::Num(result));
                        pc += 1;
                    }
                    None => return Err("integer overflow in mul".into()),
                }
            }
            Op::SlotDivImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                match i64::checked_div(v, *imm) {
                    Some(result) => {
                        stack.push(LispVal::Num(result));
                        pc += 1;
                    }
                    None => return Err("integer overflow in div".into()),
                }
            }
            Op::SlotEqImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                stack.push(LispVal::Bool(v == *imm));
                pc += 1;
            }
            Op::SlotLtImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                stack.push(LispVal::Bool(v < *imm));
                pc += 1;
            }
            Op::SlotLeImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                stack.push(LispVal::Bool(v <= *imm));
                pc += 1;
            }
            Op::SlotGtImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                stack.push(LispVal::Bool(v > *imm));
                pc += 1;
            }
            Op::SlotGeImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                stack.push(LispVal::Bool(v >= *imm));
                pc += 1;
            }
            // --- Super-fused: cmp + jump without stack traffic ---
            Op::JumpIfSlotLtImm(s, imm, addr) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                if v < *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfSlotLeImm(s, imm, addr) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                if v <= *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfSlotGtImm(s, imm, addr) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                if v > *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfSlotGeImm(s, imm, addr) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                if v >= *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfSlotEqImm(s, imm, addr) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                if v == *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            // --- Mega-fused: entire loop body in one op ---
            // RecurIncAccum(counter_slot, accum_slot, step, limit, exit_addr)
            Op::RecurIncAccum(counter, accum, step, limit, exit_addr) => {
                let cv = num_val_ref(&slots[*counter]);
                if cv >= *limit {
                    pc = *exit_addr;
                } else {
                    let av = num_val_ref(&slots[*accum]);
                    slots[*accum] = LispVal::Num(av + cv);
                    slots[*counter] = LispVal::Num(cv + step);
                    pc = 0; // jump to loop start
                }
            }
            Op::BuiltinCall(name, n_args) => {
                let mut args: Vec<LispVal> = Vec::with_capacity(*n_args);
                for _ in 0..*n_args {
                    args.push(stack.pop().unwrap_or(LispVal::Nil));
                }
                args.reverse();
                let result = eval_builtin(name, &args, None, None)?;
                stack.push(result);
                pc += 1;
            }
            Op::DictGet => {
                let key = stack.pop().unwrap_or(LispVal::Nil);
                let map = stack.pop().unwrap_or(LispVal::Nil);
                let result = match (&map, &key) {
                    (LispVal::Map(m), LispVal::Str(k)) => m.get(k).cloned().unwrap_or(LispVal::Nil),
                    _ => LispVal::Nil,
                };
                stack.push(result);
                pc += 1;
            }
            Op::DictSet => {
                let val = stack.pop().unwrap_or(LispVal::Nil);
                let key = stack.pop().unwrap_or(LispVal::Nil);
                let map = stack.pop().unwrap_or(LispVal::Nil);
                let result = match (&map, &key) {
                    (LispVal::Map(m), LispVal::Str(k)) => LispVal::Map(m.update(k.clone(), val)),
                    _ => return Err("dict/set: need (map key value)".into()),
                };
                stack.push(result);
                pc += 1;
            }
            Op::StoreAndLoadSlot(s) => {
                let val = stack.pop().unwrap_or(LispVal::Nil);
                slots[*s] = val;
                match safe_slot(&slots, *s) {
                    LispVal::Num(n) => stack.push(LispVal::Num(*n)),
                    _ => stack.push(slots[*s].clone()),
                }
                pc += 1;
            }
            Op::CallCaptured(_, _)
            | Op::CallCapturedRef(_, _) | Op::PushSelf
            | Op::PushClosure(_)
            | Op::PushBuiltin(_)
            | Op::PushLiteral(_)
            | Op::CallSelf(_)
            | Op::CallDynamic(_)
            | Op::StoreCaptured(_)
            | Op::StoreGlobal(_)
            | Op::DictMutSet(_)
            | Op::GetDefaultSlot(_, _, _, _)
            | Op::ReturnSlot(_)
            | Op::ConstructTag(_, _, _)
            | Op::TagTest(_, _)
            | Op::GetField(_)
            | Op::TracePush(_)
            | Op::TracePop => {
                return Err(
                    "loop VM: CallCaptured/CallSelf/DictMutSet/GetDefaultSlot/ReturnSlot not supported in loop body".into(),
                );
            }
        }
    }
}

/// Check if a value matches a type spec. Returns Result<bool, String>.
fn check_type(val: &LispVal, ty: &LispVal) -> Result<bool, String> {
    // Handle compound type specs (LispVal::List)
    if let LispVal::List(list) = ty {
        if list.is_empty() {
            return Ok(false);
        }
        let head = match &list[0] {
            LispVal::Sym(s) => s.as_str(),
            LispVal::Str(s) => s.as_str(),
            _ => return Ok(false),
        };
        match head {
            // (:or T1 T2 ...) — union: matches if any Ti matches
            ":or" => {
                for t in &list[1..] {
                    if check_type(val, t)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            // (:list T) — homogeneous list: all elements must match T
            ":list" => {
                if let LispVal::List(elems) = val {
                    if elems.is_empty() {
                        return Ok(true); // empty list always matches
                    }
                    let elem_type = if list.len() > 1 { &list[1] } else {
                        return Ok(true); // (:list) with no elem type = any list
                    };
                    for elem in elems {
                        if !check_type(elem, elem_type)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            // (:map K V) — map where all keys match K and values match V
            ":map" => {
                if let LispVal::Map(m) = val {
                    let key_type = if list.len() > 1 { &list[1] } else {
                        return Ok(true); // (:map) with no types = any map
                    };
                    let val_type = if list.len() > 2 { &list[2] } else {
                        return Ok(true); // (:map K) = map with key type K
                    };
                    for (k, v) in m.iter() {
                        let key_val = LispVal::Str(k.clone());
                        if !check_type(&key_val, key_type)? {
                            return Ok(false);
                        }
                        if !check_type(v, val_type)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            // (:tuple T1 T2 ...) — tuple: list of exactly len elements
            ":tuple" => {
                if let LispVal::List(elems) = val {
                    let expected_types: &[LispVal] = &list[1..];
                    if elems.len() != expected_types.len() {
                        return Ok(false);
                    }
                    for (elem, t) in elems.iter().zip(expected_types.iter()) {
                        if !check_type(elem, t)? {
                            return Ok(false);
                        }
                    }
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            // (:fn T1 ... -> R) or (:fn T1 ... → R) — arrow type: matches any fn
            ":fn" | ":function" => {
                // :fn as a compound spec just checks that the value is callable
                Ok(matches!(val,
                    LispVal::Lambda { .. } | LispVal::BuiltinFn(_) | LispVal::CaseLambda { .. } | LispVal::Memoized { .. }
                ))
            }
            _ => Ok(false),
        }
    } else {
        // Flat type spec (symbol or string)
        let type_name = match ty {
            LispVal::Sym(s) => s.as_str(),
            LispVal::Str(s) => s.as_str(),
            _ => return Ok(false),
        };
        match type_name {
            ":int" => Ok(matches!(val, LispVal::Num(_))),
            ":str" | ":string" => Ok(matches!(val, LispVal::Str(_))),
            ":float" => Ok(matches!(val, LispVal::Float(_))),
            ":num" | ":number" => Ok(matches!(val, LispVal::Num(_) | LispVal::Float(_))),
            ":bool" => Ok(matches!(val, LispVal::Bool(_))),
            ":list" => Ok(matches!(val, LispVal::List(_))),
            ":map" | ":dict" => Ok(matches!(val, LispVal::Map(_))),
            ":fn" | ":function" => Ok(matches!(val,
                LispVal::Lambda { .. } | LispVal::BuiltinFn(_) | LispVal::CaseLambda { .. } | LispVal::Memoized { .. }
            )),
            ":any" => Ok(true),
            ":nil" => Ok(matches!(val, LispVal::Nil)),
            ":sym" => Ok(matches!(val, LispVal::Sym(_))),
            _ => Ok(false),
        }
    }
}

/// Get a human-readable name for a type spec.
fn type_spec_name(ty: &LispVal) -> String {
    match ty {
        LispVal::Sym(s) => s.clone(),
        LispVal::Str(s) => s.clone(),
        other => format!("{:?}", other),
    }
}

/// Get a human-readable type name for a value.
fn val_type_name(val: &LispVal) -> String {
    match val {
        LispVal::Num(_) => ":int".into(),
        LispVal::Float(_) => ":float".into(),
        LispVal::Str(_) => ":str".into(),
        LispVal::Bool(_) => ":bool".into(),
        LispVal::Nil => ":nil".into(),
        LispVal::Sym(_) => ":sym".into(),
        LispVal::List(_) => ":list".into(),
        LispVal::Map(_) => ":map".into(),
        LispVal::Lambda { .. } => ":fn".into(),
        LispVal::BuiltinFn(_) => ":fn".into(),
        LispVal::CaseLambda { .. } => ":fn".into(),
        LispVal::Macro { .. } => ":macro".into(),
        LispVal::Delay { .. } => ":promise".into(),
        LispVal::Memoized { .. } => ":fn".into(),
        LispVal::Tagged { .. } => ":tagged".into(),
        _ => ":any".into(),
    }
}

/// Infer a basic type signature from a function value by scanning its bytecode.
/// Returns a string like "(:int -> :int)" or "(:any -> :str)".
fn infer_type_from_val(val: &LispVal) -> String {
    let (n_params, has_rest, code): (usize, bool, &[Op]) = match val {
        LispVal::Lambda { params, rest_param, compiled, .. } => {
            let n = if rest_param.is_some() { params.len().saturating_sub(1) } else { params.len() };
            let bc = compiled.as_ref().map(|cl| cl.code.as_slice()).unwrap_or(&[]);
            (n, rest_param.is_some(), bc)
        }
        LispVal::Memoized { func, .. } => match func.as_ref() {
            LispVal::Lambda { params, rest_param, compiled, .. } => {
                let n = if rest_param.is_some() { params.len().saturating_sub(1) } else { params.len() };
                let bc = compiled.as_ref().map(|cl| cl.code.as_slice()).unwrap_or(&[]);
                (n, true, bc) // memoized always has rest
            }
            _ => return "(:any -> :any)".into(),
        },
        _ => return "(:any -> :any)".into(),
    };

    // Guess return type by scanning last few ops for type-revealing operations
    let ret_type = guess_return_type_from_ops(code);
    let param_str = if n_params == 0 {
        String::new()
    } else {
        ":any".to_string()
    };
    let rest_str = if has_rest { "*" } else { "" };
    format!("({}{}{} -> {})", param_str, if n_params > 0 && has_rest { " " } else { "" }, rest_str, ret_type)
}

/// Scan bytecode ops to guess the return type of a function.
fn guess_return_type_from_ops(code: &[Op]) -> &'static str {
    // Look at last few ops before Return for type-revealing operations
    let mut has_arith = false;
    let mut has_float_arith = false;
    let mut has_str_op = false;
    let mut has_cmp = false;
    let mut has_list_op = false;

    for op in code.iter().rev().take(20) {
        match op {
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod | Op::PushI64(_) => has_arith = true,
            Op::TypedBinOp(_, Ty::F64) => has_float_arith = true,
            Op::TypedBinOp(_, Ty::I64) => has_arith = true,
            Op::BuiltinCall(name, _) | Op::PushBuiltin(name) | Op::LoadGlobal(name) => match name.as_str() {
                "str-concat" | "str-replace" | "string->number" | "number->string"
                | "substring" | "string-length" | "string-contains?" | "string-split"
                | "str-upcase" | "str-downcase" | "str-trim" | "symbol->string"
                | "string-append" | "string-prefix?" | "string-suffix?" => has_str_op = true,
                "list" | "cons" | "append" | "reverse" | "take" | "drop" | "range" | "list-ref"
                | "list-tail" | "sort" | "zip" | "list?" => has_list_op = true,
                "num->str" | "to-json" | "fmt" => has_str_op = true,
                _ => {}
            },
            Op::Eq => has_cmp = true,
            Op::TypedBinOp(_, Ty::I64) | Op::TypedBinOp(_, Ty::F64) => has_cmp = true,
            Op::PushLiteral(LispVal::Str(_)) | Op::PushLiteral(LispVal::Sym(_)) => {
                has_str_op = true
            }
            Op::PushLiteral(LispVal::Num(_)) => has_arith = true,
            Op::PushLiteral(LispVal::Float(_)) => has_float_arith = true,
            Op::PushLiteral(LispVal::List(_)) => has_list_op = true,
            Op::MakeList(_) => has_list_op = true,
            _ => {}
        }
    }

    if has_float_arith { ":float" }
    else if has_str_op && !has_arith { ":str" }
    else if has_list_op && !has_arith { ":list" }
    else if has_cmp && !has_arith { ":bool" }
    else if has_arith { ":int" }
    else { ":any" }
}

/// Check if a type spec is valid (recognized).
fn is_valid_type_spec(ty: &LispVal) -> bool {
    match ty {
        LispVal::Sym(s) => matches!(s.as_str(),
            ":int" | ":str" | ":string" | ":float" | ":num" | ":number"
            | ":bool" | ":list" | ":map" | ":dict" | ":fn" | ":function"
            | ":any" | ":nil" | ":sym"
        ),
        LispVal::Str(s) => matches!(s.as_str(),
            ":int" | ":str" | ":string" | ":float" | ":num" | ":number"
            | ":bool" | ":list" | ":map" | ":dict" | ":fn" | ":function"
            | ":any" | ":nil" | ":sym"
        ),
        // Compound type specs: (:or ...), (:list ...), (:map ...), (:tuple ...), (:fn ...)
        LispVal::List(list) if !list.is_empty() => {
            match &list[0] {
                LispVal::Sym(s) => matches!(s.as_str(), ":or" | ":list" | ":map" | ":tuple" | ":fn" | ":function"),
                LispVal::Str(s) => matches!(s.as_str(), ":or" | ":list" | ":map" | ":tuple" | ":fn" | ":function"),
                _ => false,
            }
        }
        _ => false,
    }
}

/// Extract i64 from LispVal
pub fn num_val(v: LispVal) -> i64 {
    match v {
        LispVal::Num(n) => n,
        LispVal::Float(f) => f as i64,
        _ => 0,
    }
}

pub fn num_val_ref(v: &LispVal) -> i64 {
    match v {
        LispVal::Num(n) => *n,
        LispVal::Float(f) => *f as i64,
        _ => 0,
    }
}

/// Extract f64 from any LispVal for float arithmetic promotion.
pub fn num_val_ref_f64(v: &LispVal) -> f64 {
    match v {
        LispVal::Float(f) => *f,
        LispVal::Num(n) => *n as f64,
        _ => 0.0,
    }
}

/// Safe slot read — returns Nil on out-of-bounds (matches SpecVm behavior).
#[inline]
fn safe_slot<'a>(slots: &'a [LispVal], idx: usize) -> &'a LispVal {
    slots.get(idx).unwrap_or(&LispVal::Nil)
}

/// Polymorphic arithmetic: if either operand is Float, use float arithmetic.
fn num_arith(
    a: &LispVal,
    b: &LispVal,
    int_op: impl Fn(i64, i64) -> i64,
    float_op: impl Fn(f64, f64) -> f64,
) -> LispVal {
    match (a, b) {
        (LispVal::Float(x), LispVal::Float(y)) => LispVal::Float(float_op(*x, *y)),
        (LispVal::Float(x), LispVal::Num(y)) => LispVal::Float(float_op(*x, *y as f64)),
        (LispVal::Num(x), LispVal::Float(y)) => LispVal::Float(float_op(*x as f64, *y)),
        (LispVal::Num(x), LispVal::Num(y)) => LispVal::Num(int_op(*x, *y)),
        _ => {
            if matches!(a, LispVal::Float(_)) || matches!(b, LispVal::Float(_)) {
                LispVal::Float(float_op(num_val_ref_f64(a), num_val_ref_f64(b)))
            } else {
                LispVal::Num(int_op(num_val_ref(a), num_val_ref(b)))
            }
        }
    }
}

/// Like num_arith but uses checked integer arithmetic — returns Err on overflow.
fn num_arith_checked(
    a: &LispVal,
    b: &LispVal,
    op_name: &str,
    int_op: impl Fn(i64, i64) -> Option<i64>,
    float_op: impl Fn(f64, f64) -> f64,
) -> Result<LispVal, String> {
    match (a, b) {
        (LispVal::Float(x), LispVal::Float(y)) => Ok(LispVal::Float(float_op(*x, *y))),
        (LispVal::Float(x), LispVal::Num(y)) => Ok(LispVal::Float(float_op(*x, *y as f64))),
        (LispVal::Num(x), LispVal::Float(y)) => Ok(LispVal::Float(float_op(*x as f64, *y))),
        (LispVal::Num(x), LispVal::Num(y)) => match int_op(*x, *y) {
            Some(r) => Ok(LispVal::Num(r)),
            None => Err(format!("integer overflow in {}", op_name)),
        },
        // Non-numeric operands: if either is Float, promote to float arithmetic
        _ => {
            let af = matches!(a, LispVal::Float(_)) || matches!(b, LispVal::Float(_));
            if af {
                Ok(LispVal::Float(float_op(num_val_ref_f64(a), num_val_ref_f64(b))))
            } else {
                let av = num_val_ref(a);
                let bv = num_val_ref(b);
                match int_op(av, bv) {
                    Some(r) => Ok(LispVal::Num(r)),
                    None => Err(format!("integer overflow in {}", op_name)),
                }
            }
        }
    }
}

/// Check if a float div/mod operation would divide by zero.
/// Returns Err("division by zero") or Err("modulo by zero") if the divisor is 0.0 or 0.
/// This matches the spec VM behavior: IEEE 754 NaN/inf are NOT produced.
fn check_float_zero(a: &LispVal, b: &LispVal, op_name: &str) -> Result<(), String> {
    let divisor_f64 = match b {
        LispVal::Float(f) => *f,
        LispVal::Num(n) => *n as f64,
        _ if matches!(a, LispVal::Float(_)) || matches!(b, LispVal::Float(_)) => num_val_ref_f64(b),
        _ => return Ok(()), // Pure integer path — checked_div/checked_rem handle it
    };
    if divisor_f64 == 0.0 {
        Err(format!("{} by zero", op_name))
    } else {
        Ok(())
    }
}

/// Polymorphic numeric comparison: returns bool, float-aware.
/// Non-numeric operands return false (matches spec VM behavior).
fn num_cmp(a: &LispVal, b: &LispVal, op: impl Fn(f64, f64) -> bool, int_op: impl Fn(i64, i64) -> bool) -> bool {
    match (a, b) {
        (LispVal::Float(x), LispVal::Float(y)) => op(*x, *y),
        (LispVal::Float(x), LispVal::Num(y)) => op(*x, *y as f64),
        (LispVal::Num(x), LispVal::Float(y)) => op(*x as f64, *y),
        (LispVal::Num(x), LispVal::Num(y)) => int_op(*x, *y),
        _ => false,
    }
}

/// Lisp equality
pub fn lisp_eq(a: &LispVal, b: &LispVal) -> bool {
    match (a, b) {
        (LispVal::Num(x), LispVal::Num(y)) => x == y,
        (LispVal::Float(x), LispVal::Float(y)) => x == y,
        (LispVal::Num(x), LispVal::Float(y)) => (*x as f64) == *y,
        (LispVal::Float(x), LispVal::Num(y)) => *x == (*y as f64),
        (LispVal::Bool(x), LispVal::Bool(y)) => x == y,
        (LispVal::Str(x), LispVal::Str(y)) => x == y,
        (LispVal::Nil, LispVal::Nil) => true,
        // Structural equality for complex types
        (LispVal::List(a), LispVal::List(b)) => a == b,
        (LispVal::Tagged { type_name: ta, variant_id: va, fields: fa },
         LispVal::Tagged { type_name: tb, variant_id: vb, fields: fb }) => {
            ta == tb && va == vb && fa == fb
        }
        _ => false,
    }
}

/// Evaluate a builtin by name (for Op::BuiltinCall)
/// Check if a name is a NEAR builtin
fn eval_near_builtin_match(name: &str) -> bool {
    matches!(name,
        "storage-write" | "storage_write"
        | "storage-read" | "storage_read"
        | "storage-remove" | "storage_remove"
        | "storage-has-key" | "storage_has_key"
        | "block-height" | "block_height"
        | "block-timestamp" | "block_timestamp"
        | "signer-account-id" | "signer_account_id"
        | "predecessor-account-id" | "predecessor_account_id"
        | "current-account-id" | "current_account_id"
        | "attached-deposit" | "attached_deposit"
        | "account-balance" | "account_balance"
        | "log-utf8" | "log_utf8" | "log"
        | "near-config" | "near_config"
        | "near-reset" | "near_reset"
    )
}

/// Evaluate mock NEAR builtins. Returns Some(result) if handled, None otherwise.
fn eval_near_builtin(
    name: &str,
    args: &[LispVal],
    state: &mut EvalState,
) -> Option<Result<LispVal, String>> {
    match name {
        "storage-write" | "storage_write" => {
            let key = match args.get(0) {
                Some(LispVal::Str(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => return Some(Err("storage-write: need key".into())),
            };
            let val = args.get(1).cloned().unwrap_or(LispVal::Nil);
            state.near_storage.insert(key, val);
            Some(Ok(LispVal::Bool(true)))
        }
        "storage-read" | "storage_read" => {
            let key = match args.get(0) {
                Some(LispVal::Str(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => return Some(Err("storage-read: need key".into())),
            };
            Some(Ok(state.near_storage.get(&key).cloned().unwrap_or(LispVal::Nil)))
        }
        "storage-remove" | "storage_remove" => {
            let key = match args.get(0) {
                Some(LispVal::Str(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => return Some(Err("storage-remove: need key".into())),
            };
            state.near_storage.remove(&key);
            Some(Ok(LispVal::Bool(true)))
        }
        "storage-has-key" | "storage_has_key" => {
            let key = match args.get(0) {
                Some(LispVal::Str(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => return Some(Err("storage-has-key: need key".into())),
            };
            Some(Ok(LispVal::Bool(state.near_storage.contains_key(&key))))
        }
        "block-height" | "block_height" => {
            Some(Ok(state.near_context.get("block_height").cloned()
                .unwrap_or(LispVal::Num(42_000_000))))
        }
        "block-timestamp" | "block_timestamp" => {
            Some(Ok(state.near_context.get("block_timestamp").cloned()
                .unwrap_or(LispVal::Num(1_714_000_000_000_000_000))))
        }
        "signer-account-id" | "signer_account_id" => {
            Some(Ok(state.near_context.get("signer_account_id").cloned()
                .unwrap_or(LispVal::Str("alice.near".into()))))
        }
        "predecessor-account-id" | "predecessor_account_id" => {
            Some(Ok(state.near_context.get("predecessor_account_id").cloned()
                .unwrap_or(LispVal::Str("bob.near".into()))))
        }
        "current-account-id" | "current_account_id" => {
            Some(Ok(state.near_context.get("current_account_id").cloned()
                .unwrap_or(LispVal::Str("contract.near".into()))))
        }
        "attached-deposit" | "attached_deposit" => {
            Some(Ok(state.near_context.get("attached_deposit").cloned()
                .unwrap_or(LispVal::Num(0))))
        }
        "account-balance" | "account_balance" => {
            Some(Ok(state.near_context.get("account_balance").cloned()
                .unwrap_or(LispVal::Str("10000000000000000000000000".into()))))  // 10 NEAR (yocto)
        }
        "log-utf8" | "log_utf8" | "log" => {
            let msg = match args.get(0) {
                Some(LispVal::Str(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => return Some(Ok(LispVal::Nil)),
            };
            eprintln!("[log] {}", msg);
            Some(Ok(LispVal::Nil))
        }
        "near-config" | "near_config" => {
            if args.len() == 2 {
                let key = match &args[0] {
                    LispVal::Str(s) => s.clone(),
                    v => v.to_string(),
                };
                state.near_context.insert(key, args[1].clone());
                return Some(Ok(LispVal::Bool(true)));
            }
            Some(Ok(LispVal::Nil))
        }
        "near-reset" | "near_reset" => {
            state.near_storage.clear();
            Some(Ok(LispVal::Bool(true)))
        }
        _ => None,
    }
}

pub fn eval_builtin(
    name: &str,
    args: &[LispVal],
    env: Option<&mut Env>,
    state: Option<&mut EvalState>,
) -> Result<LispVal, String> {
    // ── Mock NEAR builtins (checked first to avoid consuming state) ──
    if eval_near_builtin_match(name) {
        return match state {
            Some(st) => eval_near_builtin(name, args, st)
                .unwrap_or_else(|| Err(format!("NEAR builtin '{}' failed", name))),
            None => Err(format!("NEAR builtin '{}' requires mutable state", name)),
        };
    }

    match name {
        // ── Promises (delay/force) ──
        "make-promise" => {
            // (make-promise thunk) → wraps a 0-param closure in a Delay
            let thunk = args.get(0).cloned().unwrap_or(LispVal::Nil);
            Ok(LispVal::Delay {
                thunk: Box::new(thunk),
                cache: std::sync::Arc::new(std::sync::RwLock::new(None)),
            })
        }
        // ── Try/Catch ──
        "try-catch-impl" => {
            // (try-catch-impl try-thunk catch-thunk)
            // Calls try-thunk (0-arg lambda). On error, calls catch-thunk with error string.
            let (env_ref, state_ref) = match (env, state) {
                (Some(e), Some(s)) => (e, s),
                _ => return Err("try-catch: not available in loop context".into()),
            };
            let try_fn = args.get(0).cloned().unwrap_or(LispVal::Nil);
            let catch_fn = args.get(1).cloned().unwrap_or(LispVal::Nil);
            match vm_call_lambda(&try_fn, &[], env_ref, state_ref) {
                Ok(val) => Ok(val),
                Err(e) => vm_call_lambda(&catch_fn, &[LispVal::Str(e)], env_ref, state_ref),
            }
        }
        "force" => {
            // (force promise) → evaluates the thunk on first call, returns cached value
            let (env_ref, state_ref) = match (env, state) {
                (Some(e), Some(s)) => (e, s),
                _ => return Err("force: not available in loop context".into()),
            };
            match args.get(0) {
                Some(LispVal::Delay { thunk, cache }) => {
                    // Check if already forced
                    {
                        let cached = cache.read().unwrap();
                        if let Some(ref val) = *cached {
                            return Ok(val.clone());
                        }
                    }
                    // Force: call the thunk
                    let result = vm_call_lambda(thunk, &[], env_ref, state_ref);
                    match result {
                        Ok(val) => {
                            *cache.write().unwrap() = Some(val.clone());
                            Ok(val)
                        }
                        Err(e) => {
                            // Per Scheme semantics, re-raise on error (don't cache errors)
                            Err(e)
                        }
                    }
                }
                Some(other) => Err(format!("force: expected a promise, got {}", other)),
                None => Err("force: expected 1 argument".into()),
            }
        }
        // fork-exec: (fork-exec thunk) → snapshot env, call thunk, restore env, return result
        "fork-exec" => {
            let thunk = match args.get(0) {
                Some(t) => t,
                None => return Err("fork-exec: expected thunk argument".into()),
            };
            let (env_ref, state_ref) = match (env, state) {
                (Some(e), Some(s)) => (e, s),
                _ => return Err("fork-exec: requires env and state".into()),
            };
            let saved = env_ref.snapshot();
            let result = vm_call_lambda(thunk, &[], env_ref, state_ref);
            env_ref.restore(saved);
            result
        }
        // memoize: (memoize lambda) → returns a cached wrapper
        "memoize" => {
            let func = match args.get(0) {
                Some(f @ LispVal::Lambda { .. }) => f.clone(),
                _ => return Err("memoize: expected a lambda argument".into()),
            };
            let cache = std::sync::Arc::new(std::sync::RwLock::new(im::HashMap::new()));
            Ok(LispVal::Memoized {
                func: Box::new(func),
                cache,
            })
        }
        // par-map: (par-map fn list) → map fn over list
        "par-map" => {
            let func = match args.get(0) {
                Some(f) => f,
                None => return Err("par-map: expected function and list".into()),
            };
            let list = match args.get(1) {
                Some(LispVal::List(l)) => l,
                _ => return Err("par-map: expected list as second argument".into()),
            };
            let (env_ref, state_ref) = match (env, state) {
                (Some(e), Some(s)) => (e, s),
                _ => return Err("par-map: requires env and state".into()),
            };
            let mut results = Vec::new();
            for item in list.iter() {
                let r = vm_call_lambda(func, &[item.clone()], env_ref, state_ref);
                results.push(r.unwrap_or(LispVal::Nil));
            }
            Ok(LispVal::List(results))
        }
        // par-filter: (par-filter pred list) → filter list by pred
        "par-filter" => {
            let func = match args.get(0) {
                Some(f) => f,
                None => return Err("par-filter: expected predicate and list".into()),
            };
            let list = match args.get(1) {
                Some(LispVal::List(l)) => l,
                _ => return Err("par-filter: expected list as second argument".into()),
            };
            let (env_ref, state_ref) = match (env, state) {
                (Some(e), Some(s)) => (e, s),
                _ => return Err("par-filter: requires env and state".into()),
            };
            let mut results = Vec::new();
            for item in list.iter() {
                let r = vm_call_lambda(func, &[item.clone()], env_ref, state_ref);
                if let Ok(LispVal::Bool(true)) = r {
                    results.push(item.clone());
                }
            }
            Ok(LispVal::List(results))
        }
        // snapshot: (snapshot) → save env, return id
        "snapshot" => {
            let (env_ref, state_ref) = match (env, state) {
                (Some(e), Some(s)) => (e, s),
                _ => return Err("snapshot: requires env and state".into()),
            };
            let snap = env_ref.snapshot();
            let id = state_ref.snapshots.len() as i64;
            state_ref.snapshots.push(snap);
            Ok(LispVal::Num(id))
        }
        // rollback: (rollback) → restore most recent snapshot
        "rollback" => {
            let (env_ref, state_ref) = match (env, state) {
                (Some(e), Some(s)) => (e, s),
                _ => return Err("rollback: requires env and state".into()),
            };
            match state_ref.snapshots.pop() {
                Some(snap) => {
                    env_ref.restore(snap);
                    Ok(LispVal::Nil)
                }
                None => Err("rollback: no snapshots available".into()),
            }
        }
        // fmt: (fmt template-string dict) → string with {key} placeholders replaced
        "fmt" => {
            let template = match args.get(0) {
                Some(LispVal::Str(s)) => s.clone(),
                _ => return Err("fmt: expected template string as first argument".into()),
            };
            let dict = match args.get(1) {
                Some(LispVal::Map(map)) => map,
                _ => return Err("fmt: expected dict as second argument".into()),
            };
            let re = regex::Regex::new(r"\{(\w+)\}").map_err(|e| format!("fmt: regex error: {}", e))?;
            let result = re.replace_all(&template, |caps: &regex::Captures| {
                let key = &caps[1];
                match dict.get(key) {
                    Some(LispVal::Str(s)) => s.clone(),
                    Some(val) => val.to_string(),
                    None => caps[0].to_string(),
                }
            });
            Ok(LispVal::Str(result.to_string()))
        }
        // inspect: (inspect value) → human-readable type description
        "inspect" => {
            let val = args.get(0).cloned().unwrap_or(LispVal::Nil);
            let desc = match &val {
                LispVal::Num(n) => format!("<integer {}>", n),
                LispVal::Float(f) => format!("<float {}>", f),
                LispVal::Str(s) => format!("<string len={} \"{}\">", s.len(), s),
                LispVal::Bool(b) => format!("<boolean {}>", b),
                LispVal::Nil => "<nil>".to_string(),
                LispVal::List(l) => format!("<list len={}>", l.len()),
                LispVal::Map(d) => format!("<dict len={}>", d.len()),
                LispVal::Lambda { params, rest_param, .. } => {
                    let n = params.len();
                    let rest = if rest_param.is_some() { n - 1 } else { n };
                    format!("<lambda params={}>", rest)
                }
                LispVal::BuiltinFn(name) => format!("<builtin {}>", name),
                LispVal::Sym(s) => format!("<symbol {}>", s),
                LispVal::Macro { params, .. } => format!("<macro params={}>", params.len()),
                LispVal::Tagged { type_name, variant_id, .. } => {
                    format!("<tagged {}::{}>", type_name, variant_id)
                }
                LispVal::Delay { .. } => "<promise>".to_string(),
                LispVal::Memoized { .. } => "<memoized>".to_string(),
                LispVal::CaseLambda { clauses } => {
                    format!("<case-lambda clauses={}>", clauses.len())
                }
                _ => format!("<unknown>"),
            };
            Ok(LispVal::Str(desc))
        }
        // to-json: (to-json value) → JSON string
        "to-json" => {
            fn to_json(val: &LispVal) -> String {
                match val {
                    LispVal::Num(n) => n.to_string(),
                    LispVal::Float(f) => f.to_string(),
                    LispVal::Str(s) => format!("\"{}\"", s),
                    LispVal::Bool(b) => (if *b { "true" } else { "false" }).to_string(),
                    LispVal::Nil => "null".to_string(),
                    LispVal::List(l) => {
                        let items: Vec<String> = l.iter().map(to_json).collect();
                        format!("[{}]", items.join(","))
                    }
                    LispVal::Map(d) => {
                        let pairs: Vec<String> = d.iter()
                            .map(|(k, v)| format!("\"{}\":{}", k, to_json(v)))
                            .collect();
                        format!("{{{}}}", pairs.join(","))
                    }
                    other => format!("\"{}\"", other),
                }
            }
            let val = args.get(0).cloned().unwrap_or(LispVal::Nil);
            Ok(LispVal::Str(to_json(&val)))
        }
        // str-replace: (str-replace haystack needle replacement) → string
        "str-replace" => {
            let haystack = match args.get(0) {
                Some(LispVal::Str(s)) => s.clone(),
                _ => return Err("str-replace: expected string as first argument".into()),
            };
            let needle = match args.get(1) {
                Some(LispVal::Str(s)) => s.clone(),
                _ => return Err("str-replace: expected string as second argument".into()),
            };
            let replacement = match args.get(2) {
                Some(LispVal::Str(s)) => s.clone(),
                _ => return Err("str-replace: expected string as third argument".into()),
            };
            Ok(LispVal::Str(haystack.replace(&needle, &replacement)))
        }
        "promise?" => {
            Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::Delay { .. }))))
        }
        "macro?" => {
            Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::Macro { .. }))))
        }
        "make-case-lambda" => {
            // args are compiled lambda closures, one per clause
            let mut clauses = Vec::new();
            for lambda_val in args.iter() {
                let (n_fixed, has_rest) = match lambda_val {
                    LispVal::Lambda { params, rest_param, .. } => {
                        let n = params.len();
                        let has = rest_param.is_some();
                        (n.saturating_sub(if has { 1 } else { 0 }), has)
                    }
                    _ => return Err("make-case-lambda: expected lambda arguments".into()),
                };
                clauses.push((n_fixed, has_rest, lambda_val.clone()));
            }
            Ok(LispVal::CaseLambda { clauses })
        }
        "abs" => match args.get(0) {
            Some(LispVal::Num(n)) => Ok(LispVal::Num(n.abs())),
            Some(LispVal::Float(f)) => Ok(LispVal::Float(f.abs())),
            _ => Ok(LispVal::Num(0)),
        },
        "min" => {
            if args.is_empty() { return Ok(LispVal::Num(0)); }
            let has_float = args.iter().any(|a| matches!(a, LispVal::Float(_)));
            if has_float {
                let mut best = match args.get(0) {
                    Some(LispVal::Float(f)) => *f,
                    Some(LispVal::Num(n)) => *n as f64,
                    _ => f64::INFINITY,
                };
                for a in &args[1..] {
                    let v = match a {
                        LispVal::Float(f) => *f,
                        LispVal::Num(n) => *n as f64,
                        _ => f64::INFINITY,
                    };
                    if v < best { best = v; }
                }
                Ok(LispVal::Float(best))
            } else {
                let mut best = num_val(args.get(0).cloned().unwrap_or(LispVal::Nil));
                for a in &args[1..] {
                    let v = num_val(a.clone());
                    if v < best { best = v; }
                }
                Ok(LispVal::Num(best))
            }
        }
        "max" => {
            if args.is_empty() { return Ok(LispVal::Num(0)); }
            let has_float = args.iter().any(|a| matches!(a, LispVal::Float(_)));
            if has_float {
                let mut best = match args.get(0) {
                    Some(LispVal::Float(f)) => *f,
                    Some(LispVal::Num(n)) => *n as f64,
                    _ => f64::NEG_INFINITY,
                };
                for a in &args[1..] {
                    let v = match a {
                        LispVal::Float(f) => *f,
                        LispVal::Num(n) => *n as f64,
                        _ => f64::NEG_INFINITY,
                    };
                    if v > best { best = v; }
                }
                Ok(LispVal::Float(best))
            } else {
                let mut best = num_val(args.get(0).cloned().unwrap_or(LispVal::Nil));
                for a in &args[1..] {
                    let v = num_val(a.clone());
                    if v > best { best = v; }
                }
                Ok(LispVal::Num(best))
            }
        }
        "to-string" => Ok(LispVal::Str(format!(
            "{}",
            args.get(0).unwrap_or(&LispVal::Nil)
        ))),
        "str" => Ok(LispVal::Str(
            args.iter().map(|a| format!("{}", a)).collect(),
        )),
        "car" => match args.get(0) {
            Some(LispVal::List(l)) => Ok(l.first().cloned().unwrap_or(LispVal::Nil)),
            _ => Ok(LispVal::Nil),
        },
        "cdr" => match args.get(0) {
            Some(LispVal::List(l)) => {
                if l.len() > 1 {
                    Ok(LispVal::List(l[1..].to_vec()))
                } else {
                    Ok(LispVal::Nil)
                }
            }
            _ => Ok(LispVal::Nil),
        },
        "cons" => {
            let head = args.get(0).cloned().unwrap_or(LispVal::Nil);
            let tail = match args.get(1) {
                Some(LispVal::List(l)) => l.clone(),
                Some(LispVal::Nil) | None => vec![],
                other => vec![other.cloned().unwrap_or(LispVal::Nil)],
            };
            Ok(LispVal::List(vec![head].into_iter().chain(tail).collect()))
        }
        "list" => Ok(LispVal::List(args.to_vec())),
        "length" => match args.get(0) {
            Some(LispVal::List(l)) => Ok(LispVal::Num(l.len() as i64)),
            Some(LispVal::Str(s)) => Ok(LispVal::Num(s.len() as i64)),
            _ => Ok(LispVal::Num(0)),
        },
        "empty?" => match args.get(0) {
            Some(LispVal::List(l)) => Ok(LispVal::Bool(l.is_empty())),
            Some(LispVal::Nil) => Ok(LispVal::Bool(true)),
            _ => Ok(LispVal::Bool(false)),
        },
        "zero?" => Ok(LispVal::Bool(
            num_val(args.get(0).cloned().unwrap_or(LispVal::Nil)) == 0,
        )),
        "pos?" => Ok(LispVal::Bool(
            num_val(args.get(0).cloned().unwrap_or(LispVal::Nil)) > 0,
        )),
        "neg?" => Ok(LispVal::Bool(
            num_val(args.get(0).cloned().unwrap_or(LispVal::Nil)) < 0,
        )),
        "mod" => {
            let b = num_val(args.get(1).cloned().unwrap_or(LispVal::Nil));
            if b == 0 {
                return Err("mod by zero".into());
            } else {
                let a = num_val(args.get(0).cloned().unwrap_or(LispVal::Nil));
                Ok(LispVal::Num(a.rem_euclid(b)))
            }
        }
        "remainder" => {
            let b = num_val(args.get(1).cloned().unwrap_or(LispVal::Nil));
            if b == 0 {
                return Err("remainder by zero".into());
            }
            Ok(LispVal::Num(
                num_val(args.get(0).cloned().unwrap_or(LispVal::Nil)) % b,
            ))
        }
        "even?" => Ok(LispVal::Bool(
            num_val(args.get(0).cloned().unwrap_or(LispVal::Nil)) % 2 == 0,
        )),
        "odd?" => Ok(LispVal::Bool(
            num_val(args.get(0).cloned().unwrap_or(LispVal::Nil)) % 2 != 0,
        )),
        "nil?" => Ok(LispVal::Bool(matches!(
            args.get(0),
            Some(LispVal::Nil) | None
        ))),
        "len" => match args.get(0) {
            Some(LispVal::List(l)) => Ok(LispVal::Num(l.len() as i64)),
            Some(LispVal::Str(s)) => Ok(LispVal::Num(s.len() as i64)),
            _ => Ok(LispVal::Num(0)),
        },
        "append" => {
            let mut result = Vec::new();
            for arg in args {
                if let LispVal::List(l) = arg {
                    result.extend(l.iter().cloned());
                } else {
                    result.push(arg.clone());
                }
            }
            Ok(LispVal::List(result))
        }
        "nth" => match (args.get(0), args.get(1)) {
            (Some(LispVal::List(l)), Some(LispVal::Num(i))) => {
                Ok(l.get(*i as usize).cloned().unwrap_or(LispVal::Nil))
            }
            _ => Ok(LispVal::Nil),
        },
        "str-concat" => {
            let s: String = args
                .iter()
                .map(|a| match a {
                    LispVal::Str(st) => st.clone(),
                    _ => format!("{}", a),
                })
                .collect();
            Ok(LispVal::Str(s))
        }
        "str-length" => match args.get(0) {
            Some(LispVal::Str(s)) => Ok(LispVal::Num(s.len() as i64)),
            _ => Ok(LispVal::Num(0)),
        },
        "str-split" => match (args.get(0), args.get(1)) {
            (Some(LispVal::Str(s)), Some(LispVal::Str(sep))) => {
                let parts: Vec<LispVal> =
                    s.split(sep).map(|p| LispVal::Str(p.to_string())).collect();
                Ok(LispVal::List(parts))
            }
            _ => Ok(LispVal::List(vec![])),
        },
        "str-contains" => match (args.get(0), args.get(1)) {
            (Some(LispVal::Str(s)), Some(LispVal::Str(needle))) => {
                Ok(LispVal::Bool(s.contains(needle.as_str())))
            }
            _ => Ok(LispVal::Bool(false)),
        },
        "to-int" => match args.get(0) {
            Some(LispVal::Num(n)) => Ok(LispVal::Num(*n)),
            Some(LispVal::Float(f)) => Ok(LispVal::Num(*f as i64)),
            Some(LispVal::Str(s)) => s
                .parse::<i64>()
                .map(LispVal::Num)
                .or_else(|_| Ok(LispVal::Num(0))),
            _ => Ok(LispVal::Num(0)),
        },
        "to-float" => match args.get(0) {
            Some(LispVal::Num(n)) => Ok(LispVal::Float(*n as f64)),
            Some(LispVal::Float(f)) => Ok(LispVal::Float(*f)),
            _ => Ok(LispVal::Float(0.0)),
        },
        // --- Additional builtins for lambda bytecode ---
        "inc" => {
            let n = num_val(args.get(0).cloned().unwrap_or(LispVal::Nil));
            Ok(LispVal::Num(n + 1))
        }
        "dec" => {
            let n = num_val(args.get(0).cloned().unwrap_or(LispVal::Nil));
            Ok(LispVal::Num(n - 1))
        }
        "first" => match args.get(0) {
            Some(LispVal::List(l)) => Ok(l.first().cloned().unwrap_or(LispVal::Nil)),
            _ => Ok(LispVal::Nil),
        },
        "rest" => match args.get(0) {
            Some(LispVal::List(l)) => {
                if l.len() > 1 {
                    Ok(LispVal::List(l[1..].to_vec()))
                } else {
                    Ok(LispVal::Nil)
                }
            }
            _ => Ok(LispVal::Nil),
        },
        "equal?" => {
            let a = args.get(0).unwrap_or(&LispVal::Nil);
            let b = args.get(1).unwrap_or(&LispVal::Nil);
            Ok(LispVal::Bool(crate::helpers::lisp_equal(a, b)))
        }
        "not" => {
            let v = args.get(0).unwrap_or(&LispVal::Nil);
            Ok(LispVal::Bool(!is_truthy(v)))
        }
        "string?" => Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::Str(_))))),
        "number?" => Ok(LispVal::Bool(matches!(
            args.get(0),
            Some(LispVal::Num(_)) | Some(LispVal::Float(_))
        ))),
        "boolean?" => Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::Bool(_))))),
        "list?" => Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::List(_))))),
        "pair?" => Ok(LispVal::Bool(
            matches!(args.get(0), Some(LispVal::List(l)) if l.len() >= 2),
        )),
        "symbol?" => Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::Sym(_))))),
        "int?" => Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::Num(_))))),
        "float?" => Ok(LispVal::Bool(matches!(
            args.get(0),
            Some(LispVal::Float(_))
        ))),
        "reverse" => match args.get(0) {
            Some(LispVal::List(l)) => Ok(LispVal::List(l.iter().rev().cloned().collect())),
            Some(LispVal::Nil) | None => Ok(LispVal::List(vec![])),
            _ => Ok(LispVal::Nil),
        },
        "take" => match (args.get(0), args.get(1)) {
            (Some(LispVal::Num(n)), Some(LispVal::List(l))) => {
                Ok(LispVal::List(l.iter().take(*n as usize).cloned().collect()))
            }
            _ => Ok(LispVal::Nil),
        },
        "drop" => match (args.get(0), args.get(1)) {
            (Some(LispVal::Num(n)), Some(LispVal::List(l))) => {
                Ok(LispVal::List(l.iter().skip(*n as usize).cloned().collect()))
            }
            _ => Ok(LispVal::Nil),
        },
        "last" => match args.get(0) {
            Some(LispVal::List(l)) => Ok(l.last().cloned().unwrap_or(LispVal::Nil)),
            _ => Ok(LispVal::Nil),
        },
        "butlast" => match args.get(0) {
            Some(LispVal::List(l)) if l.len() > 1 => Ok(LispVal::List(l[..l.len() - 1].to_vec())),
            _ => Ok(LispVal::Nil),
        },
        "range" => {
            let start = num_val(args.get(0).cloned().unwrap_or(LispVal::Nil));
            let end = num_val(args.get(1).cloned().unwrap_or(LispVal::Nil));
            let step = if args.len() > 2 {
                num_val(args.get(2).cloned().unwrap_or(LispVal::Nil))
            } else {
                1
            };
            let mut result = Vec::new();
            let mut i = start;
            if step > 0 {
                while i < end {
                    result.push(LispVal::Num(i));
                    i += step;
                }
            } else if step < 0 {
                while i > end {
                    result.push(LispVal::Num(i));
                    i += step;
                }
            }
            Ok(LispVal::List(result))
        }
        "sqrt" => {
            let n = match args.get(0) {
                Some(LispVal::Num(n)) => *n as f64,
                Some(LispVal::Float(f)) => *f,
                _ => 0.0,
            };
            let result = n.sqrt();
            // Return Num for perfect integer squares
            if result.fract() == 0.0 && result.abs() <= i64::MAX as f64 {
                Ok(LispVal::Num(result as i64))
            } else {
                Ok(LispVal::Float(result))
            }
        }
        "pow" => {
            let base = num_val(args.get(0).cloned().unwrap_or(LispVal::Nil));
            let exp = num_val(args.get(1).cloned().unwrap_or(LispVal::Nil));
            Ok(LispVal::Float((base as f64).powf(exp as f64)))
        }
        // "dict" handled by dispatch_json fallback below (takes key-value pairs)
        "dict/get" | "dict-ref" => match (args.get(0), args.get(1)) {
            (Some(LispVal::Map(m)), Some(LispVal::Str(key))) => {
                Ok(m.get(key).cloned().unwrap_or(LispVal::Nil))
            }
            _ => Ok(LispVal::Nil),
        },
        "dict/set" | "dict-set" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(LispVal::Map(m)), Some(LispVal::Str(key)), Some(val)) => {
                Ok(LispVal::Map(m.update(key.clone(), val.clone())))
            }
            _ => Err("dict/set: need (map key value)".into()),
        },
        "dict/has?" => match (args.get(0), args.get(1)) {
            (Some(LispVal::Map(m)), Some(LispVal::Str(key))) => {
                Ok(LispVal::Bool(m.contains_key(key)))
            }
            _ => Ok(LispVal::Bool(false)),
        },
        "dict/keys" => match args.get(0) {
            Some(LispVal::Map(m)) => {
                let keys: Vec<LispVal> = m.keys().map(|k| LispVal::Str(k.clone())).collect();
                Ok(LispVal::List(keys))
            }
            _ => Ok(LispVal::List(vec![])),
        },
        // --- String operations (not in existing builtins) ---
        "string-append" => {
            let mut result = String::new();
            for arg in args {
                match arg {
                    LispVal::Str(s) => result.push_str(&s),
                    LispVal::Num(n) => result.push_str(&n.to_string()),
                    LispVal::Float(f) => result.push_str(&f.to_string()),
                    LispVal::Bool(b) => result.push_str(&b.to_string()),
                    LispVal::Nil => result.push_str("nil"),
                    other => result.push_str(&other.to_string()),
                }
            }
            Ok(LispVal::Str(result))
        }
        "str-ends-with" | "string-suffix?" => match (args.get(0), args.get(1)) {
            (Some(LispVal::Str(s)), Some(LispVal::Str(suffix))) => {
                Ok(LispVal::Bool(s.ends_with(suffix.as_str())))
            }
            _ => Ok(LispVal::Bool(false)),
        },
        "str-starts-with" | "string-prefix?" => match (args.get(0), args.get(1)) {
            (Some(LispVal::Str(s)), Some(LispVal::Str(prefix))) => {
                Ok(LispVal::Bool(s.starts_with(prefix.as_str())))
            }
            _ => Ok(LispVal::Bool(false)),
        },
        "substring" => match (args.get(0), args.get(1), args.get(2)) {
            (Some(LispVal::Str(s)), Some(LispVal::Num(start)), None) => {
                let start = (*start as usize).min(s.len());
                Ok(LispVal::Str(s[start..].to_string()))
            }
            (Some(LispVal::Str(s)), Some(LispVal::Num(start)), Some(LispVal::Num(end))) => {
                let start = (*start as usize).min(s.len());
                let end = (*end as usize).min(s.len());
                if start < end {
                    Ok(LispVal::Str(s[start..end].to_string()))
                } else {
                    Ok(LispVal::Str(String::new()))
                }
            }
            _ => Ok(LispVal::Str(String::new())),
        },
        "str->num" | "string->number" => match args.get(0) {
            Some(LispVal::Str(s)) => {
                if let Ok(n) = s.parse::<i64>() {
                    Ok(LispVal::Num(n))
                } else if let Ok(f) = s.parse::<f64>() {
                    Ok(LispVal::Float(f))
                } else {
                    Ok(LispVal::Bool(false))
                }
            }
            _ => Ok(LispVal::Bool(false)),
        },
        "num->str" | "number->string" => match args.get(0) {
            Some(LispVal::Num(n)) => Ok(LispVal::Str(n.to_string())),
            Some(LispVal::Float(f)) => Ok(LispVal::Str(f.to_string())),
            _ => Ok(LispVal::Str("0".to_string())),
        },
        // --- Time ---
        "now" => {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            Ok(LispVal::Float(ts))
        }
        "elapsed" => match args.get(0) {
            Some(v) => {
                let since = crate::helpers::as_float(v).unwrap_or(0.0);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                Ok(LispVal::Float(now - since))
            }
            None => Ok(LispVal::Float(0.0)),
        },
        // --- Type conversions ---
        "float" => match args.get(0) {
            Some(LispVal::Num(n)) => Ok(LispVal::Float(*n as f64)),
            Some(v) => Ok(crate::helpers::as_float(v)
                .map(LispVal::Float)
                .unwrap_or(LispVal::Float(0.0))),
            None => Ok(LispVal::Float(0.0)),
        },
        "integer" => match args.get(0) {
            Some(LispVal::Float(f)) => Ok(LispVal::Num(*f as i64)),
            Some(LispVal::Num(n)) => Ok(LispVal::Num(*n)),
            _ => Ok(LispVal::Num(0)),
        },
        "boolean" => Ok(LispVal::Bool(crate::helpers::is_truthy(
            args.get(0).unwrap_or(&LispVal::Nil),
        ))),
        // --- Error ---
        "error" => {
            let msg = match args.get(0) {
                Some(LispVal::Str(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => "error".to_string(),
            };
            Err(msg)
        }
        "apply" => {
            let (env_ref, state_ref) = match (env, state) {
                (Some(e), Some(s)) => (e, s),
                _ => return Err("apply: not available in loop context".into()),
            };
            if args.len() < 2 {
                return Err("apply: need (f ... arglist)".into());
            }
            let func = args[0].clone();
            let mut apply_args = args[1..args.len() - 1].to_vec();
            match args.last() {
                Some(LispVal::List(lst)) => apply_args.extend(lst.iter().cloned()),
                Some(LispVal::Nil) => {}
                _ => return Err("apply: last arg must be list".into()),
            }
            vm_call_lambda(&func, &apply_args, env_ref, state_ref)
        }
        "eval" => {
            let (env_ref, state_ref) = match (env, state) {
                (Some(e), Some(s)) => (e, s),
                _ => return Err("eval: not available in loop context".into()),
            };
            let datum = args.first().ok_or("eval: need 1 arg")?;
            let exprs = vec![datum.clone()];
            crate::program::run_program(&exprs, &mut env_ref.clone(), state_ref)
        }
        "doc" => {
            let name = match args.first() {
                Some(LispVal::Sym(s)) => s.to_string(),
                Some(LispVal::Str(s)) => s.to_string(),
                Some(v) => v.to_string(),
                None => return Err("doc: need 1 arg (function name)".into()),
            };
            match crate::helpers::get_doc(&name) {
                Some(d) => Ok(LispVal::Str(d.to_string())),
                None => {
                    let in_env = env.map(|e| e.get(&name).is_some()).unwrap_or(false);
                    if in_env {
                        Ok(LispVal::Str(format!("User-defined: {} (no doc)", name)))
                    } else {
                        Ok(LispVal::Str(format!("No documentation for '{}'", name)))
                    }
                }
            }
        }
        "macroexpand" => {
            let form = args.first().ok_or("macroexpand: need a form")?;
            if let LispVal::List(ref form_list) = form {
                if let Some(LispVal::Sym(ref sym_name)) = form_list.first() {
                    if let Some(env) = env {
                        if let Some(macro_val) = env.get(sym_name) {
                            if matches!(macro_val, LispVal::Macro { .. }) {
                                return expand_macro_call(&macro_val, &form_list[1..]);
                            }
                        }
                    }
                    return Err(format!("macroexpand: {} is not a macro", sym_name));
                }
            }
            Err("macroexpand: expected (macro-name args...) form".into())
        }
        _ => {
            // Intercept load-file: use run_program (VM) instead of lisp_eval (tree-walker)
            if name == "load-file" {
                if let (Some(e), Some(s)) = (env, state) {
                    let path = match args.get(0) {
                        Some(LispVal::Str(p)) => p.clone(),
                        _ => return Err("load-file: expected string path".into()),
                    };
                    let code =
                        std::fs::read_to_string(&path).map_err(|e| format!("load-file: {}", e))?;
                    let forms = crate::parser::parse_all(&code)
                        .map_err(|e| format!("load-file: parse error: {}", e))?;
                    let mut result = LispVal::Nil;
                    for form in &forms {
                        result = crate::program::run_program(
                            &[form.clone()],
                            e,
                            s,
                        )?;
                    }
                    return Ok(result);
                }
                return Err("load-file: no env/state available".into());
            }
            // --- Contract type checking builtins ---
            if name == "contract-check-param" {
                let param_name = match args.get(0) {
                    Some(LispVal::Str(s)) => s.as_str(),
                    _ => return Err("contract-check-param: expected param name string".into()),
                };
                let val = match args.get(1) {
                    Some(v) => v,
                    None => return Err("contract-check-param: expected value".into()),
                };
                let ty = match args.get(2) {
                    Some(t) => t,
                    None => return Err("contract-check-param: expected type spec".into()),
                };
                if !check_type(val, ty)? {
                    return Err(format!(
                        "contract violation: param '{}' expected {}, got {}",
                        param_name,
                        type_spec_name(ty),
                        val_type_name(val)
                    ));
                }
                return Ok(LispVal::Nil);
            }
            if name == "contract-check-return" {
                let val = match args.get(0) {
                    Some(v) => v,
                    None => return Err("contract-check-return: expected value".into()),
                };
                let ty = match args.get(1) {
                    Some(t) => t,
                    None => return Err("contract-check-return: expected type spec".into()),
                };
                if !check_type(val, ty)? {
                    return Err(format!(
                        "contract violation: return expected {}, got {}",
                        type_spec_name(ty),
                        val_type_name(val)
                    ));
                }
                return Ok(LispVal::Nil);
            }
            if name == "contract-wrap" {
                return Ok(args.get(0).cloned().unwrap_or(LispVal::Nil));
            }
            if name == "defschema" {
                let schema_name = match args.get(0) {
                    Some(LispVal::Sym(s)) => s.clone(),
                    Some(LispVal::Str(s)) => s.clone(),
                    _ => return Err("defschema: expected schema name as first arg".into()),
                };
                let mut fields: Vec<(String, LispVal)> = Vec::new();
                let mut strict = false;
                let mut i = 1;
                while i < args.len() {
                    let field_name = match &args[i] {
                        LispVal::Str(s) => s.clone(),
                        LispVal::Sym(s) => s.clone(),
                        _ => { i += 1; continue; }
                    };
                    if field_name == ":strict" { strict = true; i += 1; continue; }
                    if i + 1 < args.len() {
                        fields.push((field_name, args[i + 1].clone()));
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                let field_list: Vec<LispVal> = fields.iter()
                    .map(|(n, t)| LispVal::List(vec![LispVal::Str(n.clone()), t.clone()]))
                    .collect();
                // Store internally as Map with name, fields, and strict flag
                let schema_val = LispVal::Map(im::hashmap![
                    "name".to_string() => LispVal::Sym(schema_name.clone()),
                    "fields".to_string() => LispVal::List(field_list),
                    "strict".to_string() => LispVal::Bool(strict),
                ]);
                if let Some(e) = env { e.insert_mut(schema_name, schema_val); }
                return Ok(LispVal::Nil);
            }
            if name == "validate" {
                let data = match args.get(0) {
                    Some(LispVal::Map(m)) => m.clone(),
                    _ => return Err("validate: expected map as first arg".into()),
                };
                let schema_val = match args.get(1) {
                    // Direct schema map (e.g. when schema name resolved from env)
                    Some(LispVal::Map(m)) if m.contains_key("fields") => LispVal::Map(m.clone()),
                    Some(LispVal::Sym(s)) => {
                        match env.and_then(|e| e.get(s.as_str())) {
                            Some(v) => v.clone(),
                            None => return Err(format!("validate: unknown schema '{}'", s)),
                        }
                    }
                    Some(LispVal::Str(s)) => {
                        match env.and_then(|e| e.get(s.as_str())) {
                            Some(v) => v.clone(),
                            None => return Err(format!("validate: unknown schema '{}'", s)),
                        }
                    }
                    _ => return Err("validate: expected schema name or schema map as second arg".into()),
                };
                let schema_name_str = match &schema_val {
                    LispVal::Map(m) => m.get("name").and_then(|v| match v {
                        LispVal::Sym(s) => Some(s.clone()),
                        LispVal::Str(s) => Some(s.clone()),
                        _ => None,
                    }),
                    _ => None,
                };
                let fields_list = match &schema_val {
                    LispVal::Map(m) => m.get("fields").cloned(),
                    _ => None,
                };
                let is_strict = match &schema_val {
                    LispVal::Map(m) => matches!(m.get("strict"), Some(LispVal::Bool(true))),
                    _ => false,
                };
                match fields_list {
                    Some(LispVal::List(fields)) => {
                        for field in &fields {
                            let (fname, ftype) = match field {
                                LispVal::List(l) if l.len() >= 2 => {
                                    match (&l[0], &l[1]) {
                                        (LispVal::Str(n), t) => (n.as_str(), t),
                                        _ => continue,
                                    }
                                }
                                _ => continue,
                            };
                            match data.get(fname) {
                                None => return Err(format!("missing field '{}' in schema '{}'", fname, schema_name_str.as_deref().unwrap_or("?"))),
                                Some(v) => {
                                    if !check_type(v, ftype).unwrap_or(false) {
                                        return Err(format!("unexpected type for field '{}': expected {}, got {}", fname, type_spec_name(ftype), val_type_name(v)));
                                    }
                                }
                            }
                        }
                        if is_strict {
                            let required: std::collections::HashSet<String> = fields.iter()
                                .filter_map(|f| if let LispVal::List(l) = f {
                                    if let LispVal::Str(n) = &l[0] { Some(n.clone()) } else { None }
                                } else { None })
                                .collect();
                            for key in data.keys() {
                                if !required.contains(key) {
                                    return Err(format!("unexpected field '{}' in strict schema '{}'", key, schema_name_str.as_deref().unwrap_or("?")));
                                }
                            }
                        }
                        return Ok(LispVal::Map(data));
                    }
                    _ => return Err(format!("validate: invalid schema '{}'", schema_name_str.as_deref().unwrap_or("?"))),
                }
            }
            if name == "check" {
                let val = match args.get(0) { Some(v) => v, None => return Err("check: expected value".into()) };
                let ty = match args.get(1) { Some(t) => t, None => return Err("check: expected type spec".into()) };
                if !check_type(val, ty)? {
                    return Err(format!("type mismatch: expected {}, got {}", type_spec_name(ty), val_type_name(val)));
                }
                return Ok(val.clone());
            }
            if name == "check!" {
                let val = match args.get(0) { Some(v) => v, None => return Err("check!: expected value".into()) };
                let ty = match args.get(1) { Some(t) => t, None => return Err("check!: expected type spec".into()) };
                if !check_type(val, ty)? {
                    return Err(format!("type mismatch: expected {}, got {}", type_spec_name(ty), val_type_name(val)));
                }
                return Ok(val.clone());
            }
            if name == "matches?" {
                let val = match args.get(0) { Some(v) => v, None => return Ok(LispVal::Bool(false)) };
                let ty = match args.get(1) { Some(t) => t, None => return Ok(LispVal::Bool(false)) };
                return Ok(LispVal::Bool(check_type(val, ty).unwrap_or(false)));
            }
            if name == "valid-type?" {
                let ty = match args.get(0) { Some(t) => t, None => return Ok(LispVal::Bool(false)) };
                if is_valid_type_spec(ty) {
                    return Ok(LispVal::Str(type_spec_name(ty)));
                }
                return Ok(LispVal::Bool(false));
            }
            if name == "type-of" {
                let val = match args.get(0) { Some(v) => v, None => return Ok(LispVal::Sym(":any".into())) };
                return Ok(LispVal::Sym(val_type_name(val)));
            }
            if name == "schema" {
                // Accept either a schema name (Sym/Str) or a direct schema Map
                let (schema_map, schema_name_hint) = match args.get(0) {
                    Some(LispVal::Map(m)) if m.contains_key("fields") => {
                        let hint = m.get("name").and_then(|v| match v {
                            LispVal::Sym(s) => Some(s.clone()),
                            LispVal::Str(s) => Some(s.clone()),
                            _ => None,
                        });
                        (m.clone(), hint)
                    }
                    Some(LispVal::Sym(s)) => {
                        let key = s.as_str();
                        match env.and_then(|e| e.get(key)) {
                            Some(LispVal::Map(m)) => (m.clone(), Some(key.to_string())),
                            Some(v) => return Ok(v.clone()),
                            None => return Err(format!("schema: unknown schema '{}'", key)),
                        }
                    }
                    Some(LispVal::Str(s)) => {
                        let key = s.as_str();
                        match env.and_then(|e| e.get(key)) {
                            Some(LispVal::Map(m)) => (m.clone(), Some(key.to_string())),
                            Some(v) => return Ok(v.clone()),
                            None => return Err(format!("schema: unknown schema '{}'", key)),
                        }
                    }
                    _ => return Err("schema: expected schema name or schema map".into()),
                };
                let name_val = schema_map.get("name").cloned()
                    .unwrap_or_else(|| LispVal::Sym(schema_name_hint.unwrap_or_else(|| "?".into())));
                let fields_val = schema_map.get("fields").cloned().unwrap_or(LispVal::List(vec![]));
                let strict_val = schema_map.get("strict").cloned().unwrap_or(LispVal::Bool(false));
                return Ok(LispVal::List(vec![name_val, fields_val, strict_val]));
            }
            if name == "mark-pure" {
                // Called by the `pure` compiler special form after a define.
                // arg[0] is the function name (as a string).
                // Looks up the function in env, infers type from bytecode, stores in state.
                let func_name = match args.get(0) {
                    Some(LispVal::Str(s)) => s.clone(),
                    _ => return Ok(LispVal::Nil),
                };
                if let (Some(e), Some(s)) = (env, state) {
                    if let Some(func_val) = e.get(&func_name) {
                        let type_str = infer_type_from_val(&func_val);
                        s.pure_types.insert(func_name, type_str);
                    }
                }
                return Ok(LispVal::Nil);
            }
            if name == "pure" {
                let val = match args.get(0) {
                    Some(v) => v,
                    None => return Err("pure: expected expression".into()),
                };
                return Ok(val.clone());
            }
            if name == "pure-type" {
                if let Some(s) = state {
                    let func_name = match args.get(0) {
                        Some(LispVal::Sym(n)) => n.clone(),
                        Some(LispVal::Str(n)) => n.clone(),
                        _ => return Ok(LispVal::Nil),
                    };
                    if let Some(type_str) = s.pure_types.get(&func_name) {
                        return Ok(LispVal::Str(type_str.clone()));
                    }
                }
                return Ok(LispVal::Nil);
            }
            if name == "infer-type" {
                let val = match args.get(0) {
                    Some(v) if matches!(v, LispVal::Lambda { .. } | LispVal::BuiltinFn(_) | LispVal::CaseLambda { .. } | LispVal::Memoized { .. }) => {
                        infer_type_from_val(v)
                    }
                    _ => return Err("infer-type: expected a function".into()),
                };
                return Ok(LispVal::Str(val));
            }
            if let (Some(e), Some(s)) = (env, state) {
                // dispatch_collections needs env for user-function calls (HOFs).
                // Pass the REAL env (not a clone) so that set! mutations inside
                // HOF lambdas (e.g., for-each, map, filter) are visible to the caller.
                match crate::dispatch::dispatch_collections::handle(name, args, e, s) {
                    Ok(Some(result)) => return Ok(result),
                    Err(e) => return Err(e),
                    Ok(None) => {}
                }
                // Subsequent dispatch modules get a clone — mutations here are
                // not expected to propagate back to the caller's env.
                let mut env_clone = e.clone();
                // dispatch_state needs env and state
                #[cfg(not(target_arch = "wasm32"))]
                match crate::dispatch::dispatch_state::handle(name, args, &mut env_clone, s) {
                    Ok(Some(result)) => return Ok(result),
                    Err(e) => return Err(e),
                    Ok(None) => {}
                }
            }
            if let Ok(Some(result)) = crate::dispatch::dispatch_arithmetic::handle(name, args) {
                return Ok(result);
            }
            if let Ok(Some(result)) = crate::dispatch::dispatch_strings::handle(name, args) {
                return Ok(result);
            }
            if let Ok(Some(result)) = crate::dispatch::dispatch_predicates::handle(name, args) {
                return Ok(result);
            }
            #[cfg(not(target_arch = "wasm32"))]
            if let Ok(Some(result)) = crate::dispatch::dispatch_json::handle(name, args) {
                return Ok(result);
            }
            #[cfg(not(target_arch = "wasm32"))]
            if let Ok(Some(result)) = crate::dispatch::dispatch_http::handle(name, args) {
                return Ok(result);
            }
            if let Ok(Some(result)) = crate::dispatch::dispatch_types::handle(name, args) {
                return Ok(result);
            }

            let args_str: Vec<String> = args.iter().map(|a| {
                let s = a.to_string();
                if s.len() > 40 { format!("{}...", &s[..37]) } else { s }
            }).collect();
            Err(format!("unknown builtin '{}' with args: ({})", name, args_str.join(" ")))
        }
    }
}

/// Compiled lambda: a flat bytecode program with N param slots + captured env slots.
/// Used for fast-path map/filter/reduce — avoids env push/pop per element.
#[derive(Debug)]
pub struct CompiledLambda {
    /// Optional function name for stack traces
    pub name: Option<String>,
    pub num_param_slots: usize,
    /// If this lambda is variadic, the slot index for the &rest parameter.
    /// All args beyond num_fixed_params are packed into a list at this slot.
    pub rest_param_idx: Option<usize>,
    /// Number of fixed (non-rest) parameters
    pub num_fixed_params: usize,
    pub total_slots: usize,
    pub code: Vec<Op>,
    /// Captured environment variables. Wrapped in RwLock for interior mutability —
    /// StoreCaptured needs to persist mutations across calls (counter pattern).
    /// RwLock (not RwLock) because CompiledLambda is shared across threads via Arc.
    pub captured: std::sync::RwLock<Vec<(String, LispVal)>>,
    /// Pre-compiled inner lambdas (closures). Indexed by PushClosure(N).
    pub closures: Vec<CompiledLambda>,
    /// Outer slot indices that must be captured at runtime (from caller's slots array).
    /// Paired with names in order — at PushClosure time, read slots[i] for each entry
    /// and add to the closure's captured list.
    pub runtime_captures: Vec<(String, usize)>,
}

impl Clone for CompiledLambda {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            num_param_slots: self.num_param_slots,
            rest_param_idx: self.rest_param_idx,
            num_fixed_params: self.num_fixed_params,
            total_slots: self.total_slots,
            code: self.code.clone(),
            captured: std::sync::RwLock::new(self.captured.read().unwrap().clone()),
            closures: self.closures.clone(),
            runtime_captures: self.runtime_captures.clone(),
        }
    }
}

/// Try to compile a lambda body for fast inline evaluation.
/// Returns None if the body contains unsupported forms (macros, user-defined functions, etc.)
pub fn try_compile_lambda(
    param_names: &[String],
    body: &LispVal,
    _closed_env: &[(String, LispVal)],
    outer_env: &Env,
    func_name: Option<&str>,
    pure_type: Option<&str>,
) -> Option<CompiledLambda> {
    let mut compiler = LoopCompiler::new(param_names.to_vec());
    compiler.self_name = func_name.map(|s| s.to_string());
    // If the body is a lambda, set pending_lambda_name so the inner compiler
    // picks it up and enables CallSelf for recursive calls.
    if func_name.is_some() {
        if let LispVal::List(ref l) = body {
            if !l.is_empty() {
                if let LispVal::Sym(ref s) = l[0] {
                    if s == "lambda" || s == "fn" {
                        compiler.pending_lambda_name = func_name.map(|s| s.to_string());
                    }
                }
            }
        }
    }

    // Parse pure_type to mark parameter slots as i64.
    // Format: "int -> int", "int -> int -> int", etc.
    // Mark params as i64 where the corresponding arrow-input is "int".
    if let Some(pt) = pure_type {
        let parts: Vec<&str> = pt.split("->").map(|s| s.trim()).collect();
        // All parts except the last are inputs. If input is "int", mark param.
        for (i, part) in parts.iter().enumerate() {
            if i >= param_names.len() {
                break;
            }
            // Only mark inputs (all but last segment)
            if i < parts.len() - 1 && *part == "int" {
                compiler.mark_slot_i64(i);
            }
        }
    }
    // Don't pre-populate captured — try_capture will pull in only what's needed from outer_env.
    // closed_env contains the ENTIRE scope snapshot (all builtins, etc) — most are unused.
    if !compiler.compile_expr(body, outer_env) {
        return None;
    }
    compiler.code.push(Op::Return);
    let mut code = compiler.code;
    let slot_i64 = compiler.slot_is_i64;
    let slot_f64 = compiler.slot_is_f64;
    peephole_optimize(&mut code, &slot_i64, &slot_f64);
    peephole_optimize(&mut code, &slot_i64, &slot_f64);
    peephole_optimize(&mut code, &slot_i64, &slot_f64);
    // Compute total slots: params + any let-binding slots used in code
    // Captured vars are accessed via LoadCaptured/CallCapturedRef, not slots
    let base_slots = param_names.len();
    let mut max_slot = base_slots;
    for op in &code {
        match op {
            Op::StoreSlot(s) | Op::LoadSlot(s) | Op::StoreAndLoadSlot(s) => max_slot = max_slot.max(*s + 1),
            Op::SlotAddImm(s, _)
            | Op::SlotSubImm(s, _)
            | Op::SlotMulImm(s, _)
            | Op::SlotDivImm(s, _) => {
                max_slot = max_slot.max(*s + 1);
            }
            Op::SlotEqImm(s, _)
            | Op::SlotLtImm(s, _)
            | Op::SlotLeImm(s, _)
            | Op::SlotGtImm(s, _)
            | Op::SlotGeImm(s, _) => {
                max_slot = max_slot.max(*s + 1);
            }
            Op::JumpIfSlotLtImm(s, _, _)
            | Op::JumpIfSlotLeImm(s, _, _)
            | Op::JumpIfSlotGtImm(s, _, _)
            | Op::JumpIfSlotGeImm(s, _, _)
            | Op::JumpIfSlotEqImm(s, _, _) => {
                max_slot = max_slot.max(*s + 1);
            }
            Op::RecurIncAccum(a, b, _, _, _) => max_slot = max_slot.max(*a.max(b) + 1),
            Op::CallCaptured(s, _) => max_slot = max_slot.max(*s + 1),
            _ => {}
        }
    }
    Some(CompiledLambda {
        name: func_name.map(|s| s.to_string()),
        num_param_slots: param_names.len(),
        total_slots: max_slot,
        code,
        captured: std::sync::RwLock::new(compiler.captured),
        closures: compiler.closures,
        runtime_captures: compiler.runtime_captures,
        rest_param_idx: None,
        num_fixed_params: param_names.len(),
    })
}

/// Call a LispVal as a function through the VM path only.
/// Handles compiled lambdas and BuiltinFn. Returns Err for uncallable values.
pub fn vm_call_lambda(
    func: &LispVal,
    args: &[LispVal],
    outer_env: &mut Env,
    state: &mut EvalState,
) -> Result<LispVal, String> {
    match func {
        LispVal::Lambda {
            compiled: Some(ref cl),
            ..
        } => run_compiled_lambda(cl, args, outer_env, state),
        LispVal::BuiltinFn(name) => eval_builtin(name, args, Some(outer_env), Some(state)),
        LispVal::CaseLambda { clauses, .. } => {
            let n = args.len();
            // Try exact match on fixed-param clauses first, then fall back to rest-param
            let mut matched: Option<&LispVal> = None;
            for (n_fixed, has_rest, lambda) in clauses.iter() {
                if *has_rest {
                    // Rest-param clause: matches any arg count — keep as fallback
                    matched = Some(lambda);
                } else if *n_fixed == n {
                    // Exact match on fixed params
                    matched = Some(lambda);
                    break;
                }
            }
            match matched {
                Some(lambda) => vm_call_lambda(lambda, args, outer_env, state),
                None => Err(format!("case-lambda: no clause matches {} arguments", n)),
            }
        }
        LispVal::Memoized { func, cache } => {
            // Build cache key from args
            let key: String = args.iter().map(|a| format!("{:?}", a)).collect();
            // Check cache
            if let Ok(cached) = cache.read() {
                if let Some(result) = cached.get(&key) {
                    return Ok(result.clone());
                }
            }
            // Cache miss — call the wrapped function
            let result = vm_call_lambda(func, args, outer_env, state)?;
            // Store in cache
            if let Ok(mut cached) = cache.write() {
                cached.insert(key, result.clone());
            }
            Ok(result)
        }
        _ => Err(format!(
            "cannot call {} as a function (expected a lambda or builtin)",
            func
        )),
    }
}

/// Generate a short human-readable summary of a CompiledLambda for stack traces.
fn summarize_compiled_lambda(cl: &CompiledLambda) -> String {
    let n_params = cl.num_fixed_params;
    let mut ops_summary = String::new();
    for op in cl.code.iter().take(3) {
        if !ops_summary.is_empty() { ops_summary.push(' '); }
        match op {
            Op::BuiltinCall(name, _) => ops_summary.push_str(name),
            Op::Add => ops_summary.push('+'),
            Op::Sub => ops_summary.push('-'),
            Op::Mul => ops_summary.push('*'),
            Op::Div => ops_summary.push('/'),
            Op::PushI64(n) => { ops_summary.push_str(&n.to_string()); }
            Op::LoadSlot(_) => ops_summary.push('_'),
            _ => ops_summary.push('.'),
        }
    }
    format!("(fn [{}] {})", 
        (0..n_params).map(|i| format!("p{}", i)).collect::<Vec<_>>().join(" "),
        if ops_summary.is_empty() { "..." } else { &ops_summary }
    )
}

// ---------------------------------------------------------------------------
// Test helpers: construct CompiledLoop / CompiledLambda for unit tests
// ---------------------------------------------------------------------------

pub fn make_test_compiled_loop(
    init_vals: Vec<LispVal>,
    code: Vec<Op>,
    captured: Vec<(String, LispVal)>,
) -> CompiledLoop {
    let num_slots = init_vals.len();
    CompiledLoop {
        num_slots,
        slot_names: (0..num_slots).map(|i| format!("s{}", i)).collect(),
        init_vals,
        code,
        loop_start_pc: 0,
        captured,
    }
}

pub fn make_test_compiled_lambda(
    num_fixed_params: usize,
    total_slots: usize,
    code: Vec<Op>,
) -> CompiledLambda {
    CompiledLambda {
        name: None,
        num_param_slots: num_fixed_params,
        rest_param_idx: None,
        num_fixed_params,
        total_slots,
        code,
        captured: std::sync::RwLock::new(vec![]),
        closures: vec![],
        runtime_captures: vec![],
    }
}

pub fn run_compiled_loop_test(cl: &CompiledLoop) -> Result<LispVal, String> {
    run_compiled_loop(cl)
}

pub fn run_lambda_test(cl: &CompiledLambda, args: &[LispVal]) -> Result<LispVal, String> {
    run_compiled_lambda(cl, args, &mut crate::types::Env::new(), &mut crate::types::EvalState::new())
}

/// Validate that all slot indices in the bytecode are within bounds.
/// Returns an error string describing the first OOB slot found.
/// Used by both the differential fuzz tests and as a defensive pre-flight check.
pub fn validate_slot_indices(code: &[Op], slots_len: usize) -> Result<(), String> {
    for op in code {
        match op {
            Op::LoadSlot(s)
            | Op::StoreSlot(s)
            | Op::ReturnSlot(s)
            | Op::StoreAndLoadSlot(s)
            | Op::DictMutSet(s)
            | Op::RecurDirect(s) => {
                if *s >= slots_len {
                    return Err(format!(
                        "slot index {} out of bounds (slots_len={})",
                        s, slots_len
                    ));
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
                    return Err(format!(
                        "slot index {} out of bounds (slots_len={})",
                        s, slots_len
                    ));
                }
            }
            Op::JumpIfSlotLtImm(s, _, _)
            | Op::JumpIfSlotLeImm(s, _, _)
            | Op::JumpIfSlotGtImm(s, _, _)
            | Op::JumpIfSlotGeImm(s, _, _)
            | Op::JumpIfSlotEqImm(s, _, _) => {
                if *s >= slots_len {
                    return Err(format!(
                        "slot index {} out of bounds (slots_len={})",
                        s, slots_len
                    ));
                }
            }
            Op::RecurIncAccum(counter, accum, _, _, _) => {
                if *counter >= slots_len {
                    return Err(format!(
                        "RecurIncAccum counter slot {} out of bounds (slots_len={})",
                        counter, slots_len
                    ));
                }
                if *accum >= slots_len {
                    return Err(format!(
                        "RecurIncAccum accum slot {} out of bounds (slots_len={})",
                        accum, slots_len
                    ));
                }
            }
            Op::GetDefaultSlot(a, b, c, d) => {
                for &(name, idx) in &[("map", *a), ("key", *b), ("default", *c), ("result", *d)] {
                    if idx >= slots_len {
                        return Err(format!(
                            "GetDefaultSlot {} slot {} out of bounds (slots_len={})",
                            name, idx, slots_len
                        ));
                    }
                }
            }
            Op::Recur(n) => {
                if *n > slots_len {
                    return Err(format!(
                        "Recur({}) requires {} slots but only {} available",
                        n, n, slots_len
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Run a compiled lambda with the given arguments. Returns the result directly.
pub fn run_compiled_lambda(
    cl: &CompiledLambda,
    args: &[LispVal],
    outer_env: &mut Env,
    state: &mut EvalState,
) -> Result<LispVal, String> {
    // Stack overflow protection: check call depth before recursing
    state.call_depth += 1;
    if state.call_depth > state.max_call_depth {
        state.call_depth -= 1;
        return Err(format!(
            "call depth exceeded (max {})",
            state.max_call_depth
        ));
    }
    let fname = cl.name.as_deref().unwrap_or_else(|| {
        // Generate a hint from the first few ops: e.g. "<(fn [x] ...)>"
        let hint = summarize_compiled_lambda(cl);
        // Leak is fine — these strings are small and live for the process lifetime
        Box::leak(hint.into_boxed_str())
    });
    state.trace_push(fname);
    let result = run_compiled_lambda_inner(cl, args, outer_env, state);
    state.call_depth -= 1;
    match result {
        Err(e) => {
            let trace = state.format_trace();
            state.trace_pop();
            Err(format!("{}\n{}", e, trace))
        }
        ok => {
            state.trace_pop();
            ok
        }
    }
}

fn run_compiled_lambda_inner(
    cl: &CompiledLambda,
    args: &[LispVal],
    outer_env: &mut Env,
    state: &mut EvalState,
) -> Result<LispVal, String> {
    let mut slots: Vec<LispVal> = vec![LispVal::Nil; cl.total_slots];
    // Fill param slots from args
    let n_fixed = cl.num_fixed_params;
    for i in 0..cl.num_param_slots.min(args.len()) {
        slots[i] = args[i].clone();
    }
    // Pack rest args into a list if variadic
    if let Some(rest_idx) = cl.rest_param_idx {
        let rest_list = LispVal::List(args[n_fixed..].to_vec());
        slots[rest_idx] = rest_list;
    }
    let mut stack: Vec<LispVal> = Vec::with_capacity(8);
    // Frame stack for iterative CallSelf — avoids recursive run_compiled_lambda calls
    struct Frame {
        pc: usize,
        slots: Vec<LispVal>,
        stack: Vec<LispVal>,
    }
    let mut frames: Vec<Frame> = Vec::new();
    let code = &cl.code;
    let mut pc: usize = 0;
    let mut ops: u32 = 0;
    // Use eval_budget from state (0 = unlimited), fallback to 10M
    let lambda_budget = if state.eval_budget > 0 { state.eval_budget as u32 } else { 10_000_000 };

    loop {
        ops += 1;
        state.eval_count += 1;
        // Also check CPS-level budget (non-zero means limited)
        if state.eval_budget > 0 && state.eval_count > state.eval_budget {
            return Err(format!(
                "execution budget exceeded ({} iterations, limit: {})",
                state.eval_count, state.eval_budget
            ));
        }
        if ops > lambda_budget {
            return Err("compiled lambda: budget exceeded (possible infinite loop)".into());
        }
        if pc >= code.len() {
            return Err(format!(
                "compiled lambda: pc {} out of bounds (code len {}, ops {})",
                pc,
                code.len(),
                ops
            ));
        }
        match &code[pc] {
            Op::LoadSlot(s) => {
                let slot_ref = safe_slot(&slots, *s);
                match slot_ref {
                    LispVal::Num(n) => stack.push(LispVal::Num(*n)),
                    _ => stack.push(slot_ref.clone()),
                }
                pc += 1;
            }
            Op::LoadCaptured(idx) => {
                stack.push(cl.captured.read().unwrap()[*idx].1.clone());
                pc += 1;
            }
            Op::StoreCaptured(idx) => {
                let val = stack.pop().unwrap_or(LispVal::Nil);
                cl.captured.write().unwrap()[*idx].1 = val.clone();
                stack.push(val);
                pc += 1;
            }
            Op::LoadGlobal(name) => {
                match outer_env.get(name) {
                    Some(val) => stack.push(val.clone()),
                    None => return Err(format!("LoadGlobal: undefined {}", name)),
                }
                pc += 1;
            }
            Op::StoreGlobal(name) => {
                let val = stack.pop().unwrap_or(LispVal::Nil);
                outer_env.insert_mut(name.clone(), val.clone());
                stack.push(val);
                pc += 1;
            }
            Op::PushI64(n) => {
                stack.push(LispVal::Num(*n));
                pc += 1;
            }
            Op::PushFloat(f) => {
                stack.push(LispVal::Float(*f));
                pc += 1;
            }
            Op::PushBool(b) => {
                stack.push(LispVal::Bool(*b));
                pc += 1;
            }
            Op::PushStr(s) => {
                stack.push(LispVal::Str(s.clone()));
                pc += 1;
            }
            Op::PushNil => {
                stack.push(LispVal::Nil);
                pc += 1;
            }
            Op::MakeList(n) => {
                let mut items = Vec::with_capacity(*n);
                for _ in 0..*n {
                    items.push(stack.pop().unwrap_or(LispVal::Nil));
                }
                items.reverse();
                stack.push(LispVal::List(items));
                pc += 1;
            }
            Op::Add => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(num_arith_checked(&a, &b, "add", i64::checked_add, |x, y| x + y)?);
                pc += 1;
            }
            Op::Sub => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(num_arith_checked(&a, &b, "sub", i64::checked_sub, |x, y| x - y)?);
                pc += 1;
            }
            Op::Mul => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(num_arith_checked(&a, &b, "mul", i64::checked_mul, |x, y| x * y)?);
                pc += 1;
            }
            Op::Div => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                check_float_zero(&a, &b, "division")?;
                stack.push(num_arith_checked(&a, &b, "div", i64::checked_div, |x, y| x / y)?);
                pc += 1;
            }
            Op::Mod => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                check_float_zero(&a, &b, "modulo")?;
                stack.push(num_arith_checked(&a, &b, "mod", i64::checked_rem, |x, y| x % y)?);
                pc += 1;
            }
            Op::Eq => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(lisp_eq(&a, &b)));
                pc += 1;
            }
            Op::Lt => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(num_cmp(&a, &b, |x, y| x < y, |x, y| x < y)));
                pc += 1;
            }
            Op::Le => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(num_cmp(&a, &b, |x, y| x <= y, |x, y| x <= y)));
                pc += 1;
            }
            Op::Gt => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(num_cmp(&a, &b, |x, y| x > y, |x, y| x > y)));
                pc += 1;
            }
            Op::Ge => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(num_cmp(&a, &b, |x, y| x >= y, |x, y| x >= y)));
                pc += 1;
            }
            Op::Not => {
                let v = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(!is_truthy(&v)));
                pc += 1;
            }
            // Typed binary ops — zero dynamic dispatch
            Op::TypedBinOp(op, ty) => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                match ty {
                    Ty::I64 => {
                        let av = match &a {
                            LispVal::Num(n) => *n,
                            _ => 0,
                        };
                        let bv = match &b {
                            LispVal::Num(n) => *n,
                            _ => 0,
                        };
                        stack.push(match op {
                            BinOp::Add => {
                                LispVal::Num(i64::checked_add(av, bv)
                                    .ok_or("integer overflow in add")?)
                            }
                            BinOp::Sub => {
                                LispVal::Num(i64::checked_sub(av, bv)
                                    .ok_or("integer overflow in sub")?)
                            }
                            BinOp::Mul => {
                                LispVal::Num(i64::checked_mul(av, bv)
                                    .ok_or("integer overflow in mul")?)
                            }
                            BinOp::Div => {
                                LispVal::Num(i64::checked_div(av, bv)
                                    .ok_or("integer overflow in div")?)
                            }
                            BinOp::Mod => {
                                LispVal::Num(i64::checked_rem(av, bv)
                                    .ok_or("integer overflow in mod")?)
                            }
                            BinOp::Lt => LispVal::Bool(av < bv),
                            BinOp::Le => LispVal::Bool(av <= bv),
                            BinOp::Gt => LispVal::Bool(av > bv),
                            BinOp::Ge => LispVal::Bool(av >= bv),
                            BinOp::Eq => LispVal::Bool(av == bv),
                        });
                    }
                    Ty::F64 => {
                        let av = match &a {
                            LispVal::Float(f) => *f,
                            LispVal::Num(n) => *n as f64,
                            _ => 0.0,
                        };
                        let bv = match &b {
                            LispVal::Float(f) => *f,
                            LispVal::Num(n) => *n as f64,
                            _ => 0.0,
                        };
                        stack.push(match op {
                            BinOp::Add => LispVal::Float(av + bv),
                            BinOp::Sub => LispVal::Float(av - bv),
                            BinOp::Mul => LispVal::Float(av * bv),
                            BinOp::Div => LispVal::Float(av / bv),
                            BinOp::Mod => LispVal::Float(av % bv),
                            BinOp::Lt => LispVal::Bool(av < bv),
                            BinOp::Le => LispVal::Bool(av <= bv),
                            BinOp::Gt => LispVal::Bool(av > bv),
                            BinOp::Ge => LispVal::Bool(av >= bv),
                            BinOp::Eq => LispVal::Bool(av == bv),
                        });
                    }
                }
                pc += 1;
            }
            Op::SlotAddImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                match i64::checked_add(v, *imm) {
                    Some(result) => {
                        stack.push(LispVal::Num(result));
                        pc += 1;
                    }
                    None => return Err("integer overflow in add".into()),
                }
            }
            Op::SlotSubImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                match i64::checked_sub(v, *imm) {
                    Some(result) => {
                        stack.push(LispVal::Num(result));
                        pc += 1;
                    }
                    None => return Err("integer overflow in sub".into()),
                }
            }
            Op::SlotMulImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                match i64::checked_mul(v, *imm) {
                    Some(result) => {
                        stack.push(LispVal::Num(result));
                        pc += 1;
                    }
                    None => return Err("integer overflow in mul".into()),
                }
            }
            Op::SlotDivImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                match i64::checked_div(v, *imm) {
                    Some(result) => {
                        stack.push(LispVal::Num(result));
                        pc += 1;
                    }
                    None => return Err("integer overflow in div".into()),
                }
            }
            Op::SlotEqImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                stack.push(LispVal::Bool(v == *imm));
                pc += 1;
            }
            Op::SlotLtImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                stack.push(LispVal::Bool(v < *imm));
                pc += 1;
            }
            Op::SlotLeImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                stack.push(LispVal::Bool(v <= *imm));
                pc += 1;
            }
            Op::SlotGtImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                stack.push(LispVal::Bool(v > *imm));
                pc += 1;
            }
            Op::SlotGeImm(s, imm) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                stack.push(LispVal::Bool(v >= *imm));
                pc += 1;
            }
            Op::JumpIfSlotLtImm(s, imm, addr) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                if v < *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfSlotLeImm(s, imm, addr) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                if v <= *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfSlotGtImm(s, imm, addr) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                if v > *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfSlotGeImm(s, imm, addr) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                if v >= *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfSlotEqImm(s, imm, addr) => {
                let v = num_val_ref(safe_slot(&slots, *s));
                if v == *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::RecurIncAccum(counter, accum, step, limit, exit_addr) => {
                let cv = num_val_ref(safe_slot(&slots, *counter));
                if cv >= *limit {
                    pc = *exit_addr;
                } else {
                    let av = num_val_ref(safe_slot(&slots, *accum));
                    let new_accum = match i64::checked_add(av, cv) {
                        Some(r) => r,
                        None => return Err("integer overflow in add".into()),
                    };
                    let new_counter = match i64::checked_add(cv, *step) {
                        Some(r) => r,
                        None => return Err("integer overflow in add".into()),
                    };
                    while slots.len() <= *accum {
                        slots.push(LispVal::Nil);
                    }
                    while slots.len() <= *counter {
                        slots.push(LispVal::Nil);
                    }
                    slots[*accum] = LispVal::Num(new_accum);
                    slots[*counter] = LispVal::Num(new_counter);
                    pc = 0;
                }
            }
            Op::BuiltinCall(name, n_args) => {
                let mut bargs: Vec<LispVal> = Vec::with_capacity(*n_args);
                for _ in 0..*n_args {
                    bargs.push(stack.pop().unwrap_or(LispVal::Nil));
                }
                bargs.reverse();
                // Inline HOF handling — map/filter/for-each/reduce/sort need to call lambdas
                match name.as_str() {
                    "map" if bargs.len() == 2 => {
                        let func = &bargs[0];
                        let list = &bargs[1];
                        let vals = match list {
                            LispVal::List(l) => l,
                            LispVal::Nil => {
                                stack.push(LispVal::Nil);
                                pc += 1;
                                continue;
                            }
                            _ => {
                                stack.push(LispVal::Nil);
                                pc += 1;
                                continue;
                            }
                        };
                        let mut result = Vec::with_capacity(vals.len());
                        for v in vals.iter() {
                            result.push(vm_call_lambda(func, &[v.clone()], outer_env, state)?);
                        }
                        stack.push(LispVal::List(result));
                    }
                    "filter" if bargs.len() == 2 => {
                        let func = &bargs[0];
                        let list = &bargs[1];
                        let vals = match list {
                            LispVal::List(l) => l,
                            _ => {
                                stack.push(LispVal::Nil);
                                pc += 1;
                                continue;
                            }
                        };
                        let mut result = Vec::new();
                        for v in vals.iter() {
                            let keep = match vm_call_lambda(func, &[v.clone()], outer_env, state) {
                                Ok(LispVal::Bool(b)) => b,
                                Ok(_) => true,
                                Err(_) => false,
                            };
                            if keep {
                                result.push(v.clone());
                            }
                        }
                        stack.push(LispVal::List(result));
                    }
                    "for-each" if bargs.len() == 2 => {
                        let func = &bargs[0];
                        let list = &bargs[1];
                        let vals = match list {
                            LispVal::List(l) => l,
                            _ => {
                                stack.push(LispVal::Nil);
                                pc += 1;
                                continue;
                            }
                        };
                        for v in vals.iter() {
                            let _ = vm_call_lambda(func, &[v.clone()], outer_env, state);
                        }
                        stack.push(LispVal::Nil);
                    }
                    "sort" if bargs.len() == 2 => {
                        let comparator = bargs[0].clone();
                        let mut vals = match &bargs[1] {
                            LispVal::List(l) => l.clone(),
                            LispVal::Nil => vec![],
                            _ => {
                                stack.push(bargs[1].clone());
                                pc += 1;
                                continue;
                            }
                        };
                        let func = comparator.clone();
                        vals.sort_by(|a, b| {
                            match vm_call_lambda(
                                &func,
                                &[a.clone(), b.clone()],
                                outer_env,
                                state,
                            ) {
                                Ok(LispVal::Bool(true)) => std::cmp::Ordering::Less,
                                Ok(LispVal::Bool(false)) => std::cmp::Ordering::Greater,
                                _ => std::cmp::Ordering::Equal,
                            }
                        });
                        stack.push(LispVal::List(vals));
                    }
                    "reduce" if bargs.len() == 3 => {
                        let func = &bargs[0];
                        let init = bargs[1].clone();
                        let list = &bargs[2];
                        let vals = match list {
                            LispVal::List(l) => l,
                            _ => {
                                stack.push(init);
                                pc += 1;
                                continue;
                            }
                        };
                        let mut acc = init;
                        for v in vals.iter() {
                            acc = match vm_call_lambda(func, &[acc.clone(), v.clone()], outer_env, state) {
                                Ok(r) => r,
                                Err(_) => acc,
                            };
                        }
                        stack.push(acc);
                    }
                    _ => {
                        // Regular builtin — no lambda args needed
                        let result = eval_builtin(name, &bargs, Some(outer_env), Some(state))?;
                        stack.push(result);
                    }
                }
                pc += 1;
            }
            Op::CallCaptured(slot, n_args) => {
                let slot_ref = &slots[*slot];
                let mut cargs: Vec<LispVal> = Vec::with_capacity(*n_args);
                for _ in 0..*n_args {
                    cargs.push(stack.pop().unwrap_or(LispVal::Nil));
                }
                cargs.reverse();
                match vm_call_lambda(slot_ref, &cargs, outer_env, state) {
                    Ok(v) => stack.push(v),
                    Err(e) => return Err(e),
                }
                pc += 1;
            }
            Op::CallCapturedRef(idx, n_args) => {
                let captured_ref = cl.captured.read().unwrap()[*idx].1.clone();
                let mut cargs: Vec<LispVal> = Vec::with_capacity(*n_args);
                for _ in 0..*n_args {
                    cargs.push(stack.pop().unwrap_or(LispVal::Nil));
                }
                cargs.reverse();
                match vm_call_lambda(&captured_ref, &cargs, outer_env, state) {
                    Ok(v) => stack.push(v),
                    Err(e) => return Err(e),
                }
                pc += 1;
            }
            Op::PushClosure(idx) => {
                let inner = &cl.closures[*idx];
                let closed_env = {
                    let mut map = im::HashMap::new();
                    for (name, val) in inner.captured.read().unwrap().iter() {
                        map.insert(name.clone(), val.clone());
                    }
                    // Runtime captures: read values from current slots array
                    for (name, slot_idx) in &inner.runtime_captures {
                        let val = if *slot_idx < slots.len() {
                            slots[*slot_idx].clone()
                        } else {
                            LispVal::Nil
                        };
                        map.insert(name.clone(), val);
                    }
                    std::sync::Arc::new(std::sync::RwLock::new(map))
                };
                // Build the inner lambda's captured list for the compiled path
                let mut inner_cloned = inner.clone();
                // Merge runtime captures into captured list so the compiled inner lambda
                // can find them via captured_idx
                for (name, slot_idx) in &inner.runtime_captures {
                    let val = if *slot_idx < slots.len() {
                        slots[*slot_idx].clone()
                    } else {
                        LispVal::Nil
                    };
                    if inner_cloned.captured.read().unwrap().iter().all(|(n, _)| n != name) {
                        inner_cloned.captured.write().unwrap().push((name.clone(), val));
                    } else {
                        // Update existing captured value with runtime value
                        if let Some(entry) =
                            inner_cloned.captured.write().unwrap().iter_mut().find(|(n, _)| n == name)
                        {
                            entry.1 = val;
                        }
                    }
                }
                // Recompute total_slots to account for any new captured entries
                let captured_start = inner_cloned.num_param_slots;
                let needed = captured_start + inner_cloned.captured.read().unwrap().len();
                if needed > inner_cloned.total_slots {
                    inner_cloned.total_slots = needed;
                }
                // params from the closure's CompiledLambda
                let param_count = inner_cloned.num_param_slots;
                let param_names: Vec<String> =
                    (0..param_count).map(|i| format!("p{}", i)).collect();
                // Extract rest_param name from CompiledLambda's rest_param_idx
                let rest_param = inner_cloned.rest_param_idx.map(|idx| format!("p{}", idx));
                stack.push(LispVal::Lambda {
                    params: param_names,
                    rest_param,
                    body: Box::new(LispVal::Nil),
                    closed_env,
                    pure_type: None,
                    compiled: Some(std::sync::Arc::new(inner_cloned)),
                    memo_cache: None,
                });
                pc += 1;
            }
            Op::PushBuiltin(ref name) => {
                stack.push(LispVal::BuiltinFn(name.clone()));
                pc += 1;
            }
            Op::TracePush(ref name) => {
                state.trace_push(name);
                pc += 1;
            }
            Op::TracePop => {
                state.trace_pop();
                pc += 1;
            }
            Op::PushLiteral(ref val) => {
                stack.push(val.clone());
                pc += 1;
            }
            Op::PushSelf => {
                // Push the current function value onto the stack (for Y combinator)
                // The current function is stored in env as the defined name
                let self_fn = slots[0].clone(); // slot 0 = self param
                stack.push(self_fn);
                pc += 1;
            }
            // --- Sum-type primitives ---
            Op::ConstructTag(ref type_name, variant_id, n_fields) => {
                let n = *n_fields as usize;
                let mut fields = Vec::with_capacity(n);
                for _ in 0..n {
                    fields.push(stack.pop().unwrap_or(LispVal::Nil));
                }
                fields.reverse();
                stack.push(LispVal::Tagged {
                    type_name: type_name.clone(),
                    variant_id: *variant_id,
                    fields,
                });
                pc += 1;
            }
            Op::TagTest(ref type_name, variant_id) => {
                let matches = match stack.last() {
                    Some(LispVal::Tagged { type_name: tn, variant_id: vid, .. }) => {
                        tn == type_name && *vid == *variant_id
                    }
                    _ => false,
                };
                stack.push(LispVal::Bool(matches));
                pc += 1;
            }
            Op::GetField(idx) => {
                let val = stack.pop().unwrap_or(LispVal::Nil);
                match val {
                    LispVal::Tagged { fields, .. } => {
                        let field = fields.get(*idx as usize).cloned().unwrap_or(LispVal::Nil);
                        stack.push(field);
                    }
                    _ => {
                        return Err(format!("get-field: expected tagged value, got {}", val));
                    }
                }
                pc += 1;
            }
            Op::Recur(n) => {
                // Pop N values in reverse order into slots 0..N, reset pc
                for i in (0..*n).rev() {
                    if i < slots.len() {
                        slots[i] = stack.pop().unwrap_or(LispVal::Nil);
                    } else {
                        while slots.len() <= i {
                            slots.push(LispVal::Nil);
                        }
                        slots[i] = stack.pop().unwrap_or(LispVal::Nil);
                    }
                }
                pc = 0;
            }
            Op::RecurDirect(n) => {
                // Same as Recur but guaranteed small N
                for i in (0..*n).rev() {
                    if i < slots.len() {
                        slots[i] = stack.pop().unwrap_or(LispVal::Nil);
                    } else {
                        while slots.len() <= i {
                            slots.push(LispVal::Nil);
                        }
                        slots[i] = stack.pop().unwrap_or(LispVal::Nil);
                    }
                }
                pc = 0;
            }
            Op::Return => {
                let retval = stack.pop().unwrap_or(LispVal::Nil);
                if let Some(frame) = frames.pop() {
                    // Restore caller frame
                    slots = frame.slots;
                    stack = frame.stack;
                    stack.push(retval);
                    pc = frame.pc;
                } else {
                    return Ok(retval);
                }
            }
            Op::StoreSlot(s) => {
                if *s < slots.len() {
                    slots[*s] = stack.pop().unwrap_or(LispVal::Nil);
                } else {
                    // Extend slots for let-bound vars
                    while slots.len() <= *s {
                        slots.push(LispVal::Nil);
                    }
                    slots[*s] = stack.pop().unwrap_or(LispVal::Nil);
                }
                pc += 1;
            }
            Op::Dup => {
                if let Some(top) = stack.last() {
                    stack.push(top.clone());
                }
                pc += 1;
            }
            Op::Pop => {
                stack.pop();
                pc += 1;
            }
            Op::JumpIfFalse(addr) => {
                let val = stack.pop().unwrap_or(LispVal::Nil);
                if !is_truthy(&val) {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfTrue(addr) => {
                let val = stack.pop().unwrap_or(LispVal::Nil);
                if is_truthy(&val) {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::Jump(addr) => {
                pc = *addr;
            }
            Op::DictGet => {
                let key = stack.pop().unwrap_or(LispVal::Nil);
                let map = stack.pop().unwrap_or(LispVal::Nil);
                let result = match (&map, &key) {
                    (LispVal::Map(m), LispVal::Str(k)) => m.get(k).cloned().unwrap_or(LispVal::Nil),
                    _ => LispVal::Nil,
                };
                stack.push(result);
                pc += 1;
            }
            Op::DictSet => {
                let val = stack.pop().unwrap_or(LispVal::Nil);
                let key = stack.pop().unwrap_or(LispVal::Nil);
                let map = stack.pop().unwrap_or(LispVal::Nil);
                let result = match (&map, &key) {
                    (LispVal::Map(m), LispVal::Str(k)) => LispVal::Map(m.update(k.clone(), val)),
                    _ => return Err("dict/set: need (map key value)".into()),
                };
                stack.push(result);
                pc += 1;
            }
            Op::DictMutSet(slot_idx) => {
                let val = stack.pop().unwrap_or(LispVal::Nil);
                let key = stack.pop().unwrap_or(LispVal::Nil);
                if *slot_idx >= slots.len() {
                    return Err(format!("slot index {} out of bounds (slots_len={})", slot_idx, slots.len()));
                }
                // Mutate the dict in the slot directly — no clone
                match &mut slots[*slot_idx] {
                    LispVal::Map(ref mut m) => {
                        if let LispVal::Str(k) = &key {
                            m.insert(k.clone(), val);
                        } else {
                            return Err("dict-mut-set: key must be string".into());
                        }
                    }
                    _ => return Err("dict-mut-set: slot is not a map".into()),
                }
                // Push the mutated dict (same reference) for the result
                stack.push(slots[*slot_idx].clone());
                pc += 1;
            }
            Op::CallSelf(n_args) => {
                // Iterative self-call: save current frame, reset for new call
                let mut self_args: Vec<LispVal> = Vec::with_capacity(*n_args);
                for _ in 0..*n_args {
                    self_args.push(stack.pop().unwrap_or(LispVal::Nil));
                }
                self_args.reverse();
                // Save current frame
                let return_pc = pc + 1;
                frames.push(Frame {
                    pc: return_pc,
                    slots: std::mem::take(&mut slots),
                    stack: std::mem::take(&mut stack),
                });
                // Fresh slots for new invocation
                slots = vec![LispVal::Nil; cl.total_slots];
                for i in 0..cl.num_param_slots.min(self_args.len()) {
                    slots[i] = self_args[i].clone();
                }
                // Pack rest args if variadic
                if let Some(rest_idx) = cl.rest_param_idx {
                    let rest_list = LispVal::List(self_args[cl.num_fixed_params..].to_vec());
                    slots[rest_idx] = rest_list;
                }
                stack = Vec::with_capacity(8);
                pc = 0;
            }
            Op::CallDynamic(n_args) => {
                // Dynamic call: function is on stack top, args below it
                // Stack: [..., arg1, arg2, ..., argN, func]
                let func = stack.pop().unwrap_or(LispVal::Nil);
                let mut call_args: Vec<LispVal> = Vec::with_capacity(*n_args);
                for _ in 0..*n_args {
                    call_args.push(stack.pop().unwrap_or(LispVal::Nil));
                }
                call_args.reverse();
                match vm_call_lambda(&func, &call_args, outer_env, state) {
                    Ok(result) => stack.push(result),
                    Err(e) => return Err(e),
                }
                pc += 1;
            }
            Op::GetDefaultSlot(map_slot, key_slot, default_slot, result_slot) => {
                // Fused: result = dict/get(slots[map], slots[key]) ?? slots[default]
                // Ensure result_slot exists
                while slots.len() <= *result_slot {
                    slots.push(LispVal::Nil);
                }
                let map_val = safe_slot(&slots, *map_slot);
                let key_val = safe_slot(&slots, *key_slot);
                let result = if let (LispVal::Map(ref m), LispVal::Str(ref k)) = (map_val, key_val)
                {
                    match m.get(k) {
                        Some(v) if !matches!(v, LispVal::Nil) => v.clone(),
                        _ => safe_slot(&slots, *default_slot).clone(),
                    }
                } else {
                    safe_slot(&slots, *default_slot).clone()
                };
                if *result_slot < slots.len() {
                    slots[*result_slot] = result;
                }
                pc += 1;
            }
            Op::StoreAndLoadSlot(s) => {
                let val = stack.pop().unwrap_or(LispVal::Nil);
                if *s < slots.len() {
                    slots[*s] = val;
                    // Push Num without clone, clone everything else
                    match safe_slot(&slots, *s) {
                        LispVal::Num(n) => stack.push(LispVal::Num(*n)),
                        _ => stack.push(slots[*s].clone()),
                    }
                } else {
                    while slots.len() <= *s {
                        slots.push(LispVal::Nil);
                    }
                    slots[*s] = val;
                    stack.push(slots[*s].clone());
                }
                pc += 1;
            }
            Op::ReturnSlot(s) => {
                // Flush slot and return directly — no stack push/pop
                let slot_ref = safe_slot(&slots, *s);
                let retval = match slot_ref {
                    LispVal::Num(n) => LispVal::Num(*n),
                    LispVal::Float(f) => LispVal::Float(*f),
                    LispVal::Bool(b) => LispVal::Bool(*b),
                    LispVal::Nil => LispVal::Nil,
                    _ => slot_ref.clone(),
                };
                if let Some(frame) = frames.pop() {
                    slots = frame.slots;
                    stack = frame.stack;
                    stack.push(retval);
                    pc = frame.pc;
                } else {
                    return Ok(retval);
                }
            }
            // Unsupported ops for lambda body — shouldn't appear but handle gracefully
            _ => return Err("compiled lambda: unsupported op".into()),
        }
    }
}

/// Try to compile a loop into bytecode. Returns None if body is too complex.
pub fn try_compile_loop(
    binding_names: &[String],
    binding_vals: Vec<LispVal>,
    body: &LispVal,
    outer_env: &Env,
) -> Option<CompiledLoop> {
    let compiler = LoopCompiler::new(binding_names.to_vec());
    compiler.compile_body(binding_vals, body, outer_env)
}

/// Execute a compiled loop
pub fn exec_compiled_loop(
    cl: &CompiledLoop,
    _outer_env: &mut Env,
    _state: &mut EvalState,
) -> Result<LispVal, String> {
    run_compiled_loop(cl)
}
