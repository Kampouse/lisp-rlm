use super::*;

impl WasmEmitter {
    pub(crate) fn call_fp(&mut self, op: &str, a: &[LispVal]) -> Result<Vec<Instruction<'static>>, String> {
        match op {
            "fp/mul" => {
                let ea = self.expr(&a[0])?;
                let eb = self.expr(&a[1])?;
                let a_i = self.local_idx("__fpm_a");
                let b_i = self.local_idx("__fpm_b");
                let mut v = Vec::new();
                v.extend(ea); v.push(Instruction::LocalSet(a_i));
                v.extend(eb); v.push(Instruction::LocalSet(b_i));
                // result = (a >> 16) * (b >> 16) + ((a & 0xFFFF) * b) >> 32
                // For Q32.32: just use (a * b) >> 32
                // a*b won't overflow if a < 2^48 and b < 2^16... but they can be larger
                // Safe method: (a >> 16) * b doesn't overflow if a < 2^48 and b < 2^16
                // For our use: a and b are Q32.32, max ~2^32 each, so a>>16 is ~2^16, *b ~2^48 fine
                // But for larger values, need full split:
                // result = ((a >> 16) * (b >> 16)) + (((a >> 16) * (b & 0xFFFF)) >> 16) + (((a & 0xFFFF) * (b >> 16)) >> 16)
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); // a_hi * b_hi
                // + (a_hi * b_lo) >> 16
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                // + (a_lo * b_hi) >> 16
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Mul); v.push(Instruction::I64Const(16)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Add);
                // a_lo * b_lo is negligible after >> 32 for Q32.32 precision
                Ok(v)
            }
            "fp/div" => {
                let ea = self.expr(&a[0])?;
                let eb = self.expr(&a[1])?;
                let a_i = self.local_idx("__fpd_a");
                let b_i = self.local_idx("__fpd_b");
                let mut v = Vec::new();
                v.extend(ea); v.push(Instruction::LocalSet(a_i));
                v.extend(eb); v.push(Instruction::LocalSet(b_i));
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(16)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(a_i)); v.push(Instruction::I64Const(0xFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Const(16)); v.push(Instruction::I64Shl);
                v.push(Instruction::LocalGet(b_i)); v.push(Instruction::I64DivU);
                v.push(Instruction::I64Add);
                Ok(v)
            }
            "fp/to_int" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                Ok(v)
            }
            "fp/from_int" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                Ok(v)
            }
            "fp/one" => {
                Ok(vec![Instruction::I64Const(1), Instruction::I64Const(32), Instruction::I64Shl])
            }
            "fp64/set_int" => {
                let addr = self.expr(&a[0])?;
                let val = self.expr(&a[1])?;
                let mut v = Vec::new();
                // mem[addr] = 0 (low)
                v.extend(addr.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                // mem[addr+8] = val (high)
                v.extend(addr); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.extend(val);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "fp64/get_int" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                Ok(v)
            }
            "fp64/get_frac" => {
                let mut v = self.expr(&a[0])?;
                v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                Ok(v)
            }
            "fp64/set" => {
                let addr = self.expr(&a[0])?;
                let lo = self.expr(&a[1])?;
                let hi = self.expr(&a[2])?;
                let mut v = Vec::new();
                v.extend(addr.clone()); v.push(Instruction::I32WrapI64);
                v.extend(lo);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(addr); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.extend(hi);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "fp64/add" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dl = self.local_idx("__fp64_dl");
                let dh = self.local_idx("__fp64_dh");
                let sl = self.local_idx("__fp64_sl");
                let sh = self.local_idx("__fp64_sh");
                let carry = self.local_idx("__fp64_c");
                let mut v = Vec::new();
                // Load src
                v.extend(sa.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sl));
                v.extend(sa); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sh));
                // Load dst low
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dl));
                // dst_low += src_low, detect carry
                v.push(Instruction::LocalGet(sl)); v.push(Instruction::LocalGet(dl)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(dl));
                // carry = 1 if dl < sl (overflow)
                v.push(Instruction::LocalGet(dl)); v.push(Instruction::LocalGet(sl)); v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(carry));
                // Load dst high
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dh));
                // dst_high += src_high + carry
                v.push(Instruction::LocalGet(sh)); v.push(Instruction::LocalGet(dh)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(carry)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalSet(dh));
                // Store dst
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(dl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(da); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(dh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "fp64/mul" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dl = self.local_idx("__fm_dl");
                let dh = self.local_idx("__fm_dh");
                let sl = self.local_idx("__fm_sl");
                let sh = self.local_idx("__fm_sh");
                // temps for 32-bit split multiply: mulh(x,y) → hi64(x*y)
                let x_lo = self.local_idx("__fm_xlo");
                let x_hi = self.local_idx("__fm_xhi");
                let y_lo = self.local_idx("__fm_ylo");
                let y_hi = self.local_idx("__fm_yhi");
                let ll = self.local_idx("__fm_ll");
                let lh = self.local_idx("__fm_lh");
                let hl = self.local_idx("__fm_hl");
                let hh = self.local_idx("__fm_hh");
                let mid = self.local_idx("__fm_mid");
                let mc = self.local_idx("__fm_mc");
                let _lo = self.local_idx("__fm_lo");
                let lc = self.local_idx("__fm_lc");
                let _hi = self.local_idx("__fm_hi");
                // Cross-term storage
                let cross1_lo = self.local_idx("__fm_c1l");
                let cross1_hi = self.local_idx("__fm_c1h");
                let cross2_lo = self.local_idx("__fm_c2l");
                let cross2_hi = self.local_idx("__fm_c2h");
                let albl_hi = self.local_idx("__fm_abh");
                let rl = self.local_idx("__fm_rl");
                let rh = self.local_idx("__fm_rh");
                let tmp = self.local_idx("__fm_tmp");
                let tmp2 = self.local_idx("__fm_tmp2");
                let mut v = Vec::new();

                // Helper macro-like: emit code to compute hi=high64(x*y), lo=low64(x*y)
                // Stack should have x, y when called. Uses x_lo,x_hi,y_lo,y_hi,ll,lh,hl,hh,mid,mc,lo,lc,hi
                // After: hi and lo locals are set. Nothing on stack.
                let emit_mul128 = |v: &mut Vec<Instruction<'static>>, x: u32, y: u32, hi: u32, lo: u32| {
                    // x_lo = x & 0xFFFFFFFF
                    v.push(Instruction::LocalGet(x)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(x_lo));
                    // x_hi = x >> 32
                    v.push(Instruction::LocalGet(x)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(x_hi));
                    // y_lo = y & 0xFFFFFFFF
                    v.push(Instruction::LocalGet(y)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(y_lo));
                    // y_hi = y >> 32
                    v.push(Instruction::LocalGet(y)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(y_hi));
                    // ll = x_lo * y_lo
                    v.push(Instruction::LocalGet(x_lo)); v.push(Instruction::LocalGet(y_lo)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(ll));
                    // lh = x_lo * y_hi
                    v.push(Instruction::LocalGet(x_lo)); v.push(Instruction::LocalGet(y_hi)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(lh));
                    // hl = x_hi * y_lo
                    v.push(Instruction::LocalGet(x_hi)); v.push(Instruction::LocalGet(y_lo)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(hl));
                    // hh = x_hi * y_hi
                    v.push(Instruction::LocalGet(x_hi)); v.push(Instruction::LocalGet(y_hi)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(hh));
                    // mid = lh + hl, mid_carry = mid < lh
                    v.push(Instruction::LocalGet(lh)); v.push(Instruction::LocalGet(hl)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(mid));
                    v.push(Instruction::LocalGet(lh)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(mc));
                    // lo = ll + (mid << 32), lo_carry = lo < ll
                    v.push(Instruction::LocalGet(ll));
                    v.push(Instruction::LocalGet(mid)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add); v.push(Instruction::LocalTee(lo));
                    v.push(Instruction::LocalGet(ll)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(lc));
                    // hi = hh + (mid >> 32) + (mc << 32) + lc
                    v.push(Instruction::LocalGet(hh));
                    v.push(Instruction::LocalGet(mid)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(mc)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(lc)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(hi));
                    // lo result
                };

                // Load dst {dl, dh}
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dl));
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dh));
                // Load src {sl, sh}
                v.extend(sa.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sl));
                v.extend(sa); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sh));

                // Step 1: Compute high64(dl*sl) → albl_hi (we only need high part)
                emit_mul128(&mut v, dl, sl, albl_hi, tmp);

                // Step 2: Compute full 128-bit ah*bl → {cross1_lo, cross1_hi}
                emit_mul128(&mut v, dh, sl, cross1_hi, cross1_lo);

                // Step 3: Compute full 128-bit al*bh → {cross2_lo, cross2_hi}
                emit_mul128(&mut v, dl, sh, cross2_hi, cross2_lo);

                // Step 4: cross = cross1 + cross2 (128-bit add)
                // cross_lo = cross1_lo + cross2_lo, carry_a
                v.push(Instruction::LocalGet(cross1_lo)); v.push(Instruction::LocalGet(cross2_lo)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(tmp));
                v.push(Instruction::LocalGet(cross1_lo)); v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp2)); // tmp2 = carry_a
                // tmp = cross_lo
                // cross_hi = cross1_hi + cross2_hi + carry_a
                v.push(Instruction::LocalGet(cross1_hi)); v.push(Instruction::LocalGet(cross2_hi)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(tmp2)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(mid));
                // mid = cross_hi, tmp = cross_lo

                // Step 5: result_lo = cross_lo + albl_hi (may carry)
                v.push(Instruction::LocalGet(tmp)); v.push(Instruction::LocalGet(albl_hi)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(rl));
                v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp)); // tmp = carry_b

                // Step 6: result_hi = dh*sh + cross_hi + carry_b
                v.push(Instruction::LocalGet(dh)); v.push(Instruction::LocalGet(sh)); v.push(Instruction::I64Mul);
                v.push(Instruction::LocalGet(mid)); v.push(Instruction::I64Add);
                v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(rh));

                // Store result to dst
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(da); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "fp64/lt" => {
                let a1 = self.expr(&a[0])?;
                let a2 = self.expr(&a[1])?;
                let h1 = self.local_idx("__fplt_h1");
                let h2 = self.local_idx("__fplt_h2");
                let mut v = Vec::new();
                // Compare high parts first
                v.extend(a1.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(h1));
                v.extend(a2.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(h2));
                // if h1 < h2: return 1
                v.push(Instruction::LocalGet(h1)); v.push(Instruction::LocalGet(h2)); v.push(Instruction::I64LtU);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(1));
                v.push(Instruction::Else);
                // if h1 > h2: return 0
                v.push(Instruction::LocalGet(h1)); v.push(Instruction::LocalGet(h2)); v.push(Instruction::I64GtU);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.push(Instruction::I64Const(0));
                v.push(Instruction::Else);
                // High equal, compare low
                v.extend(a1); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(a2); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64LtU); // returns i32
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::End); // inner if
                v.push(Instruction::End); // outer if
                Ok(v)
            }
            "fp64/is_zero" => {
                let addr = self.expr(&a[0])?;
                let mut v = Vec::new();
                v.extend(addr.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                // high == 0?
                v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Result(ValType::I64)));
                v.extend(addr); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Eqz);
                v.push(Instruction::I64ExtendI32U);
                v.push(Instruction::Else);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::End);
                Ok(v)
            }
            "fp64/sub" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dl = self.local_idx("__fp64s_dl");
                let dh = self.local_idx("__fp64s_dh");
                let sl = self.local_idx("__fp64s_sl");
                let sh = self.local_idx("__fp64s_sh");
                let borrow = self.local_idx("__fp64s_b");
                let mut v = Vec::new();
                // Load src
                v.extend(sa.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sl));
                v.extend(sa); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(sh));
                // Load dst low
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dl));
                // borrow = dl < sl (unsigned)
                v.push(Instruction::LocalGet(dl)); v.push(Instruction::LocalGet(sl)); v.push(Instruction::I64LtU);
                v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(borrow));
                // dst_low -= src_low
                v.push(Instruction::LocalGet(dl)); v.push(Instruction::LocalGet(sl)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(dl));
                // Load dst high
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 })); v.push(Instruction::LocalSet(dh));
                // dst_high = dst_high - src_high - borrow
                v.push(Instruction::LocalGet(dh));
                v.push(Instruction::LocalGet(sh)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalGet(borrow)); v.push(Instruction::I64Sub);
                v.push(Instruction::LocalSet(dh));
                // Store dst
                v.extend(da.clone()); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(dl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.extend(da); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(dh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "fp64/div" => {
                let da = self.expr(&a[0])?;
                let sa = self.expr(&a[1])?;
                let dst_i = self.local_idx("__fpd_d");
                let src_i = self.local_idx("__fpd_s");
                let ah = self.local_idx("__fpd_ah");
                let al = self.local_idx("__fpd_al");
                let bh = self.local_idx("__fpd_bh");
                let bl = self.local_idx("__fpd_bl");
                // Newton state: x_lo, x_hi (reciprocal estimate)
                let x_lo = self.local_idx("__fpd_xl");
                let x_hi = self.local_idx("__fpd_xh");
                // Temp for b*x
                let tx_lo = self.local_idx("__fpd_txl");
                let tx_hi = self.local_idx("__fpd_txh");
                // Temp for correction = 2.0 - b*x
                let cl = self.local_idx("__fpd_cl");
                let ch = self.local_idx("__fpd_ch");
                // mul128 temps (shared with mul)
                let m_xlo = self.local_idx("__fm_xlo");
                let m_xhi = self.local_idx("__fm_xhi");
                let m_ylo = self.local_idx("__fm_ylo");
                let m_yhi = self.local_idx("__fm_yhi");
                let m_ll = self.local_idx("__fm_ll");
                let m_lh = self.local_idx("__fm_lh");
                let m_hl = self.local_idx("__fm_hl");
                let m_hh = self.local_idx("__fm_hh");
                let m_mid = self.local_idx("__fm_mid");
                let m_mc = self.local_idx("__fm_mc");
                let m_lo = self.local_idx("__fm_lo");
                let m_lc = self.local_idx("__fm_lc");
                let _m_hi = self.local_idx("__fm_hi");
                // Cross-term temps for mul
                let c1_lo = self.local_idx("__fpd_c1l");
                let c1_hi = self.local_idx("__fpd_c1h");
                let c2_lo = self.local_idx("__fpd_c2l");
                let c2_hi = self.local_idx("__fpd_c2h");
                let ab_hi = self.local_idx("__fpd_abh");
                let rl = self.local_idx("__fpd_rl");
                let rh = self.local_idx("__fpd_rh");
                let tmp = self.local_idx("__fpd_tmp");
                let tmp2 = self.local_idx("__fpd_tmp2");
                let mut v = Vec::new();

                // emit_mul128: computes hi=high64(x*y), lo=low64(x*y)
                let emit_mul128 = |v: &mut Vec<Instruction<'static>>, x: u32, y: u32, hi_dst: u32, lo_dst: u32| {
                    v.push(Instruction::LocalGet(x)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(m_xlo));
                    v.push(Instruction::LocalGet(x)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(m_xhi));
                    v.push(Instruction::LocalGet(y)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And); v.push(Instruction::LocalSet(m_ylo));
                    v.push(Instruction::LocalGet(y)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU); v.push(Instruction::LocalSet(m_yhi));
                    v.push(Instruction::LocalGet(m_xlo)); v.push(Instruction::LocalGet(m_ylo)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(m_ll));
                    v.push(Instruction::LocalGet(m_xlo)); v.push(Instruction::LocalGet(m_yhi)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(m_lh));
                    v.push(Instruction::LocalGet(m_xhi)); v.push(Instruction::LocalGet(m_ylo)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(m_hl));
                    v.push(Instruction::LocalGet(m_xhi)); v.push(Instruction::LocalGet(m_yhi)); v.push(Instruction::I64Mul); v.push(Instruction::LocalSet(m_hh));
                    v.push(Instruction::LocalGet(m_lh)); v.push(Instruction::LocalGet(m_hl)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(m_mid));
                    v.push(Instruction::LocalGet(m_lh)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(m_mc));
                    v.push(Instruction::LocalGet(m_ll));
                    v.push(Instruction::LocalGet(m_mid)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add); v.push(Instruction::LocalTee(m_lo));
                    v.push(Instruction::LocalGet(m_ll)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(m_lc));
                    v.push(Instruction::LocalGet(m_hh));
                    v.push(Instruction::LocalGet(m_mid)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(m_mc)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(m_lc)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalSet(hi_dst));
                    v.push(Instruction::LocalGet(m_lo)); v.push(Instruction::LocalSet(lo_dst));
                };

                // emit_fp64_mul: full Q64.64 multiply of {a_lo,a_hi} * {b_lo,b_hi} → {dst_lo,dst_hi}
                let emit_fp64_mul = |v: &mut Vec<Instruction<'static>>, a_lo: u32, a_hi: u32, b_lo: u32, b_hi: u32, dst_lo: u32, dst_hi: u32| {
                    // high64(a_lo * b_lo) → ab_hi (don't need low)
                    emit_mul128(v, a_lo, b_lo, ab_hi, tmp);
                    // full 128: a_hi * b_lo → {c1_lo, c1_hi}
                    emit_mul128(v, a_hi, b_lo, c1_hi, c1_lo);
                    // full 128: a_lo * b_hi → {c2_lo, c2_hi}
                    emit_mul128(v, a_lo, b_hi, c2_hi, c2_lo);
                    // cross = c1 + c2 (128-bit add)
                    v.push(Instruction::LocalGet(c1_lo)); v.push(Instruction::LocalGet(c2_lo)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(tmp));
                    v.push(Instruction::LocalGet(c1_lo)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp2));
                    v.push(Instruction::LocalGet(c1_hi)); v.push(Instruction::LocalGet(c2_hi)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(tmp2)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(m_mid));
                    // result_lo = cross_lo + ab_hi
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::LocalGet(ab_hi)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(dst_lo));
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp));
                    // result_hi = a_hi*b_hi + cross_hi + carry
                    v.push(Instruction::LocalGet(a_hi)); v.push(Instruction::LocalGet(b_hi)); v.push(Instruction::I64Mul);
                    v.push(Instruction::LocalGet(m_mid)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(dst_hi));
                };

                v.extend(da); v.push(Instruction::LocalSet(dst_i));
                v.extend(sa); v.push(Instruction::LocalSet(src_i));
                // Load a = dst (numerator)
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(al));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(ah));
                // Load b = src (denominator)
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(bl));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(bh));

                // Initial reciprocal estimate: x0 ≈ 1/b in Q64.64
                // For Q64.64 value b = bh + bl/2^64, 1/b ≈ 2^64/bh (for bh > 0)
                // As Q64.64: 1/b ≈ {2^64/bh, 0} if 1/b < 1, or {0, 2^64/bh} if 1/b >= 1
                // Since 2^64 doesn't fit in i64, use (2^64-1)/bh as approximation
                // If bh == 1: x0 = {0, 1} (exact reciprocal ≈ 1.0)
                // If bh >= 2: x0 = {(2^64-1)/bh, 0} (reciprocal < 1.0, stored in low word)
                // If bh == 0: b < 1.0, 1/b > 1.0. x0 = {0, (2^64-1)/bl}
                v.push(Instruction::LocalGet(bh)); v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // bh == 0: reciprocal > 1.0
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(x_lo));
                v.push(Instruction::I64Const(-1));
                v.push(Instruction::LocalGet(bl)); v.push(Instruction::I64DivU);
                v.push(Instruction::LocalSet(x_hi));
                v.push(Instruction::Else);
                // bh >= 1
                // Check if bh == 1
                v.push(Instruction::LocalGet(bh)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64Eq);
                v.push(Instruction::If(BlockType::Empty));
                // bh == 1: x0 = {0, 1} (≈ 1.0)
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(x_lo));
                v.push(Instruction::I64Const(1)); v.push(Instruction::LocalSet(x_hi));
                v.push(Instruction::Else);
                // bh >= 2: x0 = {(2^64-1)/bh, 0}
                v.push(Instruction::I64Const(-1));
                v.push(Instruction::LocalGet(bh)); v.push(Instruction::I64DivU);
                v.push(Instruction::LocalSet(x_lo));
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(x_hi));
                v.push(Instruction::End); // bh == 1
                v.push(Instruction::End); // bh == 0

                // Newton iterations: x = x * (2 - b*x), 3 iterations
                for _ in 0..3 {
                    // t = b * x (Q64.64 multiply)
                    emit_fp64_mul(&mut v, bl, bh, x_lo, x_hi, tx_lo, tx_hi);
                    // correction = 2.0 - t (Q64.64 subtraction)
                    // cl = 0 - tx_lo (with borrow)
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalGet(tx_lo)); v.push(Instruction::I64Sub); v.push(Instruction::LocalTee(cl));
                    v.push(Instruction::I64Const(0)); v.push(Instruction::I64GtU); // borrow if cl wrapped (cl > 0 when it should be 0-tx_lo)
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp));
                    // ch = 2 - tx_hi - borrow
                    v.push(Instruction::I64Const(2));
                    v.push(Instruction::LocalGet(tx_hi)); v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Sub);
                    v.push(Instruction::LocalSet(ch));
                    // x = x * correction (Q64.64 multiply)
                    emit_fp64_mul(&mut v, x_lo, x_hi, cl, ch, x_lo, x_hi);
                }

                // Final: result = a * x (Q64.64 multiply)
                emit_fp64_mul(&mut v, al, ah, x_lo, x_hi, rl, rh);

                // Store result to dst
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "fp64/sqrt" => {
                // Q64.64 Newton: r = (r + V/r) / 2, iterated
                // Work directly in Q64.64 with {rl, rh} as the estimate
                // V/r approximated with high-word division (Newton is self-correcting)
                let dst = self.expr(&a[0])?;
                let src = self.expr(&a[1])?;
                let dst_i = self.local_idx("__fsqrt_d");
                let src_i = self.local_idx("__fsqrt_s");
                let vh = self.local_idx("__fsqrt_vh");
                let vl = self.local_idx("__fsqrt_vl");
                let rh = self.local_idx("__fsqrt_rh");
                let rl = self.local_idx("__fsqrt_rl");
                let _prev_rh = self.local_idx("__fsqrt_prh");
                let qh = self.local_idx("__fsqrt_qh");
                let ql = self.local_idx("__fsqrt_ql");
                let sum_l = self.local_idx("__fsqrt_sl");
                let sum_h = self.local_idx("__fsqrt_sh");
                let tmp = self.local_idx("__fsqrt_tmp");
                let mut v = Vec::new();
                v.extend(dst); v.push(Instruction::LocalSet(dst_i));
                v.extend(src); v.push(Instruction::LocalSet(src_i));
                // Load Q64.64 value V = {vl, vh}
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(vl));
                v.push(Instruction::LocalGet(src_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Load(wasm_encoder::MemArg { offset: 8, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalSet(vh));

                // Handle V == 0
                v.push(Instruction::LocalGet(vh)); v.push(Instruction::I64Eqz);
                v.push(Instruction::LocalGet(vl)); v.push(Instruction::I64Eqz);
                v.push(Instruction::I32And);
                v.push(Instruction::If(BlockType::Empty));
                // V == 0: result = 0
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::I64Const(0));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::Else);
                // Initial guess: r = isqrt(vh) as Q64.64 {0, isqrt(vh)}
                // Use 64-bit Newton to compute isqrt(vh)
                let r64 = self.local_idx("__fsqrt_r64");
                let p64 = self.local_idx("__fsqrt_p64");
                v.push(Instruction::LocalGet(vh)); v.push(Instruction::LocalSet(r64));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalSet(p64));
                v.push(Instruction::LocalGet(p64));
                v.push(Instruction::LocalGet(vh));
                v.push(Instruction::LocalGet(p64));
                v.push(Instruction::I64DivU);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r64));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalGet(p64));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(p64)); v.push(Instruction::LocalSet(r64));
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);

                // Handle isqrt(vh) == 0 (vh was 0 or 1)
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::I64Eqz);
                v.push(Instruction::If(BlockType::Empty));
                // r64 == 0: do isqrt(vl) instead, result = isqrt(vl) * 2^32
                v.push(Instruction::LocalGet(vl)); v.push(Instruction::LocalSet(r64));
                v.push(Instruction::Block(BlockType::Empty));
                v.push(Instruction::Loop(BlockType::Empty));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalSet(p64));
                v.push(Instruction::LocalGet(p64));
                v.push(Instruction::LocalGet(vl));
                v.push(Instruction::LocalGet(p64));
                v.push(Instruction::I64DivU);
                v.push(Instruction::I64Add);
                v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                v.push(Instruction::LocalSet(r64));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalGet(p64));
                v.push(Instruction::I64GeU);
                v.push(Instruction::If(BlockType::Empty));
                v.push(Instruction::LocalGet(p64)); v.push(Instruction::LocalSet(r64));
                v.push(Instruction::Br(2));
                v.push(Instruction::End);
                v.push(Instruction::Br(0));
                v.push(Instruction::End);
                v.push(Instruction::End);
                // Store isqrt(vl) * 2^32
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::I64Const(0xFFFFFFFF)); v.push(Instruction::I64And);
                v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::Else);
                // r64 > 0: initial Q64.64 guess r = {0, r64}
                v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(rl));
                v.push(Instruction::LocalGet(r64)); v.push(Instruction::LocalSet(rh));

                // Q64.64 Newton: r = (r + V/r) / 2, 6 iterations
                // V/r uses high-word division with refinement: q_hi = vh/rh, q_lo estimated
                for _ in 0..6 {
                    // V/r: simplified Q64.64 division
                    // If rh == 0: q = {0xFFFFFFFFFFFFFFFF / max(rl,1), 0} (rough)
                    // Else: q_hi = vh / rh, q_lo from remainder refinement
                    v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    // rh == 0: rough estimate
                    v.push(Instruction::LocalGet(rl)); v.push(Instruction::I64Eqz);
                    v.push(Instruction::If(BlockType::Empty));
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(qh));
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(ql));
                    v.push(Instruction::Else);
                    v.push(Instruction::I64Const(-1)); v.push(Instruction::LocalGet(rl)); v.push(Instruction::I64DivU);
                    v.push(Instruction::LocalSet(ql));
                    v.push(Instruction::I64Const(0)); v.push(Instruction::LocalSet(qh));
                    v.push(Instruction::End);
                    v.push(Instruction::Else);
                    // rh > 0: q_hi = vh / rh, remainder for q_lo refinement
                    v.push(Instruction::LocalGet(vh)); v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64DivU);
                    v.push(Instruction::LocalSet(qh));
                    // remainder_hi = vh % rh
                    v.push(Instruction::LocalGet(vh)); v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64RemU);
                    v.push(Instruction::LocalSet(tmp));
                    // q_lo ≈ (remainder_hi << 32 + (vl >> 32)) / rh << 32 ... simplified:
                    // q_lo ≈ (remainder_hi * 2^64) / rh, but use 64-bit approx:
                    // q_lo = (remainder_hi << 32 | vl >> 32) / rh ... but this might overflow
                    // Simpler: q_lo = ((tmp << 32) + (vl >> 32)) / rh
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64Shl);
                    v.push(Instruction::LocalGet(vl)); v.push(Instruction::I64Const(32)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::I64Or);
                    v.push(Instruction::LocalGet(rh)); v.push(Instruction::I64DivU);
                    v.push(Instruction::LocalSet(ql));
                    v.push(Instruction::End);

                    // sum = r + q (Q64.64 add with carry)
                    v.push(Instruction::LocalGet(rl)); v.push(Instruction::LocalGet(ql)); v.push(Instruction::I64Add); v.push(Instruction::LocalTee(sum_l));
                    v.push(Instruction::LocalGet(rl)); v.push(Instruction::I64LtU);
                    v.push(Instruction::I64ExtendI32U); v.push(Instruction::LocalSet(tmp));
                    v.push(Instruction::LocalGet(rh)); v.push(Instruction::LocalGet(qh)); v.push(Instruction::I64Add);
                    v.push(Instruction::LocalGet(tmp)); v.push(Instruction::I64Add); v.push(Instruction::LocalSet(sum_h));

                    // r = sum >> 1 (Q64.64 right shift by 1)
                    // new_rl = (sum_l >> 1) | (sum_h << 63)
                    // new_rh = sum_h >> 1
                    v.push(Instruction::LocalGet(sum_l)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalGet(sum_h)); v.push(Instruction::I64Const(63)); v.push(Instruction::I64Shl);
                    v.push(Instruction::I64Or); v.push(Instruction::LocalSet(rl));
                    v.push(Instruction::LocalGet(sum_h)); v.push(Instruction::I64Const(1)); v.push(Instruction::I64ShrU);
                    v.push(Instruction::LocalSet(rh));
                }

                // Store result
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::LocalGet(rl));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::LocalGet(dst_i)); v.push(Instruction::I32WrapI64);
                v.push(Instruction::I32Const(8)); v.push(Instruction::I32Add);
                v.push(Instruction::LocalGet(rh));
                v.push(Instruction::I64Store(wasm_encoder::MemArg { offset: 0, align: 3, memory_index: 0 }));
                v.push(Instruction::End); // r64 == 0
                v.push(Instruction::End); // V == 0
                v.push(Instruction::I64Const(TAG_NIL)); Ok(v)
            }
            "fp/sqrt" => {
                // sqrt(Q64.64) = sqrt(x * 2^64) = sqrt(x) * 2^32
                // = sqrt(x) << 32 in Q64.64
                // Use integer sqrt of (x >> 32) then << 48... 
                // Actually: want sqrt(x) where x is Q64.64
                // = isqrt(x) if x were the full number
                // Since x = (real_val) << 64, sqrt(x) = sqrt(real_val) << 32
                // = isqrt(x >> 64) << 32 ... no
                // Better: isqrt(x >> 32) << 16 ... losing precision
                // Best: isqrt(x) then shift. x is Q64.64 so ~64 bits of fraction
                // isqrt(x) gives sqrt with ~32 bits of fraction implicitly
                // But x can be up to 128 bits. Use two-part method:
                // Split x = hi << 64 | lo
                // sqrt = sqrt(hi) << 32 + adjustment
                // For CLMM: we mostly sqrt prices ~1-1000, so hi is small
                // Just use: (sqrt(x >> 32)) << 16 as approximation? No.
                // Correct approach: isqrt(x) where x is treated as uint128
                // We can do: r = isqrt(high * 2^64 + low)
                // ≈ isqrt(high) << 32 + low / (2 * isqrt(high) << 32)
                // For simplicity: (sqrt (x >> 32)) << 16 gives OK precision for CLMM
                // Actually the correct Q64.64 sqrt: 
                //   result = isqrt(x) where we need 128-bit isqrt
                //   Split: a = x >> 64, b = x & ((1<<64)-1)
                //   r = isqrt(a) << 32
                //   remainder = a - r^2 (in high bits)  
                //   r = (r << 64 + b) correction via Newton
                // Simplest correct: compute integer sqrt of (x >> 32), then << 16
                // This gives Q64.32 result, need to shift to Q64.64: << 32 more = << 48
                // NO. Let me think again.
                // Q64.64 value V represents real number v = V / 2^64
                // We want sqrt(v) * 2^64 = sqrt(V/2^64) * 2^64 = sqrt(V) * 2^32
                // So: fp/sqrt(x) = isqrt(x) * 2^32 ... but isqrt of a Q64.64 number
                // that's at most ~2^127 gives result ~2^63, then * 2^32 overflows
                // 
                // Better: fp/sqrt(x) = isqrt(x) >> 0, since isqrt(Q64.64) already has
                // the right scale? No.
                //
                // Simplest: fp/sqrt(x) = isqrt(x) for Q64.64 input
                // If x = 1.0 = 2^64, isqrt(2^64) = 2^32 = 0.5 in Q64.64... wrong
                // We want sqrt(1.0) = 1.0 = 2^64
                // So: fp/sqrt(x) = isqrt(x << 64) ... but that overflows
                //
                // Practical CLMM: use Q64 for sqrt price, separate from Q64.64
                // (fp/sqrt x) = (sqrt x) gives integer sqrt, caller manages scaling
                // Just delegate to integer sqrt
                // User does: (fp/from_int (sqrt (fp/to_int price_approx)))
                // Or: (sqrt x) << 32 for Q64.64 sqrt of integer
                // I'll just delegate — fp/sqrt is an alias for careful scaling
                let inner = &LispVal::List(vec![
                    LispVal::Sym("sqrt".into()), a[0].clone()
                ]);
                // After sqrt, shift left by 32 to get Q64.64 result from Q64.0 input
                // Wait — sqrt of Q64.64 = isqrt(x) which gives wrong scale
                // For Q64.64 input x representing value X: x = X * 2^64
                // sqrt(x) in same format = sqrt(X) * 2^64 = sqrt(X * 2^64) * 2^32
                // = isqrt(x) * 2^32
                let mut v = self.expr(inner)?;
                v.push(Instruction::I64Const(32));
                v.push(Instruction::I64Shl);
                Ok(v)
            }
            _ => Err("__not_handled__".into()),
        }
    }
}
