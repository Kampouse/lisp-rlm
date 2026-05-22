use super::*;

impl WasmEmitter {
    pub(crate) fn call_bitwise(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "clz" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Clz);
                Ok(v)
            }
            "ctz" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Ctz);
                Ok(v)
            }
            "popcnt" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Popcnt);
                Ok(v)
            }
            "bit_get" => {
                let x = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(x);
                v.extend(idx);
                v.push(Instruction::I64ShrU); // x >> idx
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64And); // & 1
                Ok(v)
            }
            "bit_set" => {
                let x = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(x);
                v.push(Instruction::I64Const(1));
                v.extend(idx);
                v.push(Instruction::I64Shl); // 1 << idx
                v.push(Instruction::I64Or); // x | (1 << idx)
                Ok(v)
            }
            "bit_clr" => {
                let x = self.expr(&a[0])?;
                let idx = self.expr(&a[1])?;
                let mut v = Vec::new();
                v.extend(x);
                v.push(Instruction::I64Const(1));
                v.extend(idx);
                v.push(Instruction::I64Shl); // 1 << idx
                v.push(Instruction::I64Const(-1i64)); // all ones
                v.push(Instruction::I64Xor); // ~(1 << idx)
                v.push(Instruction::I64And); // x & ~(1 << idx)
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
