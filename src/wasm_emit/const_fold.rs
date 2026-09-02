use super::*;

impl WasmEmitter {
    pub(crate) fn const_eval(&self, e: &LispVal) -> Option<LispVal> {
        match e {
            LispVal::Num(_) => Some(e.clone()),
            LispVal::List(items) if items.len() >= 3 => {
                let LispVal::Sym(op) = &items[0] else {
                    return None;
                };
                let args: Vec<LispVal> = items[1..]
                    .iter()
                    .filter_map(|x| self.const_eval(x))
                    .collect();
                if args.len() != items.len() - 1 {
                    return None;
                } // not all constant
                let nums: Option<Vec<i64>> = args
                    .iter()
                    .map(|x| {
                        if let LispVal::Num(n) = x {
                            Some(*n)
                        } else {
                            None
                        }
                    })
                    .collect();
                let nums = match nums {
                    Some(n) => n,
                    None => return None,
                };
                let result = match op.as_str() {
                    "+" => {
                        let r = nums
                            .iter()
                            .skip(1)
                            .try_fold(nums[0], |a: i64, &b: &i64| a.checked_add(b));
                        r? // return None on overflow (don't fold)
                    }
                    "-" if nums.len() == 1 => nums[0].checked_neg()?,
                    "-" => {
                        let r = nums
                            .iter()
                            .skip(1)
                            .try_fold(nums[0], |a: i64, &b: &i64| a.checked_sub(b));
                        r?
                    }
                    "*" => {
                        let r = nums
                            .iter()
                            .skip(1)
                            .try_fold(nums[0], |a: i64, &b: &i64| a.checked_mul(b));
                        r?
                    }
                    "wrap-add" => nums.iter().skip(1).fold(nums[0], |a, &b| a.wrapping_add(b)),
                    "wrap-sub" if nums.len() == 1 => nums[0].wrapping_neg(),
                    "wrap-sub" => nums.iter().skip(1).fold(nums[0], |a, &b| a.wrapping_sub(b)),
                    "wrap-mul" => nums.iter().skip(1).fold(nums[0], |a, &b| a.wrapping_mul(b)),
                    "/" if nums.len() == 2 && nums[1] != 0 => nums[0] / nums[1],
                    "mod" if nums.len() == 2 && nums[1] != 0 => nums[0] % nums[1],
                    "<" if nums.len() == 2 => {
                        if nums[0] < nums[1] {
                            return Some(LispVal::Bool(true));
                        } else {
                            return Some(LispVal::Bool(false));
                        }
                    }
                    ">" if nums.len() == 2 => {
                        if nums[0] > nums[1] {
                            return Some(LispVal::Bool(true));
                        } else {
                            return Some(LispVal::Bool(false));
                        }
                    }
                    "<=" if nums.len() == 2 => {
                        if nums[0] <= nums[1] {
                            return Some(LispVal::Bool(true));
                        } else {
                            return Some(LispVal::Bool(false));
                        }
                    }
                    ">=" if nums.len() == 2 => {
                        if nums[0] >= nums[1] {
                            return Some(LispVal::Bool(true));
                        } else {
                            return Some(LispVal::Bool(false));
                        }
                    }
                    "=" if nums.len() == 2 => {
                        if nums[0] == nums[1] {
                            return Some(LispVal::Bool(true));
                        } else {
                            return Some(LispVal::Bool(false));
                        }
                    }
                    "abs" if nums.len() == 1 => nums[0].abs(),
                    "max" => *nums.iter().max().unwrap(),
                    "min" => *nums.iter().min().unwrap(),
                    _ => return None,
                };
                Some(LispVal::Num(result))
            }
            _ => None,
        }
    }

    pub(crate) fn fold_binop(
        &mut self,
        a: &[LispVal],
        op: Instruction<'static>,
        identity: i64,
    ) -> Result<Vec<Instruction<'static>>, String> {
        if a.is_empty() {
            return self.emit_tagged_const(identity, TAG_NUM);
        }
        // Deep constant folding: try to const_eval each arg first
        let folded_args: Vec<LispVal> = a
            .iter()
            .map(|x| self.const_eval(x).unwrap_or_else(|| x.clone()))
            .collect();
        // Non-numeric literal operand, ONLY under an active try (the checker
        // is lenient inside try bodies, so these reach the emitter): emit a
        // catch jump. Outside try, behavior is unchanged — applications
        // (List-of-Sym forms) are runtime values, never literals here.
        if !self.try_stack.is_empty() {
            let literal_nonnum = |x: &LispVal| match x {
                LispVal::Str(_) | LispVal::Bool(_) | LispVal::Nil | LispVal::Vec(_) => true,
                // List: literal data only when head is not a symbol (i.e. not
                // an application form like (fib (- n 1)))
                LispVal::List(l) => !l.first().map(|h| matches!(h, LispVal::Sym(_))).unwrap_or(true),
                _ => false,
            };
            if folded_args.iter().any(|x| literal_nonnum(x)) {
                let mut v = Vec::new();
                if self.try_guard(&mut v, "arith: non-numeric operand") {
                    return Ok(v);
                }
            }
        }
        // If all args folded to constants, compute at compile time (checked!)
        let all_const = folded_args.iter().all(|x| matches!(x, LispVal::Num(_)));
        if all_const {
            let nums: Vec<i64> = folded_args
                .iter()
                .map(|x| if let LispVal::Num(n) = x { *n } else { 0 })
                .collect();
            let folded = match &op {
                Instruction::I64Add => nums
                    .iter()
                    .skip(1)
                    .try_fold(nums[0], |acc: i64, &x: &i64| acc.checked_add(x)),
                Instruction::I64Sub => nums
                    .iter()
                    .skip(1)
                    .try_fold(nums[0], |acc: i64, &x: &i64| acc.checked_sub(x)),
                Instruction::I64Mul => nums
                    .iter()
                    .skip(1)
                    .try_fold(nums[0], |acc: i64, &x: &i64| acc.checked_mul(x)),
                _ => None,
            };
            match folded {
                Some(result) => match self.emit_tagged_const(result, TAG_NUM) {
                    Ok(v) => return Ok(v),
                    // tagged-range overflow (|result| >= 2^60): under try this
                    // is a catchable runtime overflow for the interpreter —
                    // emit a catch jump; otherwise propagate the loud error.
                    Err(msg) => {
                        let mut v = Vec::new();
                        if self.try_guard(&mut v, "arith: overflow") {
                            return Ok(v);
                        }
                        return Err(msg);
                    }
                },
                None => {
                    // compile-time overflow: same try handling
                    let mut v = Vec::new();
                    if self.try_guard(&mut v, "arith: overflow") {
                        return Ok(v);
                    }
                    return Err("arithmetic overflow at compile time".into());
                }
            }
        }
        // Runtime path. Add/Sub operate on TAGGED operands directly:
        // (a<<3) ± (b<<3) == (a±b)<<3, so a checked add/sub on tagged values
        // traps EXACTLY when the result leaves the 61-bit payload range —
        // no untag/retag, no silent wrap window. Mul must untag (tagged
        // squares don't compose); the product stays untagged across args and
        // is range-checked + re-tagged once, after the loop.
        let tagged_direct = matches!(op, Instruction::I64Add | Instruction::I64Sub);
        let mut v = self.expr(&folded_args[0])?;
        if !tagged_direct {
            v.extend(self.emit_untag());
        }
        for x in &folded_args[1..] {
            v.extend(self.expr(x)?);
            if !tagged_direct {
                v.extend(self.emit_untag());
            }
            match &op {
                Instruction::I64Add => v.extend(self.emit_checked_add()),
                Instruction::I64Sub => v.extend(self.emit_checked_sub()),
                Instruction::I64Mul => v.extend(self.emit_checked_mul()),
                _ => v.push(op.clone()),
            }
        }
        if !tagged_direct {
            v.extend(self.emit_checked_retag());
        }
        Ok(v)
    }

    pub(crate) fn fold_binop_wrapping(
        &mut self,
        a: &[LispVal],
        op: Instruction<'static>,
        identity: i64,
    ) -> Result<Vec<Instruction<'static>>, String> {
        if a.is_empty() {
            return Ok(self.emit_tagged_const_wrapping(identity, TAG_NUM));
        }
        let folded_args: Vec<LispVal> = a
            .iter()
            .map(|x| self.const_eval(x).unwrap_or_else(|| x.clone()))
            .collect();
        let all_const = folded_args.iter().all(|x| matches!(x, LispVal::Num(_)));
        if all_const {
            let nums: Vec<i64> = folded_args
                .iter()
                .map(|x| if let LispVal::Num(n) = x { *n } else { 0 })
                .collect();
            let folded = match &op {
                Instruction::I64Add => Some(
                    nums.iter()
                        .skip(1)
                        .fold(nums[0], |acc, &x| acc.wrapping_add(x)),
                ),
                Instruction::I64Sub => Some(
                    nums.iter()
                        .skip(1)
                        .fold(nums[0], |acc, &x| acc.wrapping_sub(x)),
                ),
                Instruction::I64Mul => Some(
                    nums.iter()
                        .skip(1)
                        .fold(nums[0], |acc, &x| acc.wrapping_mul(x)),
                ),
                _ => None,
            };
            if let Some(result) = folded {
                return Ok(self.emit_tagged_const_wrapping(result, TAG_NUM));
            }
        }
        let mut v = self.expr(&folded_args[0])?;
        v.extend(self.emit_untag());
        for x in &folded_args[1..] {
            v.extend(self.expr(x)?);
            v.extend(self.emit_untag());
            v.push(op.clone());
        }
        v.extend(self.emit_tag_num());
        Ok(v)
    }

    pub(crate) fn fold_binop_safe(
        &mut self,
        a: &[LispVal],
        _op: Instruction<'static>,
        identity: i64,
        is_div: bool,
    ) -> Result<Vec<Instruction<'static>>, String> {
        if a.is_empty() {
            return self.emit_tagged_const(identity, TAG_NUM);
        }
        let mut v = self.expr(&a[0])?;
        v.extend(self.emit_untag());
        for x in &a[1..] {
            v.extend(self.expr(x)?);
            v.extend(self.emit_untag());
            if is_div {
                v.extend(self.emit_safe_div());
            } else {
                v.extend(self.emit_safe_rem());
            }
        }
        v.extend(self.emit_tag_num());
        Ok(v)
    }

    pub(crate) fn cmp(
        &mut self,
        a: &[LispVal],
        op: Instruction<'static>,
    ) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = self.expr(&a[0])?;
        v.extend(self.emit_untag());
        v.extend(self.expr(&a[1])?);
        v.extend(self.emit_untag());
        v.push(op);
        v.push(Instruction::I64ExtendI32U);
        v.extend(self.emit_tag_bool());
        Ok(v)
    }

    pub(crate) fn eq(&mut self, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = self.expr(&a[0])?;
        v.extend(self.expr(&a[1])?);
        let h = self.ensure_val_eq_helper();
        v.push(Instruction::Call(USER_BASE | h));
        v.extend(self.emit_tag_bool());
        Ok(v)
    }

    pub(crate) fn neq(&mut self, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = self.expr(&a[0])?;
        v.extend(self.expr(&a[1])?);
        let h = self.ensure_val_eq_helper();
        v.push(Instruction::Call(USER_BASE | h));
        // helper returns i64 0/1 (uniform sig); eqz → i32 1 = not-equal
        v.push(Instruction::I64Eqz);
        v.push(Instruction::I64ExtendI32U);
        v.extend(self.emit_tag_bool());
        Ok(v)
    }

    /// __h_val_eq(a, b) -> i64 (1 = equal) — STRUCTURAL equality:
    /// str = len + bytes, array = count + elements (recursive), others = raw
    /// tagged i64 compare. Fixes (= "a" "a") / (= (list 1) (list 1)) being
    /// pointer comparisons (dynamically-built strings were never equal).
    pub(crate) fn ensure_val_eq_helper(&mut self) -> u32 {
        if let Some(idx) = self.val_eq_helper {
            return idx;
        }
        use Instruction as I;
        let idx = self.funcs.len();
        let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
        let ma1 = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
        let mut v: Vec<Instruction<'static>> = Vec::new();
        // locals: 2=tag_a 3=tag_b 4=la/lb/pa 5=lb/pb 6=i 7=res 8=na 9=nb 10=k
        // tag mismatch → 0
        v.push(I::LocalGet(0)); v.push(I::I64Const(7)); v.push(I::I64And); v.push(I::LocalSet(2));
        v.push(I::LocalGet(1)); v.push(I::I64Const(7)); v.push(I::I64And); v.push(I::LocalSet(3));
        v.push(I::LocalGet(2)); v.push(I::LocalGet(3)); v.push(I::I64Ne);
        v.push(I::If(wasm_encoder::BlockType::Empty));
        v.push(I::I64Const(0)); v.push(I::Return);
        v.push(I::End);
        // str? len + byte loop
        v.push(I::LocalGet(2)); v.push(I::I64Const(TAG_STR)); v.push(I::I64Eq);
        v.push(I::If(wasm_encoder::BlockType::Empty));
        v.push(I::LocalGet(0)); v.push(I::I64Const(TAG_BITS)); v.push(I::I64ShrU); v.push(I::I64Const(32)); v.push(I::I64ShrU); v.push(I::LocalSet(4)); // la
        v.push(I::LocalGet(1)); v.push(I::I64Const(TAG_BITS)); v.push(I::I64ShrU); v.push(I::I64Const(32)); v.push(I::I64ShrU); v.push(I::LocalSet(5)); // lb
        v.push(I::LocalGet(4)); v.push(I::LocalGet(5)); v.push(I::I64Ne);
        v.push(I::If(wasm_encoder::BlockType::Empty));
        v.push(I::I64Const(0)); v.push(I::Return);
        v.push(I::End);
        v.push(I::I64Const(0)); v.push(I::LocalSet(6));
        v.push(I::Block(wasm_encoder::BlockType::Empty));
        v.push(I::Loop(wasm_encoder::BlockType::Empty));
        v.push(I::LocalGet(6)); v.push(I::LocalGet(4)); v.push(I::I64GeU); v.push(I::BrIf(1));
        // pa+i vs pb+i byte
        v.push(I::LocalGet(0)); v.push(I::I64Const(TAG_BITS)); v.push(I::I64ShrU); v.push(I::I64Const(0xFFFF_FFFF)); v.push(I::I64And); v.push(I::LocalGet(6)); v.push(I::I64Add); v.push(I::I32WrapI64);
        v.push(I::I32Load8U(ma1.clone()));
        v.push(I::LocalGet(1)); v.push(I::I64Const(TAG_BITS)); v.push(I::I64ShrU); v.push(I::I64Const(0xFFFF_FFFF)); v.push(I::I64And); v.push(I::LocalGet(6)); v.push(I::I64Add); v.push(I::I32WrapI64);
        v.push(I::I32Load8U(ma1.clone()));
        v.push(I::I32Ne);
        v.push(I::If(wasm_encoder::BlockType::Empty));
        v.push(I::I64Const(0)); v.push(I::Return);
        v.push(I::End);
        v.push(I::LocalGet(6)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::LocalSet(6));
        v.push(I::Br(0));
        v.push(I::End); v.push(I::End);
        v.push(I::I64Const(1)); v.push(I::Return);
        v.push(I::End);
        // array? count + elementwise recurse
        v.push(I::LocalGet(2)); v.push(I::I64Const(TAG_ARRAY)); v.push(I::I64Eq);
        v.push(I::If(wasm_encoder::BlockType::Empty));
        v.push(I::LocalGet(0)); v.push(I::I64Const(TAG_BITS)); v.push(I::I64ShrU); v.push(I::I32WrapI64); v.push(I::I64Load(ma8.clone())); v.push(I::LocalSet(8)); // na
        v.push(I::LocalGet(1)); v.push(I::I64Const(TAG_BITS)); v.push(I::I64ShrU); v.push(I::I32WrapI64); v.push(I::I64Load(ma8.clone())); v.push(I::LocalSet(9)); // nb
        v.push(I::LocalGet(8)); v.push(I::LocalGet(9)); v.push(I::I64Ne);
        v.push(I::If(wasm_encoder::BlockType::Empty));
        v.push(I::I64Const(0)); v.push(I::Return);
        v.push(I::End);
        v.push(I::I64Const(0)); v.push(I::LocalSet(6));
        v.push(I::Block(wasm_encoder::BlockType::Empty));
        v.push(I::Loop(wasm_encoder::BlockType::Empty));
        v.push(I::LocalGet(6)); v.push(I::LocalGet(8)); v.push(I::I64GeU); v.push(I::BrIf(1));
        v.push(I::LocalGet(0)); v.push(I::I64Const(TAG_BITS)); v.push(I::I64ShrU); v.push(I::LocalGet(6)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::I64Const(8)); v.push(I::I64Mul); v.push(I::I64Add); v.push(I::I32WrapI64); v.push(I::I64Load(ma8.clone()));
        v.push(I::LocalGet(1)); v.push(I::I64Const(TAG_BITS)); v.push(I::I64ShrU); v.push(I::LocalGet(6)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::I64Const(8)); v.push(I::I64Mul); v.push(I::I64Add); v.push(I::I32WrapI64); v.push(I::I64Load(ma8.clone()));
        v.push(I::Call(USER_BASE | idx as u32));
        v.push(I::I64Eqz);
        v.push(I::If(wasm_encoder::BlockType::Empty));
        v.push(I::I64Const(0)); v.push(I::Return);
        v.push(I::End);
        v.push(I::LocalGet(6)); v.push(I::I64Const(1)); v.push(I::I64Add); v.push(I::LocalSet(6));
        v.push(I::Br(0));
        v.push(I::End); v.push(I::End);
        v.push(I::I64Const(1)); v.push(I::Return);
        v.push(I::End);
        // else: raw compare (num/num, bool, nil, fnref). Bools stay tagged
        // words here — the INTERPRETER is the parity source: (= #t 1) is
        // false (different tags), (= #t #t) is true (same word).
        v.push(I::LocalGet(0)); v.push(I::LocalGet(1)); v.push(I::I64Eq); v.push(I::I64ExtendI32U);
        self.funcs.push(FuncDef {
            name: "__h_val_eq".into(),
            param_count: 2,
            local_count: 11, // locals 2..10 (local_count = highest index + 1)
            instrs: v,
            local_entries: None, custom_type: None,
        });
        self.val_eq_helper = Some(idx as u32);
        idx as u32
    }
}
