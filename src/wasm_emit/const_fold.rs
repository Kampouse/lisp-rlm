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
            return Ok(self.emit_tagged_const(identity, TAG_NUM));
        }
        // Deep constant folding: try to const_eval each arg first
        let folded_args: Vec<LispVal> = a
            .iter()
            .map(|x| self.const_eval(x).unwrap_or_else(|| x.clone()))
            .collect();
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
                Some(result) => return Ok(self.emit_tagged_const(result, TAG_NUM)),
                None => return Err("arithmetic overflow at compile time".into()),
            }
        }
        let mut v = self.expr(&folded_args[0])?;
        v.extend(self.emit_untag());
        for x in &folded_args[1..] {
            v.extend(self.expr(x)?);
            v.extend(self.emit_untag());
            match &op {
                Instruction::I64Add => v.extend(self.emit_checked_add()),
                Instruction::I64Sub => v.extend(self.emit_checked_sub()),
                Instruction::I64Mul => v.extend(self.emit_checked_mul()),
                _ => v.push(op.clone()),
            }
        }
        v.extend(self.emit_tag_num());
        Ok(v)
    }

    pub(crate) fn fold_binop_wrapping(
        &mut self,
        a: &[LispVal],
        op: Instruction<'static>,
        identity: i64,
    ) -> Result<Vec<Instruction<'static>>, String> {
        if a.is_empty() {
            return Ok(self.emit_tagged_const(identity, TAG_NUM));
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
                return Ok(self.emit_tagged_const(result, TAG_NUM));
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
            return Ok(self.emit_tagged_const(identity, TAG_NUM));
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
        v.push(Instruction::I64Eq);
        v.push(Instruction::I64ExtendI32U);
        v.extend(self.emit_tag_bool());
        Ok(v)
    }

    pub(crate) fn neq(&mut self, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        let mut v = self.expr(&a[0])?;
        v.extend(self.expr(&a[1])?);
        v.push(Instruction::I64Ne);
        v.push(Instruction::I64ExtendI32U);
        v.extend(self.emit_tag_bool());
        Ok(v)
    }
}
