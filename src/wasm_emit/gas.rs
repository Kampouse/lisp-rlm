use super::*;

impl WasmEmitter {
    pub(crate) fn gas_check_instrs(gas_local: u32) -> Vec<Instruction<'static>> {
        vec![
            Instruction::LocalGet(gas_local),
            Instruction::I64Const(1),
            Instruction::I64Sub,
            Instruction::LocalTee(gas_local),
            Instruction::I64Const(0),
            Instruction::I64LeS,
            // I64LeS produces i32, use directly for If
            Instruction::If(BlockType::Empty),
            Instruction::Unreachable,
            Instruction::End,
        ]
    }

    pub(crate) fn inject_gas_checks(
        instrs: Vec<Instruction<'static>>,
        gas_local: u32,
    ) -> Vec<Instruction<'static>> {
        let check = Self::gas_check_instrs(gas_local);
        let mut out = Vec::with_capacity(instrs.len() * 2);
        for i in &instrs {
            match i {
                Instruction::Br(_) => {
                    out.extend(check.iter().cloned());
                    out.push(i.clone());
                }
                Instruction::Call(idx) if *idx >= HOST_BASE && *idx < USER_BASE => {
                    out.extend(check.iter().cloned());
                    out.push(i.clone());
                }
                _ => out.push(i.clone()),
            }
        }
        out
    }

    pub(crate) fn peephole(instrs: Vec<Instruction<'static>>) -> Vec<Instruction<'static>> {
        let mut out = Vec::with_capacity(instrs.len());
        let mut i = 0;
        while i < instrs.len() {
            // Pattern: LocalSet(n) followed by LocalGet(n) → remove LocalGet
            // Check 4-instruction patterns first (untag+retag elimination)
            if i + 3 < instrs.len() {
                match (&instrs[i], &instrs[i + 1], &instrs[i + 2], &instrs[i + 3]) {
                    // untag(3) then tag(3): I64Const(3), I64ShrU, I64Const(3), I64Shl → noop
                    (
                        Instruction::I64Const(3),
                        Instruction::I64ShrU,
                        Instruction::I64Const(3),
                        Instruction::I64Shl,
                    ) => {
                        i += 4;
                        continue;
                    }
                    // tag(3) then untag(3): I64Const(3), I64Shl, I64Const(3), I64ShrU → noop
                    (
                        Instruction::I64Const(3),
                        Instruction::I64Shl,
                        Instruction::I64Const(3),
                        Instruction::I64ShrU,
                    ) => {
                        i += 4;
                        continue;
                    }
                    _ => {}
                }
            }
            if i + 1 < instrs.len() {
                match (&instrs[i], &instrs[i + 1]) {
                    (Instruction::LocalSet(n), Instruction::LocalGet(m)) if n == m => {
                        // Replace LocalSet+LocalGet with LocalTee (stores without popping)
                        out.push(Instruction::LocalTee(*n));
                        i += 2;
                        continue;
                    }
                    // Pattern: x, I64Const(0), I64Add → x (additive identity)
                    (Instruction::I64Const(0), Instruction::I64Add) => {
                        i += 2;
                        continue;
                    }
                    // Pattern: x, I64Const(0), I64Or → x (or with zero)
                    (Instruction::I64Const(0), Instruction::I64Or) => {
                        i += 2;
                        continue;
                    }
                    // Pattern: I64Const(0), I64Shl → noop (x << 0 = x)
                    (Instruction::I64Const(0), Instruction::I64Shl) => {
                        i += 2;
                        continue;
                    }
                    // Pattern: I64Const(0), I64ShrU → noop (x >> 0 = x)
                    (Instruction::I64Const(0), Instruction::I64ShrU) => {
                        i += 2;
                        continue;
                    }
                    // Pattern: I64Const(1), I64Sub → can't remove (x - 1 ≠ x), skip
                    _ => {}
                }
            }
            out.push(instrs[i].clone());
            i += 1;
        }
        out
    }
}
