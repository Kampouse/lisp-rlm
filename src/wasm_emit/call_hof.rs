use super::*;

impl WasmEmitter {
    pub(crate) fn call_hof(
        &mut self,
        op: &str,
        a: &[LispVal],
    ) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "range-reduce" => {
                if a.len() < 5 {
                    return Err(
                        "range-reduce: need (range-reduce init start end acc_var body)".into(),
                    );
                }
                let LispVal::Sym(acc_var) = &a[3] else {
                    return Err("reduce: acc must be symbol".into());
                };
                let acc_idx = self.local_idx(acc_var);
                let it_idx = self.local_idx("__it");
                // Both acc and it are stored TAGGED so body can read them normally.
                // The body result is untagged for accumulation, then re-tagged.
                let mut v = Vec::new();
                // acc = init (tagged)
                v.extend(self.expr(&a[0])?);
                v.push(Instruction::LocalSet(acc_idx));
                // it = start (tagged)
                v.extend(self.expr(&a[1])?);
                v.push(Instruction::LocalSet(it_idx));
                // while loop
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                // untag(it) >= untag(end) → exit with acc (already tagged)
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.emit_untag());
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(acc_idx));
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // acc = body (untag body result, re-tag for storage)
                v.extend(self.expr(&a[4])?);
                v.extend(self.emit_untag());
                v.extend(self.emit_tag_num());
                v.push(Instruction::LocalSet(acc_idx));
                // it += 1 (tagged: add 8 = 1<<3 since TAG_NUM=0)
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(8)); // tagged increment
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(TAG_NIL)); // fallback
                v.push(Instruction::End); // block
                Ok(v)
            }
            "map-into" => {
                if a.len() < 4 {
                    return Err("map-into: need (map-into offset start end body)".into());
                }
                let it_idx = self.local_idx("__it");
                let off_idx = self.local_idx("__off");
                let count_idx = self.local_idx("__count");
                let mut v = Vec::new();
                // off = mem_offset (untag), it = start (untag), count = 0
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(off_idx));
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                // it >= end → exit
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                // return count as tagged Num
                v.push(Instruction::LocalGet(count_idx));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // mem[off] = body(it) — store untagged value
                v.push(Instruction::LocalGet(off_idx));
                v.push(Instruction::I32WrapI64);
                v.extend(self.expr(&a[3])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                }));
                // off += 8, it += 1, count += 1
                v.push(Instruction::LocalGet(off_idx));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(off_idx));
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::End); // block
                Ok(v)
            }
            "filter-count" => {
                if a.len() < 3 {
                    return Err("filter-count: need (filter-count start end pred)".into());
                }
                let it_idx = self.local_idx("__it");
                let count_idx = self.local_idx("__count");
                let mut v = Vec::new();
                v.extend(self.expr(&a[0])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // if pred(it): count += 1 (use tagged truthiness)
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_cond_branch());
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End); // loop
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::End); // block
                Ok(v)
            }
            "hof/map" => {
                if a.len() < 3 {
                    return Err(
                        "hof/map: need (hof/map (lambda (x) body) start end [offset])".into(),
                    );
                }
                let (param, body) = Self::extract_lambda(&a[0])?;
                let param_idx = self.local_idx(&param);
                let it_idx = self.local_idx("__hof_it");
                let count_idx = self.local_idx("__hof_count");
                let out_offset = if a.len() > 3 {
                    match &a[3] {
                        LispVal::Num(n) => *n as i64,
                        _ => return Err("hof/map: offset must be number".into()),
                    }
                } else {
                    2048i64
                };
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let tmp = self.local_idx("__hof_tmp");
                let mut v = Vec::new();
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // param = tagged(it) — pass tagged value to lambda
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.emit_tag_num());
                v.push(Instruction::LocalSet(param_idx));
                v.extend(self.expr(&body)?);
                v.push(Instruction::LocalSet(tmp));
                // Store untagged result
                v.push(Instruction::I64Const(out_offset));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(tmp));
                v.extend(self.emit_untag());
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::End);
                Ok(v)
            }
            "hof/filter" => {
                if a.len() < 3 {
                    return Err(
                        "hof/filter: need (hof/filter (lambda (x) pred) start end [offset])".into(),
                    );
                }
                let (param, body) = Self::extract_lambda(&a[0])?;
                let param_idx = self.local_idx(&param);
                let it_idx = self.local_idx("__hof_it");
                let count_idx = self.local_idx("__hof_count");
                let out_offset = if a.len() > 3 {
                    match &a[3] {
                        LispVal::Num(n) => *n as i64,
                        _ => return Err("hof/filter: offset must be number".into()),
                    }
                } else {
                    2048i64
                };
                let ma = wasm_encoder::MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                };
                let mut v = Vec::new();
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(count_idx));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // param = tagged(it) — pass tagged value to lambda
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.emit_tag_num());
                v.push(Instruction::LocalSet(param_idx));
                v.extend(self.expr(&body)?);
                v.extend(self.emit_cond_branch());
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::I64Const(out_offset));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                // Store untagged it value
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(count_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(count_idx));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::End);
                Ok(v)
            }
            "hof/reduce" => {
                if a.len() < 4 {
                    return Err(
                        "hof/reduce: need (hof/reduce (lambda (acc x) body) init start end)".into(),
                    );
                }
                let (params, body) = Self::extract_lambda_2param(&a[0])?;
                let acc_idx = self.local_idx(&params[0]);
                let param_idx = self.local_idx(&params[1]);
                let it_idx = self.local_idx("__hof_it");
                let mut v = Vec::new();
                // acc = init (untagged), it = start (untagged)
                v.extend(self.expr(&a[1])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(acc_idx));
                v.extend(self.expr(&a[2])?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Block(BlockType::Result(ValType::I64)));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.expr(&a[3])?);
                v.extend(self.emit_untag());
                v.push(Instruction::I64GeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(acc_idx));
                v.extend(self.emit_tag_num());
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                // param = tagged(it), acc = tagged(acc)
                v.push(Instruction::LocalGet(it_idx));
                v.extend(self.emit_tag_num());
                v.push(Instruction::LocalSet(param_idx));
                v.push(Instruction::LocalGet(acc_idx));
                v.extend(self.emit_tag_num());
                v.push(Instruction::LocalSet(acc_idx));
                // body result → untag for accumulation
                v.extend(self.expr(&body)?);
                v.extend(self.emit_untag());
                v.push(Instruction::LocalSet(acc_idx));
                v.push(Instruction::LocalGet(it_idx));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(it_idx));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::I64Const(TAG_NIL));
                v.push(Instruction::End);
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
