use super::*;

impl WasmEmitter {
    pub(crate) fn call_predicate(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "number?" => {
                if a.len() != 1 { return Err("number?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(7));
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_NUM));
                v.push(Instruction::I64Eq);           // → i32
                v.push(Instruction::I64ExtendI32U);   // → i64
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            "zero?" => {
                if a.len() != 1 { return Err("zero?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.extend(self.emit_untag());
                v.push(Instruction::I64Eqz);          // → i32
                v.push(Instruction::I64ExtendI32U);   // → i64
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            "nil?" => {
                if a.len() != 1 { return Err("nil?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(TAGGED_NIL));
                v.push(Instruction::I64Eq);           // → i32
                v.push(Instruction::I64ExtendI32U);   // → i64
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            "list?" => {
                if a.len() != 1 { return Err("list?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(7));
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_ARRAY));
                v.push(Instruction::I64Eq);           // → i32
                v.push(Instruction::I64ExtendI32U);   // → i64
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            "bool?" => {
                if a.len() != 1 { return Err("bool?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(7));
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_BOOL));
                v.push(Instruction::I64Eq);           // → i32
                v.push(Instruction::I64ExtendI32U);   // → i64
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            "string?" => {
                if a.len() != 1 { return Err("string?: expected 1 arg".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(7));
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(TAG_STR));
                v.push(Instruction::I64Eq);           // → i32
                v.push(Instruction::I64ExtendI32U);   // → i64
                v.extend(self.emit_tag_bool());
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
