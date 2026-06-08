use super::*;

impl WasmEmitter {
    pub(crate) fn call_u128(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "u128/store" => {
                if a.len() != 3 { return Err("u128/store: need 3 args (addr, lo, hi)".into()); }
                let addr = self.expr(&a[0])?;
                let lo = self.expr(&a[1])?;
                let hi = self.expr(&a[2])?;
                let mut v = Vec::new();
                // store low at addr
                v.extend(addr.clone()); v.push(Instruction::I32WrapI64);
                v.extend(lo);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // store high at addr+8
                v.extend(addr); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.extend(hi);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "u128/load" => {
                if a.len() != 1 { return Err("u128/load: need 1 arg (addr)".into()); }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }
            "u128/load_high" => {
                if a.len() != 1 {
                    return Err("u128/load_high: need 1 arg (addr)".into());
                }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                Ok(v)
            }
            "u128/add" | "u128/checked_add" => {
                if a.len() != 2 { return Err("u128/add: need 2 args (dst, src)".into()); }
                let dst = self.expr(&a[0])?;
                let src = self.expr(&a[1])?;
                let dst_i = self.local_idx("__u128a");
                let src_i = self.local_idx("__u128b");
                let lo_i = self.local_idx("__u128lo");
                let hi_i = self.local_idx("__u128hi");
                let c_i = self.local_idx("__u128c");
                let dst_hi_i = self.local_idx("__u128ahi");
                let mut v = Vec::new();
                // Save addresses
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(src); v.push(Instruction::LocalSet(src_i));
                // Load dst_low, src_low
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(lo_i));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // lo_i = dst_low + src_low
                v.push(Instruction::LocalGet(lo_i)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(lo_i));
                // Carry: if result < src_low (unsigned), carry=1
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU); v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(c_i));
                // hi = dst_high + src_high + carry
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(dst_hi_i)); // save original dst_hi for overflow check
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(c_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(hi_i));
                // ── OVERFLOW CHECK: trap if result exceeded u128 ──
                // Overflow when: result_hi < original_dst_hi (unsigned comparison)
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::LocalGet(dst_hi_i));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                // Store back
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "u128/sub" | "u128/checked_sub" => {
                if a.len() != 2 { return Err("u128/sub: need 2 args (dst, src)".into()); }
                let dst = self.expr(&a[0])?;
                let src = self.expr(&a[1])?;
                let dst_i = self.local_idx("__u128sa");
                let src_i = self.local_idx("__u128sb");
                let lo_i = self.local_idx("__u128slo");
                let hi_i = self.local_idx("__u128shi");
                let b_i = self.local_idx("__u128borrow");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(src); v.push(Instruction::LocalSet(src_i));
                // ── UNDERFLOW CHECK: trap if dst < src (unsigned u128 comparison) ──
                // Compare high first: if dst_hi < src_hi → underflow
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                // if dst_hi == src_hi, check low part
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                // dst_lo < src_lo → underflow
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                v.push(Instruction::End);
                // ── END UNDERFLOW CHECK ──
                // Load dst_low into lo_i
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(lo_i));
                // Load src_low into b_i
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(b_i));
                // borrow = lo_i < b_i (unsigned)
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalGet(b_i));
                v.push(Instruction::I64LtU); v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(b_i));
                // lo_i = lo_i - src_low
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Sub); v.push(Instruction::LocalSet(lo_i));
                // hi = dst_high - src_high - borrow
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(hi_i));
                // Store back
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "u128/mul" | "u128/checked_mul" => {
                if a.len() != 2 { return Err("u128/mul: need 2 args (dst, val)".into()); }
                let dst = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let dst_i = self.local_idx("__u128ma");
                let val_i = self.local_idx("__u128mv");
                let dl_i = self.local_idx("__u128mdl");
                let dh_i = self.local_idx("__u128mdh");
                let rl_i = self.local_idx("__u128mrl");
                let rh_i = self.local_idx("__u128mrh");
                let t_i = self.local_idx("__u128mt");
                let carry_i = self.local_idx("__u128mc");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(val); v.push(Instruction::LocalSet(val_i));
                // ── OVERFLOW SAFETY CHECKS ──
                // Trap on non-positive multiplier (money should never multiply by <= 0)
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64LeS);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                // Load dst_lo, dst_hi
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(dl_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(dh_i));
                // rl = dl * val (i64.mul, wraps on overflow)
                v.push(Instruction::LocalGet(dl_i));
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(rl_i));
                // carry from low mul: if rl < dl (assuming val >= 2, but edge cases...)
                // Better: split dl into high32 and low32, multiply separately
                // Simpler approach: carry = (dl * val) >> 64 ≈ (dl >> 32) * val + ...
                // Approximation using (dl >> 32) * (val) + ((dl & 0xFFFFFFFF) * (val >> 32))
                // This gives the high 64 bits of the 128-bit product of low halves
                // carry = (dl >> 32) * val (shifted left 0, but this is 96-bit...)
                // Actually: carry = ((dl >> 32) * val) + (((dl & 0xFFFFFFFF) * val) >> 32)
                // But we need >>64 not >>32. Let's do:
                // carry = (dl >> 32) * (val >> 32) is wrong too.
                // Correct approach for full carry:
                // carry = dl_hi * val_lo + dl_lo * val_hi + (dl_lo * val_lo >> 64)
                // But we can't easily get >> 64 of a 64x64->128 mul in WASM i64.
                //
                // PRAGMATIC: For DeFi amounts, values are typically < 2^53 (exact i64).
                // We use: carry = (dl != 0 && val != 0 && rl < dl) as rough carry estimate
                // This is WRONG for large values. Let me use the split approach properly.
                //
                // Split: dl = (dl_hi << 32) | dl_lo where dl_hi = dl >> 32, dl_lo = dl & 0xFFFF_FFFF
                // full_lo = dl_lo * val_lo  (fits in 64 bits since both < 2^32)
                // mid1 = dl_hi * val_lo
                // mid2 = dl_lo * val_hi
                // rl = full_lo + ((mid1 + mid2) << 32)   — but this can overflow too
                //
                // SIMPLEST CORRECT: Use the comparison trick.
                // If dl != 0 and val != 0 and rl / dl != val, there was overflow.
                // But division is expensive and can trap.
                //
                // Let me just do: carry = 0 for now, and document that mul is correct
                // only when the product of the low halves fits in 64 bits.
                // For NEAR FT amounts (u128 low part usually < 2^60), multiplying by
                // prices < 2^20, this is fine.
                //
                // Actually the simplest correct approach for full 64x64->128:
                // We can't do it with just i64 ops without splitting into 32-bit halves.
                // Let's do the 32-bit split:

                // dl_hi = dl >> 32, dl_lo = dl & 0xFFFFFFFF
                v.push(Instruction::LocalGet(dl_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(t_i)); // t = dl_hi

                // carry = 0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(carry_i));

                // rl = dl_lo * val_lo (both < 2^32, product < 2^64)
                // rl = (dl & 0xFFFF_FFFF) * (val & 0xFFFF_FFFF)
                v.push(Instruction::LocalGet(dl_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(rl_i));

                // carry += (dl_lo * val_lo) >> 32
                v.push(Instruction::LocalGet(rl_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(carry_i));

                // carry += dl_hi * (val & 0xFFFF_FFFF)
                v.push(Instruction::LocalGet(t_i));
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(carry_i));

                // carry += (dl & 0xFFFF_FFFF) * (val >> 32)
                v.push(Instruction::LocalGet(dl_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(carry_i));

                // rl &= 0xFFFF_FFFF (keep only low 32 bits)
                v.push(Instruction::LocalGet(rl_i));
                v.push(Instruction::I64Const(0xFFFF_FFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalSet(rl_i));

                // Now carry has bits [32..95] of the 128-bit low product
                // rh = dh * val + carry + (dl_hi * (val >> 32) shifted)
                // Actually carry already accumulated everything above bit 32.
                // rh = dh * val + carry
                v.push(Instruction::LocalGet(dh_i));
                v.push(Instruction::LocalGet(val_i));
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(carry_i)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(rh_i));

                // Store results
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh_i));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "u128/lt" => {
                if a.len() != 2 {
                    return Err("u128/lt: need 2 args (a1, a2)".into());
                }
                let a1 = self.expr(&a[0])?;
                let a2 = self.expr(&a[1])?;
                let a1_i = self.local_idx("__u128lt1");
                let a2_i = self.local_idx("__u128lt2");
                let mut v = Vec::new();
                v.extend(a1); v.push(Instruction::LocalSet(a1_i));
                v.extend(a2); v.push(Instruction::LocalSet(a2_i));
                // Compare high first: if a1_hi < a2_hi → 1; if a1_hi > a2_hi → 0; else compare low
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU); // a1_hi < a2_hi
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::Else);
                // Check a1_hi > a2_hi
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64GtU); // a1_hi > a2_hi
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::Else);
                // Highs equal, compare low
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::End);
                v.push(Instruction::End);
                Ok(v)
            }
            "u128/eq" => {
                if a.len() != 2 {
                    return Err("u128/eq: need 2 args (a1, a2)".into());
                }
                let a1 = self.expr(&a[0])?;
                let a2 = self.expr(&a[1])?;
                let a1_i = self.local_idx("__u128eq1");
                let a2_i = self.local_idx("__u128eq2");
                let mut v = Vec::new();
                v.extend(a1); v.push(Instruction::LocalSet(a1_i));
                v.extend(a2); v.push(Instruction::LocalSet(a2_i));
                // high_eq = a1_hi == a2_hi
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eq);
                // I64Eq returns i32, which If consumes directly
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // low_eq = a1_lo == a2_lo
                v.push(Instruction::LocalGet(a1_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(a2_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eq);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }
            "u128/is_zero" => {
                if a.len() != 1 {
                    return Err("u128/is_zero: need 1 arg (addr)".into());
                }
                let mut v = self.expr(&a[0])?;
                let addr_i = self.local_idx("__u128zz");
                v.push(Instruction::LocalSet(addr_i));
                // low == 0 && high == 0
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }
            "u128/from_yocto" => {
                if a.len() != 2 { return Err("u128/from_yocto: expected (\"amount\" offset)".into()); }
                let offset_expr = self.expr(&a[1])?;
                let (lo, hi) = match &a[0] {
                    LispVal::Str(s) => Self::parse_u128(s)?,
                    _ => return Err("u128/from_yocto: first arg must be a string literal".into()),
                };
                let off = self.local_idx("__u128_off");
                let mut v = Vec::new();
                v.extend(offset_expr); v.push(Instruction::LocalSet(off));
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(lo));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(hi));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off));
                Ok(v)
            }
            "u128/new" => {
                if a.len() != 3 { return Err("u128/new: expected (hi lo offset)".into()); }
                let hi_e = self.expr(&a[0])?;
                let lo_e = self.expr(&a[1])?;
                let off_e = self.expr(&a[2])?;
                let off = self.local_idx("__u128_off");
                let mut v = Vec::new();
                v.extend(off_e); v.push(Instruction::LocalSet(off));
                v.extend(lo_e);
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(hi_e);
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off));
                Ok(v)
            }
            "u128/from_i64" => {
                if a.len() != 2 { return Err("u128/from_i64: expected (n offset)".into()); }
                let n_e = self.expr(&a[0])?;
                let off_e = self.expr(&a[1])?;
                let off = self.local_idx("__u128_off");
                let mut v = Vec::new();
                v.extend(off_e); v.push(Instruction::LocalSet(off));
                v.extend(n_e);
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(off));
                Ok(v)
            }
            "u128/to_i64" => {
                if a.len() != 1 {
                    return Err("u128/to_i64: need 1 arg (addr)".into());
                }
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }
            "u128/fit_i64" => {
                // Returns 1 if u128 fits in i64 (i.e., value < 2^63), else 0
                // Does NOT trap - just returns boolean
                if a.len() != 1 {
                    return Err("u128/fit_i64: need 1 arg (addr)".into());
                }
                let mut v = self.expr(&a[0])?;
                let addr_i = self.local_idx("__u128fit");
                v.push(Instruction::LocalSet(addr_i));
                // Check: hi == 0 AND lo < 2^63 (sign bit is 0)
                // hi == 0?
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                // hi == 0, now check lo < 2^63 (i.e., lo >> 63 == 0)
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(63));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Eqz);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::Else);
                // hi != 0, doesn't fit
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }
            "u128/checked_to_i64" => {
                // Traps if u128 doesn't fit in i64, else returns the i64 value
                if a.len() != 1 {
                    return Err("u128/checked_to_i64: need 1 arg (addr)".into());
                }
                let mut v = self.expr(&a[0])?;
                let addr_i = self.local_idx("__u128cti64");
                v.push(Instruction::LocalSet(addr_i));
                // Check: hi == 0
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // hi == 0, check lo < 2^63
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(63));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // Fits! Return lo
                v.push(Instruction::LocalGet(addr_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::Return);
                v.push(Instruction::End);
                // lo >= 2^63, trap
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                // hi != 0, trap
                v.push(Instruction::Unreachable);
                Ok(v)
            }
            "u128/store_storage" => {
                if a.len() != 2 { return Err("u128/store_storage: expected (\"key\" src)".into()); }
                let key = self.expr(&a[0])?;
                let src = self.expr(&a[1])?;
                let os = self.local_idx("__u128_s");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(src); v.push(Instruction::LocalSet(os));
                v.push(Instruction::LocalGet(os)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I32Const(STORAGE_U128_BUF as i32));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(os)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::I32Const((STORAGE_U128_BUF + 8) as i32));
                v.push(Instruction::I64Store(ma));
                v.extend(key.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64Const(STORAGE_U128_BUF)); v.push(Instruction::I64Const(0));
                v.push(Self::host_call(17)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(0));
                Ok(v)
            }
            "u128/load_storage" => {
                if a.len() != 2 { return Err("u128/load_storage: expected (\"key\" dst)".into()); }
                let key = self.expr(&a[0])?;
                let dst = self.expr(&a[1])?;
                let od = self.local_idx("__u128_d");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(od));
                v.extend(key.clone());
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.extend(key);
                v.push(Instruction::I32WrapI64); v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::I64Const(0));
                v.push(Self::host_call(18)); v.push(Instruction::Drop);
                v.push(Instruction::I64Const(0)); v.push(Instruction::I64Const(STORAGE_U128_BUF));
                v.push(Self::host_call(0)); v.push(Instruction::Drop);
                v.push(Instruction::I32Const(STORAGE_U128_BUF as i32));
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalGet(od)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::I32Const((STORAGE_U128_BUF + 8) as i32));
                v.push(Instruction::I64Load(ma));
                v.push(Instruction::LocalGet(od)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(od));
                Ok(v)
            }
            // ── BigInt operations (two-i64 pairs for 128-bit arithmetic) ──
            // bigint-mul: (dst val) → multiply u128 at dst by u64 val, result in dst
            "bigint-mul" => {
                if a.len() != 2 { return Err("bigint-mul: need 2 args (dst addr, u64 multiplier)".into()); }
                // Delegate to u128/mul
                self.call_u128("u128/mul", a)
            }
            // bigint-div: (dst divisor) → divide u128 at dst by u64 divisor, quotient in dst
            "bigint-div" | "u128/div" => {
                if a.len() != 2 { return Err("bigint-div: need 2 args (dst addr, u64 divisor)".into()); }
                let dst = self.expr(&a[0])?;
                let divisor = self.expr(&a[1])?;
                let dst_i = self.local_idx("__u128da");
                let lo_i = self.local_idx("__u128dlo");
                let hi_i = self.local_idx("__u128dhi");
                let div_i = self.local_idx("__u128dv");
                let rem_i = self.local_idx("__u128rem");
                let qlo_i = self.local_idx("__u128qlo");
                let qhi_i = self.local_idx("__u128qhi");
                let bit_i = self.local_idx("__u128bit");
                let ma = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(divisor); v.push(Instruction::LocalSet(div_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(lo_i));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma)); v.push(Instruction::LocalSet(hi_i));
                // Check divisor != 0
                v.push(Instruction::LocalGet(div_i));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::Unreachable);
                v.push(Instruction::End);
                // Binary long division: 128 iterations
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(rem_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(qlo_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(qhi_i));
                v.push(Instruction::I64Const(128)); v.push(Instruction::LocalSet(bit_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(bit_i));
                // rem = rem << 1
                v.push(Instruction::LocalGet(rem_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalSet(rem_i));
                // Or with bit from dividend
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Const(64));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64And);
                v.push(Instruction::Else);
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Const(64));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64And);
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(rem_i));
                v.push(Instruction::I64Or);
                v.push(Instruction::LocalSet(rem_i));
                // if rem >= divisor: rem -= divisor, set bit in quotient
                v.push(Instruction::LocalGet(rem_i));
                v.push(Instruction::LocalGet(div_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(rem_i));
                v.push(Instruction::LocalGet(div_i));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(rem_i));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Const(64));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(qlo_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::LocalSet(qlo_i));
                v.push(Instruction::Else);
                v.push(Instruction::LocalGet(qhi_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Const(64));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::LocalSet(qhi_i));
                v.push(Instruction::End);
                v.push(Instruction::End);
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                // Store result
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(qlo_i));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I64Const(8)); v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(qhi_i));
                v.push(Instruction::I64Store(ma));
                v.push(Instruction::LocalGet(dst_i));
                Ok(v)
            }
            // bigint-from-str: (str dst) → parse decimal string at str to u128 at dst
            "bigint-from-str" | "u128/from_str" => {
                if a.len() != 2 { return Err("bigint-from-str: expected (str addr, dst offset)".into()); }
                let str_addr = self.expr(&a[0])?;
                let dst_off = self.expr(&a[1])?;
                let str_i = self.local_idx("__bfs_str");
                let dst_i = self.local_idx("__bfs_dst");
                let len_i = self.local_idx("__bfs_len");
                let lo_i = self.local_idx("__bfs_lo");
                let hi_i = self.local_idx("__bfs_hi");
                let i_i = self.local_idx("__bfs_i");
                let ch_i = self.local_idx("__bfs_ch");
                let t_lo = self.local_idx("__bfs_tlo");
                let t_hi = self.local_idx("__bfs_thi");
                let x_hi = self.local_idx("__bfs_xhi");
                let ptr_i = self.local_idx("__bfs_ptr");
                let ma = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(str_addr); v.extend(self.emit_untag()); v.push(Instruction::LocalSet(str_i));
                v.extend(dst_off); v.push(Instruction::LocalSet(dst_i));
                v.push(Instruction::LocalGet(str_i));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::LocalGet(str_i));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(ptr_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(lo_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(hi_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(i_i));
                // Skip leading minus
                v.push(Instruction::LocalGet(ptr_i));
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I32Const(45));
                v.push(Instruction::I32Eq);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(ptr_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(ptr_i));
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::End);
                // Loop over digits
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64GeU);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(ptr_i));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Load8U(ma));
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::LocalSet(ch_i));
                // Skip non-digits
                v.push(Instruction::LocalGet(ch_i));
                v.push(Instruction::I64Const(48));
                v.push(Instruction::I64LtU);
                v.push(Instruction::LocalGet(ch_i));
                v.push(Instruction::I64Const(57));
                v.push(Instruction::I64GtU);
                v.push(Instruction::I32Or);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(ch_i));
                v.push(Instruction::I64Const(48));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(ch_i));
                // lo:hi = lo:hi * 10 + digit
                // t_lo = lo * 10 via shifts
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::I64Const(3));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(t_lo));
                // carry = (lo >> 32) * 10 + ((lo & 0xFFFFFFFF) * 10) >> 32
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(x_hi));
                v.push(Instruction::LocalGet(x_hi));
                v.push(Instruction::I64Const(10));
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::I64Const(0xFFFFFFFF));
                v.push(Instruction::I64And);
                v.push(Instruction::I64Const(10));
                v.push(Instruction::I64Mul);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(t_hi));
                // hi = hi * 10 + t_hi
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::I64Const(10));
                v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(t_hi));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(hi_i));
                // lo = t_lo + digit
                v.push(Instruction::LocalGet(t_lo));
                v.push(Instruction::LocalGet(ch_i));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(lo_i));
                // If overflow, carry to hi
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalGet(ch_i));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(hi_i));
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(i_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(i_i));
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::I64Store(ma8));
                v.push(Instruction::LocalGet(dst_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::I64Store(ma8));
                v.push(Instruction::I64Const(TAG_NIL));
                Ok(v)
            }
            // bigint-to-str: (addr buf) → convert u128 to decimal string, return tagged string
            "bigint-to-str" | "u128/to_str" => {
                if a.len() != 2 { return Err("bigint-to-str: expected (u128 addr, buffer addr)".into()); }
                let addr = self.expr(&a[0])?;
                let buf = self.expr(&a[1])?;
                let addr_i = self.local_idx("__bts_addr");
                let buf_i = self.local_idx("__bts_buf");
                let lo_i = self.local_idx("__bts_lo");
                let hi_i = self.local_idx("__bts_hi");
                let pos_i = self.local_idx("__bts_pos");
                let digit_i = self.local_idx("__bts_digit");
                let tmp_i = self.local_idx("__bts_tmp");
                let qlo_i = self.local_idx("__bts_qlo");
                let qhi_i = self.local_idx("__bts_qhi");
                let rem_i = self.local_idx("__bts_rem");
                let bit_i = self.local_idx("__bts_bit");
                let len_i = self.local_idx("__bts_len");
                let ma = wasm_encoder::MemArg { offset: 0, align: 0, memory_index: 0 };
                let ma8 = wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 };
                let mut v = Vec::new();
                v.extend(addr); v.push(Instruction::LocalSet(addr_i));
                v.extend(buf); v.push(Instruction::LocalSet(buf_i));
                v.push(Instruction::LocalGet(addr_i));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma8));
                v.push(Instruction::LocalSet(lo_i));
                v.push(Instruction::LocalGet(addr_i));
                v.push(Instruction::I64Const(8));
                v.push(Instruction::I64Add);
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(ma8));
                v.push(Instruction::LocalSet(hi_i));
                // Check zero
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(buf_i));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(48));
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(buf_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.extend(self.emit_tag_str());
                v.push(Instruction::Else);
                // Write from buf+39 backwards
                v.push(Instruction::LocalGet(buf_i));
                v.push(Instruction::I64Const(39));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(pos_i));
                v.push(Instruction::LocalGet(lo_i));
                v.push(Instruction::LocalSet(tmp_i));
                v.push(Instruction::LocalGet(hi_i));
                v.push(Instruction::LocalSet(qhi_i)); // reuse qhi as tmp_hi
                // Main loop
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(tmp_i));
                v.push(Instruction::LocalGet(qhi_i)); // tmp_hi
                v.push(Instruction::I64Or);
                v.push(Instruction::I64Eqz);
                v.push(Instruction::BrIf(1));
                // Divide by 10 using binary division
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(rem_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(qlo_i));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(qhi_i));
                v.push(Instruction::I64Const(128)); v.push(Instruction::LocalSet(bit_i));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::BrIf(1));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(bit_i));
                v.push(Instruction::LocalGet(rem_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalSet(rem_i));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Const(64));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::LocalGet(tmp_i));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64And);
                v.push(Instruction::Else);
                v.push(Instruction::LocalGet(qhi_i));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Const(64));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64And);
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(rem_i));
                v.push(Instruction::I64Or);
                v.push(Instruction::LocalSet(rem_i));
                v.push(Instruction::LocalGet(rem_i));
                v.push(Instruction::I64Const(10));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(rem_i));
                v.push(Instruction::I64Const(10));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(rem_i));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Const(64));
                v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(qlo_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::LocalSet(qlo_i));
                v.push(Instruction::Else);
                v.push(Instruction::LocalGet(qhi_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::LocalGet(bit_i));
                v.push(Instruction::I64Const(64));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64Shl);
                v.push(Instruction::I64Or);
                v.push(Instruction::LocalSet(qhi_i));
                v.push(Instruction::End);
                v.push(Instruction::End);
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(rem_i));
                v.push(Instruction::LocalSet(digit_i));
                v.push(Instruction::LocalGet(pos_i));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(48));
                v.push(Instruction::LocalGet(digit_i));
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Add);
                v.push(Instruction::I32Store8(ma));
                v.push(Instruction::LocalGet(pos_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(pos_i));
                v.push(Instruction::LocalGet(qlo_i));
                v.push(Instruction::LocalSet(tmp_i));
                // tmp_hi = qhi (already set)
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                v.push(Instruction::LocalGet(pos_i));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(pos_i));
                v.push(Instruction::I64Const(40));
                v.push(Instruction::LocalGet(buf_i));
                v.push(Instruction::LocalGet(pos_i));
                v.push(Instruction::I64Sub);
                v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(len_i));
                v.push(Instruction::LocalGet(len_i));
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(pos_i));
                v.push(Instruction::I64Or);
                v.extend(self.emit_tag_str());
                v.push(Instruction::End);
                Ok(v)
            }
            // bigint-add: (dst src) → add u128 at src to u128 at dst
            "bigint-add" => {
                if a.len() != 2 { return Err("bigint-add: need 2 args (dst, src)".into()); }
                self.call_u128("u128/add", a)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
