use crate::helpers::is_truthy;
use crate::types::{Env, EvalState, LispVal};

// ---------------------------------------------------------------------------
// Loop Bytecode Compiler — tight VM for loop/recur
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
    /// Call captured function from cl.captured[idx] with N args (no slot copy)
    CallCapturedRef(usize, usize),
    /// Push a pre-compiled closure from cl.closures[idx] onto the stack
    PushClosure(usize),
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
}

impl LoopCompiler {
    fn new(slot_names: Vec<String>) -> Self {
        Self {
            slot_map: slot_names,
            code: Vec::new(),
            captured: Vec::new(),
            closures: Vec::new(),
        }
    }

    /// Look up binding name → slot index (bindings first, then captured env)
    fn slot_of(&self, name: &str) -> Option<usize> {
        if let Some(idx) = self.slot_map.iter().position(|s| s == name) {
            return Some(idx);
        }
        None
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
            return true;
        }
        false
    }

    /// Try to compile an expression. Returns false if unsupported.
    fn compile_expr(&mut self, expr: &LispVal, outer_env: &Env) -> bool {
        match expr {
            LispVal::Num(n) => {
                self.code.push(Op::PushI64(*n));
                true
            }
            LispVal::Float(f) => {
                self.code.push(Op::PushFloat(*f));
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
                    "true" => { self.code.push(Op::PushBool(true)); return true; }
                    "false" => { self.code.push(Op::PushBool(false)); return true; }
                    "nil" => { self.code.push(Op::PushNil); return true; }
                    _ => {}
                }
                if let Some(slot) = self.slot_of(name) {
                    self.code.push(Op::LoadSlot(slot));
                    true
                } else if let Some(idx) = self.captured_idx(name) {
                    self.code.push(Op::LoadCaptured(idx));
                    true
                } else if self.try_capture(name, outer_env) {
                    // Just captured — must be in captured_idx now
                    let idx = self.captured_idx(name).unwrap();
                    self.code.push(Op::LoadCaptured(idx));
                    true
                } else {
                    false
                }
            }
            LispVal::List(list) if list.is_empty() => {
                self.code.push(Op::PushNil);
                true
            }
            LispVal::List(list) => {
                if let LispVal::Sym(op) = &list[0] {
                    match op.as_str() {
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
                            self.code.push(Op::PushBool(false));
                            self.code.push(Op::Eq);
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
                        // recur: compile args, emit Recur(N) — valid in any tail position
                        "recur" => {
                            let num_slots = self.slot_map.len();
                            if list.len() - 1 != num_slots {
                                return false;
                            }
                            for arg in &list[1..] {
                                if !self.compile_expr(arg, outer_env) {
                                    return false;
                                }
                            }
                            self.code.push(Op::Recur(num_slots));
                            true
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
                            while i < list.len() {
                                let clause = match list.get(i) {
                                    Some(LispVal::List(c)) if c.len() >= 2 => c.clone(),
                                    _ => {
                                        return false;
                                    }
                                };
                                // else clause — just compile result
                                if clause[0] == LispVal::Sym("else".into()) {
                                    if !self.compile_expr(&clause[1], outer_env) {
                                        return false;
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
                            // patch all end jumps
                            let end_pc = self.code.len();
                            for idx in end_jumps {
                                self.code[idx] = Op::Jump(end_pc);
                            }
                            true
                        }
                        // let: (let ((x init) ...) body)
                        "let" | "let*" => {
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
                            // Track slots we shadow so we can restore them
                            let mut shadowed: Vec<(String, usize)> = Vec::new();
                            let mut all_ok = true;
                            for binding in bindings {
                                match binding {
                                    LispVal::List(pair) if pair.len() >= 2 => {
                                        if let LispVal::Sym(name) = &pair[0] {
                                            if !self.compile_expr(&pair[1], outer_env) {
                                                all_ok = false;
                                                break;
                                            }
                                            // Check if this name already has a slot (shadowing)
                                            if let Some(existing) = self.slot_map.iter().position(|s| s == name) {
                                                self.code.push(Op::StoreSlot(existing));
                                                shadowed.push((name.clone(), existing));
                                            } else {
                                                let slot_idx = self.slot_map.len();
                                                self.slot_map.push(name.clone());
                                                self.code.push(Op::StoreSlot(slot_idx));
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
                            }
                            // Remove any newly added slot names (not shadows)
                            self.slot_map.truncate(let_start);
                            all_ok
                        }
                        // when: (when test body...) → if test (begin body...)
                        "when" => {
                            if list.len() < 3 { return false; }
                            let test = &list[1];
                            if !self.compile_expr(test, outer_env) { return false; }
                            let jf_idx = self.code.len();
                            self.code.push(Op::JumpIfFalse(0));
                            // Compile body as implicit begin
                            for (i, arg) in list[2..].iter().enumerate() {
                                if !self.compile_expr(arg, outer_env) { return false; }
                                if i + 1 < list.len() - 2 {
                                    self.code.push(Op::Pop);
                                }
                            }
                            self.code[jf_idx] = Op::JumpIfFalse(self.code.len());
                            true
                        }
                        // unless: (unless test body...) → if (not test) (begin body...)
                        "unless" => {
                            if list.len() < 3 { return false; }
                            let test = &list[1];
                            if !self.compile_expr(test, outer_env) { return false; }
                            let jt_idx = self.code.len();
                            self.code.push(Op::JumpIfTrue(0));
                            // Compile body as implicit begin
                            for (i, arg) in list[2..].iter().enumerate() {
                                if !self.compile_expr(arg, outer_env) { return false; }
                                if i + 1 < list.len() - 2 {
                                    self.code.push(Op::Pop);
                                }
                            }
                            self.code[jt_idx] = Op::JumpIfTrue(self.code.len());
                            true
                        }
                        "set!" => {
                            if list.len() != 3 { return false; }
                            let name = match &list[1] {
                                LispVal::Sym(s) => s.clone(),
                                _ => return false,
                            };
                            // Only compile set! for local bindings (params/let), not captured vars.
                            // Captured vars are copies — mutation would be lost (no write-back to outer env).
                            if self.slot_map.iter().position(|s| s == &name).is_none() {
                                return false; // captured var — force fallback to tree-walking
                            }
                            let slot = match self.slot_of(&name) {
                                Some(s) => s,
                                None => return false,
                            };
                            if !self.compile_expr(&list[2], outer_env) { return false; }
                            self.code.push(Op::StoreSlot(slot));
                            self.code.push(Op::LoadSlot(slot)); // set! returns the new value
                            true
                        }
                        "lambda" => {
                            // (lambda (params...) body...)
                            if list.len() < 3 { return false; }
                            let params: Vec<String> = match list.get(1) {
                                Some(LispVal::List(ps)) => ps.iter().filter_map(|p| match p {
                                    LispVal::Sym(s) => Some(s.clone()),
                                    _ => None,
                                }).collect(),
                                Some(LispVal::Sym(s)) => vec![s.clone()],
                                _ => return false,
                            };
                            if params.is_empty() { return false; }
                            // Compile lambda body in a new compiler
                            let mut inner = LoopCompiler::new(params.clone());
                            let body = &list[2..];
                            let mut ok = true;
                            for expr in body {
                                if !inner.compile_expr(expr, outer_env) {
                                    ok = false;
                                    break;
                                }
                            }
                            if !ok { return false; }
                            // Compute total_slots
                            let base = params.len();
                            let mut max_slot = base;
                            for op in &inner.code {
                                match op {
                                    Op::LoadSlot(s) | Op::StoreSlot(s) => {
                                        if *s >= max_slot { max_slot = *s + 1; }
                                    }
                                    Op::SlotAddImm(s, _) | Op::SlotMulImm(s, _) => {
                                        if *s >= max_slot { max_slot = *s + 1; }
                                    }
                                    Op::CallCaptured(s, _) => {
                                        if *s >= max_slot { max_slot = *s + 1; }
                                    }
                                    _ => {}
                                }
                            }
                            let idx = self.closures.len();
                            self.closures.push(CompiledLambda {
                                num_param_slots: params.len(),
                                total_slots: max_slot,
                                code: inner.code,
                                captured: inner.captured,
                                closures: inner.closures,
                            });
                            self.code.push(Op::PushClosure(idx));
                            true
                        }
                        _ => {
                            // Function call: captured var or assumed builtin
                            let n_args = list.len() - 1;
                            for arg in &list[1..] {
                                if !self.compile_expr(arg, outer_env) {
                                    return false;
                                }
                            }
                            if let Some(idx) = self.captured_idx(op) {
                                self.code.push(Op::CallCapturedRef(idx, n_args));
                            } else if let Some(slot) = self.slot_of(op) {
                                self.code.push(Op::CallCaptured(slot, n_args));
                            } else if self.try_capture(op, outer_env) {
                                let idx = self.captured_idx(op).unwrap();
                                self.code.push(Op::CallCapturedRef(idx, n_args));
                            } else {
                                self.code.push(Op::BuiltinCall(op.clone(), n_args));
                            }
                            true
                        }
                    }
                } else {
                    false
                }
            }
            _ => false,
        }
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
                peephole_optimize(&mut code);
                // Second pass: now that 3-op and 2-op fusions are done, check for mega-fuse
                peephole_optimize(&mut code);
                // Third pass: 2-op fusion may have created new JumpIfSlotCmpImm for mega-fuse
                peephole_optimize(&mut code);
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
            peephole_optimize(&mut code);
            // Second pass: now that 3-op and 2-op fusions are done, check for mega-fuse
            peephole_optimize(&mut code);
            // Third pass: 2-op fusion may have created new JumpIfSlotCmpImm for mega-fuse
            peephole_optimize(&mut code);
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

/// Peephole optimizer: fuse LoadSlot + PushI64 + Arith/Cmp sequences,
/// convert small Recur → RecurDirect, fuse SlotCmpImm + JumpIfFalse,
/// and remap jump targets.
fn peephole_optimize(code: &mut Vec<Op>) {
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
                let slot_ref = &slots[*s];
                match slot_ref {
                    LispVal::Num(n) => stack.push(LispVal::Num(*n)),
                    _ => stack.push(slot_ref.clone()),
                }
                pc += 1;
            }
            Op::LoadCaptured(idx) => {
                // Note: in run_compiled_loop, captured is in slots. In run_compiled_lambda, it's in cl.captured.
                // The loop VM pre-fills captured into slots, so this op shouldn't appear there.
                // If it does, fall through to error.
                stack.push(cl.captured[*idx].1.clone());
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
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                stack.push(LispVal::Num(a + b));
                pc += 1;
            }
            Op::Sub => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                stack.push(LispVal::Num(a - b));
                pc += 1;
            }
            Op::Mul => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                stack.push(LispVal::Num(a * b));
                pc += 1;
            }
            Op::Div => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                if b == 0 {
                    return Err("division by zero".into());
                }
                stack.push(LispVal::Num(a / b));
                pc += 1;
            }
            Op::Mod => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                if b == 0 {
                    return Err("modulo by zero".into());
                }
                stack.push(LispVal::Num(a % b));
                pc += 1;
            }
            Op::Eq => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(lisp_eq(&a, &b)));
                pc += 1;
            }
            Op::Lt => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                stack.push(LispVal::Bool(a < b));
                pc += 1;
            }
            Op::Le => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                stack.push(LispVal::Bool(a <= b));
                pc += 1;
            }
            Op::Gt => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                stack.push(LispVal::Bool(a > b));
                pc += 1;
            }
            Op::Ge => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                stack.push(LispVal::Bool(a >= b));
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
                let v = num_val_ref(&slots[*s]);
                let result = v + imm;
                // DON'T write back to slot — Recur/RecurDirect pops from stack
                stack.push(LispVal::Num(result));
                pc += 1;
            }
            Op::SlotSubImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                let result = v - imm;
                // DON'T write back to slot — Recur/RecurDirect pops from stack
                stack.push(LispVal::Num(result));
                pc += 1;
            }
            Op::SlotMulImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Num(v * imm));
                pc += 1;
            }
            Op::SlotDivImm(s, imm) => {
                if *imm == 0 {
                    return Err("division by zero".into());
                }
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Num(v / imm));
                pc += 1;
            }
            Op::SlotEqImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Bool(v == *imm));
                pc += 1;
            }
            Op::SlotLtImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Bool(v < *imm));
                pc += 1;
            }
            Op::SlotLeImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Bool(v <= *imm));
                pc += 1;
            }
            Op::SlotGtImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Bool(v > *imm));
                pc += 1;
            }
            Op::SlotGeImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Bool(v >= *imm));
                pc += 1;
            }
            // --- Super-fused: cmp + jump without stack traffic ---
            Op::JumpIfSlotLtImm(s, imm, addr) => {
                let v = num_val_ref(&slots[*s]);
                if v < *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfSlotLeImm(s, imm, addr) => {
                let v = num_val_ref(&slots[*s]);
                if v <= *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfSlotGtImm(s, imm, addr) => {
                let v = num_val_ref(&slots[*s]);
                if v > *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfSlotGeImm(s, imm, addr) => {
                let v = num_val_ref(&slots[*s]);
                if v >= *imm {
                    pc = *addr;
                } else {
                    pc += 1;
                }
            }
            Op::JumpIfSlotEqImm(s, imm, addr) => {
                let v = num_val_ref(&slots[*s]);
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
                let result = eval_builtin(name, &args)?;
                stack.push(result);
                pc += 1;
            }
            Op::CallCaptured(_, _) | Op::CallCapturedRef(_, _) | Op::PushClosure(_) => {
                return Err("loop VM: CallCaptured not supported in loop body".into());
            }
        }
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

/// Polymorphic arithmetic: if either operand is Float, use float arithmetic.
fn num_arith(
    a: &LispVal, b: &LispVal,
    int_op: impl Fn(i64, i64) -> i64,
    float_op: impl Fn(f64, f64) -> f64,
) -> LispVal {
    match (a, b) {
        (LispVal::Float(x), LispVal::Float(y)) => LispVal::Float(float_op(*x, *y)),
        (LispVal::Float(x), LispVal::Num(y)) => LispVal::Float(float_op(*x, *y as f64)),
        (LispVal::Num(x), LispVal::Float(y)) => LispVal::Float(float_op(*x as f64, *y)),
        (LispVal::Num(x), LispVal::Num(y)) => LispVal::Num(int_op(*x, *y)),
        _ => LispVal::Num(0),
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
        _ => false,
    }
}

/// Evaluate a builtin by name (for Op::BuiltinCall)
pub fn eval_builtin(name: &str, args: &[LispVal]) -> Result<LispVal, String> {
    match name {
        "abs" => Ok(LispVal::Num(
            num_val(args.get(0).cloned().unwrap_or(LispVal::Nil)).abs(),
        )),
        "min" => {
            let a = num_val(args.get(0).cloned().unwrap_or(LispVal::Nil));
            let b = num_val(args.get(1).cloned().unwrap_or(LispVal::Nil));
            Ok(LispVal::Num(a.min(b)))
        }
        "max" => {
            let a = num_val(args.get(0).cloned().unwrap_or(LispVal::Nil));
            let b = num_val(args.get(1).cloned().unwrap_or(LispVal::Nil));
            Ok(LispVal::Num(a.max(b)))
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
            }
            Ok(LispVal::Num(
                num_val(args.get(0).cloned().unwrap_or(LispVal::Nil)) % b,
            ))
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
            (Some(LispVal::Num(i)), Some(LispVal::List(l))) => {
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
        "number?" => Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::Num(_)) | Some(LispVal::Float(_))))),
        "boolean?" => Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::Bool(_))))),
        "list?" => Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::List(_))))),
        "pair?" => Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::List(l)) if l.len() >= 2))),
        "symbol?" => Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::Sym(_))))),
        "int?" => Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::Num(_))))),
        "float?" => Ok(LispVal::Bool(matches!(args.get(0), Some(LispVal::Float(_))))),
        "reverse" => match args.get(0) {
            Some(LispVal::List(l)) => Ok(LispVal::List(l.iter().rev().cloned().collect())),
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
            Some(LispVal::List(l)) if l.len() > 1 => {
                Ok(LispVal::List(l[..l.len()-1].to_vec()))
            }
            _ => Ok(LispVal::Nil),
        },
        "range" => {
            let start = num_val(args.get(0).cloned().unwrap_or(LispVal::Nil));
            let end = num_val(args.get(1).cloned().unwrap_or(LispVal::Nil));
            let step = if args.len() > 2 { num_val(args.get(2).cloned().unwrap_or(LispVal::Nil)) } else { 1 };
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
            let n = num_val(args.get(0).cloned().unwrap_or(LispVal::Nil));
            Ok(LispVal::Float((n as f64).sqrt()))
        }
        "pow" => {
            let base = num_val(args.get(0).cloned().unwrap_or(LispVal::Nil));
            let exp = num_val(args.get(1).cloned().unwrap_or(LispVal::Nil));
            Ok(LispVal::Float((base as f64).powf(exp as f64)))
        }
        "dict/get" | "dict-ref" => {
            match (args.get(0), args.get(1)) {
                (Some(LispVal::Map(m)), Some(LispVal::Str(key))) => {
                    Ok(m.get(key).cloned().unwrap_or(LispVal::Nil))
                }
                _ => Ok(LispVal::Nil),
            }
        }
        "dict/set" | "dict-set" => {
            match (args.get(0), args.get(1), args.get(2)) {
                (Some(LispVal::Map(m)), Some(LispVal::Str(key)), Some(val)) => {
                    Ok(LispVal::Map(m.update(key.clone(), val.clone())))
                }
                _ => Err("dict/set: need (map key value)".into()),
            }
        }
        "dict/has?" => {
            match (args.get(0), args.get(1)) {
                (Some(LispVal::Map(m)), Some(LispVal::Str(key))) => {
                    Ok(LispVal::Bool(m.contains_key(key)))
                }
                _ => Ok(LispVal::Bool(false)),
            }
        }
        "dict/keys" => {
            match args.get(0) {
                Some(LispVal::Map(m)) => {
                    let keys: Vec<LispVal> = m.keys().map(|k| LispVal::Str(k.clone())).collect();
                    Ok(LispVal::List(keys))
                }
                _ => Ok(LispVal::List(vec![])),
            }
        }
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
        "str-ends-with" | "string-suffix?" => {
            match (args.get(0), args.get(1)) {
                (Some(LispVal::Str(s)), Some(LispVal::Str(suffix))) => {
                    Ok(LispVal::Bool(s.ends_with(suffix.as_str())))
                }
                _ => Ok(LispVal::Bool(false)),
            }
        }
        "str-starts-with" | "string-prefix?" => {
            match (args.get(0), args.get(1)) {
                (Some(LispVal::Str(s)), Some(LispVal::Str(prefix))) => {
                    Ok(LispVal::Bool(s.starts_with(prefix.as_str())))
                }
                _ => Ok(LispVal::Bool(false)),
            }
        }
        "substring" => {
            match (args.get(0), args.get(1), args.get(2)) {
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
            }
        }
        "str->num" | "string->number" => {
            match args.get(0) {
                Some(LispVal::Str(s)) => {
                    if let Ok(n) = s.parse::<i64>() {
                        Ok(LispVal::Num(n))
                    } else if let Ok(f) = s.parse::<f64>() {
                        Ok(LispVal::Float(f))
                    } else {
                        Ok(LispVal::Nil)
                    }
                }
                _ => Ok(LispVal::Nil),
            }
        }
        "num->str" | "number->string" => {
            match args.get(0) {
                Some(LispVal::Num(n)) => Ok(LispVal::Str(n.to_string())),
                Some(LispVal::Float(f)) => Ok(LispVal::Str(f.to_string())),
                _ => Ok(LispVal::Str("0".to_string())),
            }
        }
        // --- Time ---
        "now" => {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            Ok(LispVal::Float(ts))
        }
        "elapsed" => {
            match args.get(0) {
                Some(v) => {
                    let since = crate::helpers::as_float(v).unwrap_or(0.0);
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs_f64())
                        .unwrap_or(0.0);
                    Ok(LispVal::Float(now - since))
                }
                None => Ok(LispVal::Float(0.0)),
            }
        }
        // --- Type conversions ---
        "float" => {
            match args.get(0) {
                Some(LispVal::Num(n)) => Ok(LispVal::Float(*n as f64)),
                Some(v) => {
                    Ok(crate::helpers::as_float(v).map(LispVal::Float)
                        .unwrap_or(LispVal::Float(0.0)))
                }
                None => Ok(LispVal::Float(0.0)),
            }
        }
        "integer" => {
            match args.get(0) {
                Some(LispVal::Float(f)) => Ok(LispVal::Num(*f as i64)),
                Some(LispVal::Num(n)) => Ok(LispVal::Num(*n)),
                _ => Ok(LispVal::Num(0)),
            }
        }
        "boolean" => {
            Ok(LispVal::Bool(crate::helpers::is_truthy(args.get(0).unwrap_or(&LispVal::Nil))))
        }
        // --- Error ---
        "error" => {
            let msg = match args.get(0) {
                Some(LispVal::Str(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => "error".to_string(),
            };
            Err(msg)
        }
        _ => Err(format!("loop bytecode: unknown builtin '{}'", name)),
    }
}

/// Compiled lambda: a flat bytecode program with N param slots + captured env slots.
/// Used for fast-path map/filter/reduce — avoids env push/pop per element.
#[derive(Clone, Debug)]
pub struct CompiledLambda {
    pub num_param_slots: usize,
    pub total_slots: usize,
    pub code: Vec<Op>,
    pub captured: Vec<(String, LispVal)>,
    /// Pre-compiled inner lambdas (closures). Indexed by PushClosure(N).
    pub closures: Vec<CompiledLambda>,
}

/// Try to compile a lambda body for fast inline evaluation.
/// Returns None if the body contains unsupported forms (macros, user-defined functions, etc.)
pub fn try_compile_lambda(
    param_names: &[String],
    body: &LispVal,
    _closed_env: &[(String, LispVal)],
    outer_env: &Env,
) -> Option<CompiledLambda> {
    let mut compiler = LoopCompiler::new(param_names.to_vec());
    // Don't pre-populate captured — try_capture will pull in only what's needed from outer_env.
    // closed_env contains the ENTIRE scope snapshot (all builtins, etc) — most are unused.
    if !compiler.compile_expr(body, outer_env) {
        return None;
    }
    compiler.code.push(Op::Return);
    let mut code = compiler.code;
    peephole_optimize(&mut code);
    peephole_optimize(&mut code);
    peephole_optimize(&mut code);
    // Compute total slots: params + any let-binding slots used in code
    // Captured vars are accessed via LoadCaptured/CallCapturedRef, not slots
    let base_slots = param_names.len();
    let mut max_slot = base_slots;
    for op in &code {
        match op {
            Op::StoreSlot(s) | Op::LoadSlot(s) => max_slot = max_slot.max(*s + 1),
            Op::SlotAddImm(s, _) | Op::SlotSubImm(s, _) | Op::SlotMulImm(s, _) | Op::SlotDivImm(s, _) => {
                max_slot = max_slot.max(*s + 1);
            }
            Op::SlotEqImm(s, _) | Op::SlotLtImm(s, _) | Op::SlotLeImm(s, _) | Op::SlotGtImm(s, _) | Op::SlotGeImm(s, _) => {
                max_slot = max_slot.max(*s + 1);
            }
            Op::JumpIfSlotLtImm(s, _, _) | Op::JumpIfSlotLeImm(s, _, _) | Op::JumpIfSlotGtImm(s, _, _) | Op::JumpIfSlotGeImm(s, _, _) | Op::JumpIfSlotEqImm(s, _, _) => {
                max_slot = max_slot.max(*s + 1);
            }
            Op::RecurIncAccum(a, b, _, _, _) => max_slot = max_slot.max(*a.max(b) + 1),
            Op::CallCaptured(s, _) => max_slot = max_slot.max(*s + 1),
            _ => {}
        }
    }
    Some(CompiledLambda {
        num_param_slots: param_names.len(),
        total_slots: max_slot,
        code,
        captured: compiler.captured,
        closures: compiler.closures,
    })
}

/// Run a compiled lambda with the given arguments. Returns the result directly.
pub fn run_compiled_lambda(
    cl: &CompiledLambda,
    args: &[LispVal],
    outer_env: &Env,
    state: &mut EvalState,
) -> Result<LispVal, String> {
    let mut slots: Vec<LispVal> = vec![LispVal::Nil; cl.total_slots];
    // Fill param slots only — captured vars accessed via LoadCaptured/CallCapturedRef
    for i in 0..cl.num_param_slots {
        slots[i] = args.get(i).cloned().unwrap_or(LispVal::Nil);
    }
    let mut stack: Vec<LispVal> = Vec::with_capacity(8);
    let code = &cl.code;
    let mut pc: usize = 0;
    let mut ops: u32 = 0;
    const LAMBDA_BUDGET: u32 = 1_000_000;

    loop {
        ops += 1;
        if ops > LAMBDA_BUDGET {
            return Err("compiled lambda: budget exceeded (possible infinite loop)".into());
        }
        match &code[pc] {
            Op::LoadSlot(s) => {
                let slot_ref = &slots[*s];
                match slot_ref {
                    LispVal::Num(n) => stack.push(LispVal::Num(*n)),
                    _ => stack.push(slot_ref.clone()),
                }
                pc += 1;
            }
            Op::LoadCaptured(idx) => {
                // Note: in run_compiled_loop, captured is in slots. In run_compiled_lambda, it's in cl.captured.
                // The loop VM pre-fills captured into slots, so this op shouldn't appear there.
                // If it does, fall through to error.
                stack.push(cl.captured[*idx].1.clone());
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
            Op::Add => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(num_arith(&a, &b, |x, y| x + y, |x, y| x + y));
                pc += 1;
            }
            Op::Sub => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(num_arith(&a, &b, |x, y| x - y, |x, y| x - y));
                pc += 1;
            }
            Op::Mul => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(num_arith(&a, &b, |x, y| x * y, |x, y| x * y));
                pc += 1;
            }
            Op::Div => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                let bf = num_val_ref(&b);
                if bf == 0 {
                    return Err("division by zero".into());
                }
                stack.push(num_arith(&a, &b, |x, y| x / y, |x, y| x / y));
                pc += 1;
            }
            Op::Mod => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                if b == 0 {
                    return Err("modulo by zero".into());
                }
                stack.push(LispVal::Num(a % b));
                pc += 1;
            }
            Op::Eq => {
                let b = stack.pop().unwrap_or(LispVal::Nil);
                let a = stack.pop().unwrap_or(LispVal::Nil);
                stack.push(LispVal::Bool(lisp_eq(&a, &b)));
                pc += 1;
            }
            Op::Lt => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                stack.push(LispVal::Bool(a < b));
                pc += 1;
            }
            Op::Le => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                stack.push(LispVal::Bool(a <= b));
                pc += 1;
            }
            Op::Gt => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                stack.push(LispVal::Bool(a > b));
                pc += 1;
            }
            Op::Ge => {
                let b = num_val(stack.pop().unwrap_or(LispVal::Nil));
                let a = num_val(stack.pop().unwrap_or(LispVal::Nil));
                stack.push(LispVal::Bool(a >= b));
                pc += 1;
            }
            Op::SlotAddImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Num(v + imm));
                pc += 1;
            }
            Op::SlotSubImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Num(v - imm));
                pc += 1;
            }
            Op::SlotMulImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Num(v * imm));
                pc += 1;
            }
            Op::SlotDivImm(s, imm) => {
                if *imm == 0 {
                    return Err("division by zero".into());
                }
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Num(v / imm));
                pc += 1;
            }
            Op::SlotEqImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Bool(v == *imm));
                pc += 1;
            }
            Op::SlotLtImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Bool(v < *imm));
                pc += 1;
            }
            Op::SlotLeImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Bool(v <= *imm));
                pc += 1;
            }
            Op::SlotGtImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Bool(v > *imm));
                pc += 1;
            }
            Op::SlotGeImm(s, imm) => {
                let v = num_val_ref(&slots[*s]);
                stack.push(LispVal::Bool(v >= *imm));
                pc += 1;
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
                        if let LispVal::Lambda { rest_param: None, compiled: Some(ref cl2), .. } = func {
                            let comp_cl = cl2.clone();
                            for v in vals.iter() {
                                match run_compiled_lambda(&comp_cl, &[v.clone()], outer_env, state) {
                                    Ok(r) => result.push(r),
                                    Err(_) => {
                                        let mut env_clone = outer_env.clone();
                                        match crate::eval::call_val(func, &[v.clone()], &mut env_clone, state) {
                                            Ok(crate::eval::continuation::EvalResult::Value(r)) => result.push(r),
                                            Ok(crate::eval::continuation::EvalResult::TailCall { expr, env: tail_env }) => {
                                                let mut e2 = tail_env;
                                                result.push(crate::eval::lisp_eval(&expr, &mut e2, state)?);
                                            }
                                            Err(e) => return Err(e),
                                        }
                                    }
                                }
                            }
                        } else {
                            for v in vals.iter() {
                                let mut env_clone = outer_env.clone();
                                match crate::eval::call_val(func, &[v.clone()], &mut env_clone, state) {
                                    Ok(crate::eval::continuation::EvalResult::Value(r)) => result.push(r),
                                    Ok(crate::eval::continuation::EvalResult::TailCall { expr, env: tail_env }) => {
                                        let mut e2 = tail_env;
                                        result.push(crate::eval::lisp_eval(&expr, &mut e2, state)?);
                                    }
                                    Err(e) => return Err(e),
                                }
                            }
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
                        if let LispVal::Lambda { rest_param: None, compiled: Some(ref cl2), .. } = func {
                            let comp_cl = cl2.clone();
                            for v in vals.iter() {
                                let keep = match run_compiled_lambda(&comp_cl, &[v.clone()], outer_env, state) {
                                    Ok(LispVal::Bool(b)) => b,
                                    Ok(_) => true,
                                    Err(_) => {
                                        let mut env_clone = outer_env.clone();
                                        match crate::eval::call_val(func, &[v.clone()], &mut env_clone, state) {
                                            Ok(crate::eval::continuation::EvalResult::Value(LispVal::Bool(b))) => b,
                                            _ => false,
                                        }
                                    }
                                };
                                if keep { result.push(v.clone()); }
                            }
                        } else {
                            for v in vals.iter() {
                                let mut env_clone = outer_env.clone();
                                let keep = match crate::eval::call_val(func, &[v.clone()], &mut env_clone, state) {
                                    Ok(crate::eval::continuation::EvalResult::Value(LispVal::Bool(b))) => b,
                                    _ => false,
                                };
                                if keep { result.push(v.clone()); }
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
                        if let LispVal::Lambda { rest_param: None, compiled: Some(ref cl2), .. } = func {
                            let comp_cl = cl2.clone();
                            for v in vals.iter() {
                                let _ = run_compiled_lambda(&comp_cl, &[v.clone()], outer_env, state);
                            }
                        } else {
                            for v in vals.iter() {
                                let mut env_clone = outer_env.clone();
                                let _ = crate::eval::call_val(func, &[v.clone()], &mut env_clone, state);
                            }
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
                        if let LispVal::Lambda { rest_param: None, compiled: Some(ref cl2), .. } = comparator {
                            let comp_cl = cl2.clone();
                            vals.sort_by(|a, b| {
                                match run_compiled_lambda(&comp_cl, &[a.clone(), b.clone()], outer_env, state) {
                                    Ok(LispVal::Bool(true)) => std::cmp::Ordering::Less,
                                    Ok(LispVal::Bool(false)) => std::cmp::Ordering::Greater,
                                    _ => std::cmp::Ordering::Equal,
                                }
                            });
                        } else {
                            let func = comparator.clone();
                            vals.sort_by(|a, b| {
                                let mut env_clone = outer_env.clone();
                                match crate::eval::call_val(&func, &[a.clone(), b.clone()], &mut env_clone, state) {
                                    Ok(crate::eval::continuation::EvalResult::Value(LispVal::Bool(true))) => std::cmp::Ordering::Less,
                                    Ok(crate::eval::continuation::EvalResult::Value(LispVal::Bool(false))) => std::cmp::Ordering::Greater,
                                    _ => std::cmp::Ordering::Equal,
                                }
                            });
                        }
                        stack.push(LispVal::List(vals));
                    }
                    _ => {
                        // Regular builtin — no lambda args needed
                        let result = eval_builtin(name, &bargs)?;
                        stack.push(result);
                    }
                }
                pc += 1;
            }
            Op::CallCaptured(slot, n_args) => {
                let func = slots[*slot].clone();
                let mut cargs: Vec<LispVal> = Vec::with_capacity(*n_args);
                for _ in 0..*n_args {
                    cargs.push(stack.pop().unwrap_or(LispVal::Nil));
                }
                cargs.reverse();
                if let LispVal::Lambda { rest_param: None, compiled: Some(ref cl2), .. } = func {
                    match run_compiled_lambda(cl2, &cargs, outer_env, state) {
                        Ok(v) => { stack.push(v); }
                        Err(_) => {
                            let mut env_clone = outer_env.clone();
                            match crate::eval::call_val(&func, &cargs, &mut env_clone, state)? {
                                crate::eval::continuation::EvalResult::Value(v) => { stack.push(v); }
                                crate::eval::continuation::EvalResult::TailCall { expr, env: tail_env } => {
                                    env_clone = tail_env;
                                    stack.push(crate::eval::lisp_eval(&expr, &mut env_clone, state)?);
                                }
                            }
                        }
                    }
                } else {
                    let mut env_clone = outer_env.clone();
                    match crate::eval::call_val(&func, &cargs, &mut env_clone, state)? {
                        crate::eval::continuation::EvalResult::Value(v) => { stack.push(v); }
                        crate::eval::continuation::EvalResult::TailCall { expr, env: tail_env } => {
                            env_clone = tail_env;
                            stack.push(crate::eval::lisp_eval(&expr, &mut env_clone, state)?);
                        }
                    }
                }
                pc += 1;
            }
            Op::CallCapturedRef(idx, n_args) => {
                let func = cl.captured[*idx].1.clone();
                let mut cargs: Vec<LispVal> = Vec::with_capacity(*n_args);
                for _ in 0..*n_args {
                    cargs.push(stack.pop().unwrap_or(LispVal::Nil));
                }
                cargs.reverse();
                if let LispVal::Lambda { rest_param: None, compiled: Some(ref cl2), .. } = func {
                    match run_compiled_lambda(cl2, &cargs, outer_env, state) {
                        Ok(v) => { stack.push(v); }
                        Err(_) => {
                            let mut env_clone = outer_env.clone();
                            match crate::eval::call_val(&func, &cargs, &mut env_clone, state)? {
                                crate::eval::continuation::EvalResult::Value(v) => { stack.push(v); }
                                crate::eval::continuation::EvalResult::TailCall { expr, env: tail_env } => {
                                    env_clone = tail_env;
                                    stack.push(crate::eval::lisp_eval(&expr, &mut env_clone, state)?);
                                }
                            }
                        }
                    }
                } else {
                    let mut env_clone = outer_env.clone();
                    match crate::eval::call_val(&func, &cargs, &mut env_clone, state)? {
                        crate::eval::continuation::EvalResult::Value(v) => { stack.push(v); }
                        crate::eval::continuation::EvalResult::TailCall { expr, env: tail_env } => {
                            env_clone = tail_env;
                            stack.push(crate::eval::lisp_eval(&expr, &mut env_clone, state)?);
                        }
                    }
                }
                pc += 1;
            }
            Op::PushClosure(idx) => {
                let inner = &cl.closures[*idx];
                let closed_env = {
                    let mut map = im::HashMap::new();
                    for (name, val) in &inner.captured {
                        map.insert(name.clone(), val.clone());
                    }
                    std::sync::Arc::new(std::sync::RwLock::new(map))
                };
                // Build a dummy body (Nil) — compiled path never reads it
                stack.push(LispVal::Lambda {
                    params: Vec::new(), // params baked into CompiledLambda
                    rest_param: None,
                    body: Box::new(LispVal::Nil),
                    closed_env,
                    pure_type: None,
                    compiled: Some(Box::new(inner.clone())),
                });
                pc += 1;
            }
            Op::Return => {
                return Ok(stack.pop().unwrap_or(LispVal::Nil));
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
